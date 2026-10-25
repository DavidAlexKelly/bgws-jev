//! Experiments for the analysis crate (§15): each benchmark vignette as a
//! pure function of `(figures, seed)`, and whole-scenario batches.
//!
//! Where the sim has the domain wired end to end the experiment plays a
//! scenario through it; otherwise it runs the domain's model directly on a
//! representative engagement (the component-level vignette), with the same
//! figures, so the sensitivity analysis still sees every figure that drives it.

use bgws_analysis::report::RunOutcome;
use bgws_analysis::{batch::MetricFn, vignettes, CheckOptions, Experiment, VignetteResult};
use bgws_core::{Figures, Seed, Side, SimTime, StreamKey};
use bgws_domains::air_defence::{engage_raid, AdSystem, Assignment, KillChain, Threat, ThreatClass};
use bgws_domains::missile::{Flight, FlightClass, Profile};
use bgws_domains::V3;
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Act, Activity, Health, Ident};
use bgws_sim::{Sim, SimOptions};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

/// One replicate's value for a vignette metric (None: nothing to measure).
pub type Value = Option<f64>;

fn rng(seed: Seed, what: &'static str) -> ChaCha8Rng {
    seed.stream(StreamKey::new(what, 0, 0, 0))
}

/// MBT first round at 2 km, stationary full-exposed MBT, frontal [P27].
pub fn mbt_engagement(f: &Figures, seed: Seed) -> Value {
    use bgws_models::direct_fire::*;
    let p = DirectFireParams::from_figures(f);
    let fc = FireControl::from_quality(0.9, &p);
    let shot = Shot { range_m: 2000.0, firer_moving: false, target_crossing_mps: 0.0, skill_mult: 1.0, suppression: 0.0 };
    let b = error_budget(&p.mbt_gun, &fc, &shot, &p);
    let dims = VehicleDims { length_m: 9.8, width_m: 3.5, hull_height_m: 1.6, turret_height_m: 0.9 };
    let pres = presented_vehicle(&dims, 0.0, VehiclePosture::Exposed, &p);
    let mut r = rng(seed, "v.mbt");
    let mut e = Engagement::new(&b, &mut r);
    Some(if e.fire(&pres, &b, &p, &mut r).hit { 1.0 } else { 0.0 })
}

/// FPV strike by a skilled crew: of the hits, did it fail to destroy? [P9]
pub fn fpv(f: &Figures, seed: Seed) -> Value {
    use bgws_ops::drones::*;
    let p = StrikeParams::from_figures(f);
    let mut r = rng(seed, "v.fpv");
    let spec = DroneSpec::from_figures(if r.gen::<f64>() < 0.2 { DroneType::FpvFibre } else { DroneType::FpvRf }, f);
    let targets = [TargetClass::Mbt, TargetClass::Ifv, TargetClass::Truck, TargetClass::Artillery, TargetClass::InfantryOpen, TargetClass::InfantryCover];
    let inputs = StrikeInputs {
        operator: Operator { skill: r.gen_range(0.7..0.95), fatigue: r.gen_range(0.0..0.4) },
        target: targets[r.gen_range(0..targets.len())],
        target_speed_mps: if r.gen::<f64>() < 0.5 { 0.0 } else { r.gen_range(2.0..8.0) },
        concealment: r.gen_range(0.0..0.5),
        wind_mps: r.gen_range(0.0..8.0),
        link_quality: if r.gen::<f64>() < 0.3 { r.gen_range(0.85..0.98) } else { 1.0 },
        distance_m: r.gen_range(2_000.0..12_000.0),
        approach_s: 60.0,
        counter_drone_rate: if r.gen::<f64>() < 0.3 { 0.003 } else { 0.0 },
        hit_modifier: 1.0,
        kill_modifier: 1.0,
    };
    let o = fpv_strike(&spec, &p, &inputs, &mut r);
    if !o.hit() {
        return None;
    }
    Some(if o.stage == StrikeStage::Destroyed { 0.0 } else { 1.0 })
}

fn owa_raid(f: &Figures, n_real: u32, n_decoy: u32) -> Vec<Threat> {
    let prof = Profile::from_figures(FlightClass::OneWayDrone, f);
    (0..n_real + n_decoy)
        .map(|i| {
            let a = i as f64 * 0.013;
            let from = V3::new(120_000.0 * a.cos(), 120_000.0 * a.sin(), 0.0);
            let class = if i < n_real { ThreatClass::OneWayDrone } else { ThreatClass::Decoy };
            Threat { id: i, class, flight: Flight::new(prof.clone(), from, V3::ZERO, i as f64 * 3.0), cost: if i < n_real { 5.0e4 } else { 1.0e4 }, value: 1.0e6 }
        })
        .collect()
}

/// A Shahed-type raid (60% strike, 40% decoy) against a layered defence [P10].
pub fn shahed(f: &Figures, seed: Seed) -> Value {
    let chain = KillChain::from_figures(f);
    let mut sys = vec![AdSystem::from_figures("mrsam", "mrsam", V3::new(-2_000.0, 0.0, 0.0), f), AdSystem::from_figures("ewSoftKill", "ew", V3::ZERO, f)];
    for i in 0..3 {
        let a = i as f64 * std::f64::consts::TAU / 3.0;
        sys.push(AdSystem::from_figures("mobileGun", &format!("gun{i}"), V3::new(8_000.0 * a.cos(), 8_000.0 * a.sin(), 0.0), f));
        sys.push(AdSystem::from_figures("interceptorDrone", &format!("idrone{i}"), V3::new(4_000.0 * a.cos(), 4_000.0 * a.sin(), 0.0), f));
    }
    let out = engage_raid(&mut sys, &owa_raid(f, 60, 40), &chain, Assignment::Optimal, 1.0, &mut rng(seed, "v.shahed"));
    Some(out.intercept_rate())
}

/// Ballistic missiles against two BMD batteries, shoot-shoot-look-shoot [P20, P29].
pub fn bmd(f: &Figures, seed: Seed) -> Value {
    let chain = KillChain::from_figures(f);
    let prof = Profile::from_figures(FlightClass::Ballistic, f);
    let mut sys = vec![AdSystem::from_figures("bmd", "bmd1", V3::ZERO, f), AdSystem::from_figures("bmd", "bmd2", V3::new(3_000.0, 0.0, 0.0), f)];
    let raid: Vec<Threat> = (0..6)
        .map(|i| Threat { id: i, class: ThreatClass::Ballistic, flight: Flight::new(prof.clone(), V3::new(300_000.0, 20_000.0 * i as f64, 0.0), V3::new(0.0, 500.0, 0.0), 30.0 * i as f64), cost: 3.0e6, value: 1.0e8 })
        .collect();
    let out = engage_raid(&mut sys, &raid, &chain, Assignment::Optimal, 1.0, &mut rng(seed, "v.bmd"));
    Some(out.threats.iter().filter(|t| t.killed_by.is_some()).count() as f64 / out.threats.len() as f64)
}

/// A destroyer group against a drone raid: interceptor missiles per threat engaged [R6.2].
pub fn red_sea(f: &Figures, seed: Seed) -> Value {
    let chain = KillChain::from_figures(f);
    let owa = Profile::from_figures(FlightClass::OneWayDrone, f);
    let mut sys = vec![
        AdSystem::from_figures("lrsam", "SM-2/6", V3::ZERO, f),
        AdSystem::from_figures("mrsam", "ESSM", V3::ZERO, f),
        AdSystem::from_figures("gun5in", "5-inch", V3::ZERO, f),
        AdSystem::from_figures("gun", "CIWS", V3::ZERO, f),
        AdSystem::from_figures("fighter", "CAP1", V3::new(-15_000.0, 30_000.0, 6_000.0), f),
        AdSystem::from_figures("fighter", "CAP2", V3::new(15_000.0, 30_000.0, 6_000.0), f),
        AdSystem::from_figures("ewSoftKill", "EW", V3::ZERO, f),
    ];
    let raid: Vec<Threat> = (0..20)
        .map(|i| Threat { id: i, class: ThreatClass::OneWayDrone, flight: Flight::new(owa.clone(), V3::new(-20_000.0 + 2_000.0 * i as f64, 150_000.0, 0.0), V3::ZERO, 45.0 * i as f64), cost: 2.0e4, value: 5.0e5 })
        .collect();
    let out = engage_raid(&mut sys, &raid, &chain, Assignment::Optimal, 1.0, &mut rng(seed, "v.redsea"));
    let engaged = out.threats.iter().filter(|t| t.shots > 0).count();
    if engaged == 0 {
        return None;
    }
    Some((out.shots_by_system["SM-2/6"] + out.shots_by_system["ESSM"]) as f64 / engaged as f64)
}

/// A minefield breach with a line charge, 400–600 m deep [P13].
pub fn breach(f: &Figures, seed: Seed) -> Value {
    use bgws_models::engineers::*;
    let p = EngineerParams::from_figures(f);
    let mut r = rng(seed, "v.breach");
    let field = Minefield { depth_m: r.gen_range(400.0..600.0), density_per_m2: r.gen_range(0.005..0.015), observed: false };
    Some(breach_lane(BreachMethod::LineCharge, &field, &p).time_s)
}

fn precision(f: &Figures, seed: Seed, jammed: bool) -> Value {
    use bgws_ops::pnt::MunitionNav;
    let acc = MunitionNav::guided_shell(f, 70.0, if jammed { Some(70.0) } else { None }).accuracy();
    let p = acc.p_within(7.0);
    Some(if rng(seed, "v.precision").gen::<f64>() < p { 1.0 } else { 0.0 })
}

/// An Excalibur-type round with GNSS clear [P15].
pub fn precision_clear(f: &Figures, seed: Seed) -> Value {
    precision(f, seed, false)
}

/// The same round with GNSS denied for the whole flight [P15, P16].
pub fn precision_jammed(f: &Figures, seed: Seed) -> Value {
    precision(f, seed, true)
}

fn sortie(f: &Figures, seed: Seed, tempo: bgws_domains::air::sortie::Tempo) -> Value {
    use bgws_domains::air::sortie::*;
    let p = SortieParams::from_figures(f, tempo);
    Some(simulate_fleet(&p, 24, 10, &mut rng(seed, "v.sortie")).per_aircraft_per_day)
}

/// Fighter sorties per aircraft per day, surge [P30].
pub fn sortie_surge(f: &Figures, seed: Seed) -> Value {
    sortie(f, seed, bgws_domains::air::sortie::Tempo::Surge)
}

/// Sustained tempo [P30].
pub fn sortie_sustained(f: &Figures, seed: Seed) -> Value {
    sortie(f, seed, bgws_domains::air::sortie::Tempo::Sustained)
}

/// Time from wounding to the first medical care (the aid post) for a T1
/// casualty, with extraction under drones [R7].
pub fn casevac(f: &Figures, seed: Seed) -> Value {
    use bgws_ops::medical::*;
    let mut r = rng(seed, "v.casevac");
    let cfg = ChainConfig {
        poi_delay: bgws_core::SimDuration::from_mins(r.gen_range(30.0..90.0)),
        forward: vec![Fleet::new("forward ambulance", 3, 4, 20.0, &[Triage::T1, Triage::T2])],
        rear: vec![Fleet::new("ground ambulance", 3, 4, 60.0, &[Triage::T1, Triage::T2])],
        aid_teams: 2,
        surgical_tables: 4,
        to_hospital: bgws_core::SimDuration::from_hours(6.0),
    };
    let mut chain = MedicalChain::new(cfg, f, seed);
    chain.injure(0, Triage::T1, SimTime::ZERO);
    let out = chain.run(SimTime::from_secs_i(3 * 86_400));
    out.first().and_then(|o| o.times.reached_aid_post).map(|t| t.secs() / 60.0)
}

/// A company attack across open ground on a defended position, played in the
/// sim on flat ground with no breakpoint: the attacker's losses when its
/// advance stops (no manoeuvre unit still advancing for a minute) [P14].
pub fn attack_halt(scenario: &ScenarioV4) -> impl Fn(&Figures, Seed) -> Value + Sync + '_ {
    move |f: &Figures, seed: Seed| {
        let mut sc = scenario.clone();
        sc.victory.clear();
        let mut sim = Sim::with_options(&sc, seed, f.clone(), SimOptions { flat_ground: true, ..Default::default() }).ok()?;
        let mut still = 0.0;
        let mut last_loss = 0.0;
        for step in 1..=720 {
            let out = sim.run_with_rules(SimTime::from_secs(step as f64 * 10.0));
            let (mut fit, mut start, mut advancing) = (0.0, 0.0, 0usize);
            for &e in &sim.ctx.order {
                let (Some(i), Some(h), Some(a)) = (sim.world.get::<Ident>(e), sim.world.get::<Health>(e), sim.world.get::<Act>(e)) else { continue };
                if i.side != Side::Blue || i.hq {
                    continue;
                }
                fit += if h.destroyed { 0.0 } else { h.fit as f64 };
                start += h.start as f64;
                if !h.destroyed && matches!(a.activity, Activity::Move { .. } | Activity::Engage { .. }) {
                    advancing += 1;
                }
            }
            let loss = 1.0 - fit / start.max(1.0);
            if loss > 0.0 && advancing == 0 {
                still += 10.0;
                if still >= 60.0 {
                    return Some(loss);
                }
            } else {
                still = 0.0;
            }
            last_loss = loss;
            if out.is_some() {
                break;
            }
        }
        if last_loss > 0.0 { Some(last_loss) } else { None }
    }
}

/// A whole scenario played with the rules: the outcome for batch statistics.
pub fn scenario_run(scenario: &ScenarioV4, limit: SimTime, opts: SimOptions) -> impl Fn(&Figures, Seed) -> RunOutcome + Sync + '_ {
    move |f: &Figures, seed: Seed| {
        let mut sim = match Sim::with_options(scenario, seed, f.clone(), opts.clone()) {
            Ok(s) => s,
            Err(e) => return RunOutcome { over: Some(format!("load error: {e}")), time: SimTime::ZERO, strength: Default::default() },
        };
        let out = sim.run_with_rules(limit);
        RunOutcome {
            over: out.as_ref().map(|o| format!("{} ({})", o.winner.map(|w| format!("{w} wins")).unwrap_or_else(|| "draw".into()), o.reason)),
            time: sim.now(),
            strength: sim.strength(),
        }
    }
}

/// Run the benchmark vignettes that have an experiment here.
pub fn run_vignettes(figures: &Figures, seed: Seed, options: &CheckOptions, only: Option<&str>, attack_scenario: Option<&ScenarioV4>) -> Vec<VignetteResult> {
    let id_metric = |v: &vignettes::Vignette| MetricFn::new(v.metric.name.clone(), v.metric.unit.clone(), |o: &Value| *o);
    let mut out = Vec::new();
    for v in vignettes::benchmark_vignettes() {
        if only.map(|o| !v.id.contains(o)).unwrap_or(false) {
            continue;
        }
        let m = id_metric(&v);
        let check = |e: &(dyn Fn(&Figures, Seed) -> Value + Sync)| {
            let f = |fig: &Figures, s: Seed| e(fig, s);
            v.check(&Wrap(&f), &m, figures, seed, options)
        };
        let r = match v.id.as_str() {
            "mbt-engagement" => check(&mbt_engagement),
            "fpv-effectiveness" => check(&fpv),
            "shahed-raid" => check(&shahed),
            "ballistic-missile-defence" => check(&bmd),
            "breach-time" => check(&breach),
            "precision-fires-clear" => check(&precision_clear),
            "precision-fires-jammed" => check(&precision_jammed),
            "sortie-surge" => check(&sortie_surge),
            "sortie-sustained" => check(&sortie_sustained),
            "casevac-delay" => check(&casevac),
            "red-sea-magazine" => check(&red_sea),
            "attack-halt" => match attack_scenario {
                Some(sc) => {
                    let e = attack_halt(sc);
                    check(&e)
                }
                None => continue,
            },
            _ => continue,
        };
        out.push(r);
    }
    out
}

/// Lets a `&dyn Fn` be an `Experiment`.
struct Wrap<'a, F: Fn(&Figures, Seed) -> Value + Sync>(&'a F);

impl<F: Fn(&Figures, Seed) -> Value + Sync> Experiment for Wrap<'_, F> {
    type Output = Value;
    fn run(&self, figures: &Figures, seed: Seed) -> Value {
        (self.0)(figures, seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_vignettes_are_in_band() {
        let f = crate::default_figures();
        let opts = CheckOptions { level: 0.95, replicates: Some(60) };
        let results = run_vignettes(&f, Seed(1), &opts, None, None);
        assert!(results.len() >= 10, "{}", results.len());
        let out: Vec<_> = results.iter().filter(|r| r.verdict != bgws_analysis::vignettes::Verdict::InBand).map(|r| format!("{} = {:.3} not in [{}, {}]", r.id, r.estimate, r.band.lo, r.band.hi)).collect();
        assert!(out.is_empty(), "{out:#?}");
    }
}

/// Morris screening then Sobol indices for a vignette over its figures, and
/// the research backlog of unsourced figures ranked by sensitivity (§15.4).
pub fn sensitivity(id: &str, replicates: usize, prefix: Option<&str>, seed: Seed) -> anyhow::Result<()> {
    use bgws_analysis::sensitivity::*;
    let figures = crate::default_figures();
    let v = vignettes::find(id).ok_or_else(|| anyhow::anyhow!("no vignette `{id}`"))?;
    let exp: Box<dyn Fn(&Figures, Seed) -> Value + Sync> = match id {
        "mbt-engagement" => Box::new(mbt_engagement),
        "fpv-effectiveness" => Box::new(fpv),
        "shahed-raid" => Box::new(shahed),
        "ballistic-missile-defence" => Box::new(bmd),
        "breach-time" => Box::new(breach),
        "precision-fires-clear" => Box::new(precision_clear),
        "precision-fires-jammed" => Box::new(precision_jammed),
        "sortie-surge" => Box::new(sortie_surge),
        "sortie-sustained" => Box::new(sortie_sustained),
        "casevac-delay" => Box::new(casevac),
        "red-sea-magazine" => Box::new(red_sea),
        _ => anyhow::bail!("vignette `{id}` has no experiment here"),
    };
    let prefixes: Vec<String> = match prefix {
        Some(p) => vec![p.to_string()],
        None => v.driving_figures.iter().map(|k| k.split('.').take(2).collect::<Vec<_>>().join(".")).collect(),
    };
    let filter = |f: &bgws_core::Figure| prefixes.is_empty() || prefixes.iter().any(|p| f.key.starts_with(p.as_str()));
    let facs = factors(&figures, filter);
    if facs.is_empty() {
        anyhow::bail!("no figures match");
    }
    eprintln!("{} factors", facs.len());
    let metric = MetricFn::new(v.metric.name.clone(), v.metric.unit.clone(), |o: &Value| *o);
    let wrap = |f: &Figures, s: Seed| exp(f, s);
    let resp = BatchResponse { experiment: &Wrap(&wrap), metric: &metric, config: bgws_analysis::batch::BatchConfig::new(replicates, seed), statistic: v.band.statistic };
    let morris = MorrisOptions::from_figures(&figures);
    let sobol = SobolOptions::from_figures(&figures);
    let report = analyse(&v.id, &figures, &facs, &resp, &morris, &sobol, seed);
    println!("{}", bgws_analysis::report::sensitivity_markdown(&report));
    Ok(())
}
