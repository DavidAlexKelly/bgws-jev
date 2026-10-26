//! Comms, C2, EW, PNT and drones in real games (RUST_ENGINE.md §8.5, §8.7,
//! §8.12, §18 phases 4 and 5): the M3 battalion attack and the M4 drone
//! line play; detect-to-strike [P2], FPV effectiveness [P9], precision
//! under jamming [P15] and kill-zone depth [P8] are measured from games;
//! out-of-comms units keep to their intent; losing an HQ slows its units'
//! reports; runs are deterministic.

use bgws_core::{EntityId, LogEvent, Seed, Side, SimTime};
use bgws_ops::drones::{DroneType, StrikeStage};
use bgws_scenario::v4::{Emcon, ScenarioV4};
use bgws_sim::domains::drones::{self as c4mod, c2, comms, drones, ew, C4};
use bgws_sim::{default_figures, Sim};
use std::collections::BTreeMap;

fn scenario(name: &str) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn kinds(ev: &[LogEvent]) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for e in ev {
        *m.entry(e.kind.clone()).or_insert(0) += 1;
    }
    m
}

fn id(sim: &Sim, name: &str) -> EntityId {
    sim.ctx.by_name[name]
}

fn play(name: &str, seed: u64, minutes: f64) -> (Sim, Vec<LogEvent>) {
    let sc = scenario(name);
    let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(minutes * 60.0));
    let ev = sim.drain_events();
    (sim, ev)
}

/// Play as the host and the vignettes do: each question's rules choice, and
/// the heuristic planner for HQ plans (it positions fire and drone teams by
/// their reach, where the sim's own rules plan only sends the attack in).
fn play_planned(name: &str, seed: u64, minutes: f64) -> (Sim, Vec<LogEvent>) {
    let sc = scenario(name);
    let mut list = bgws_sim::all_figures();
    list.extend(bgws_decide::figures());
    let figures = bgws_core::Figures::from_list(list).unwrap();
    let planner = bgws_decide::HeuristicPlanner::from_figures(&figures);
    let mut sim = Sim::new(&sc, Seed(seed), figures).unwrap();
    let limit = SimTime::from_secs(minutes * 60.0);
    loop {
        match sim.advance(limit) {
            bgws_sim::Step::Advanced | bgws_sim::Step::Over(_) => break,
            bgws_sim::Step::NeedsAnswers { questions, .. } => {
                let a = questions
                    .iter()
                    .map(|q| if q.kind == bgws_core::QuestionKind::Hq { planner.answer(q, bgws_core::AnswerSource::Rules) } else { bgws_core::Answer::simple(q, q.rules_choice.clone(), bgws_core::AnswerSource::Rules) })
                    .collect();
                sim.answer(a).unwrap();
            }
        }
    }
    let ev = sim.drain_events();
    (sim, ev)
}

#[test]
fn m3_battalion_attack_runs_on_its_nets() {
    let (sim, ev) = play("m3-battalion-attack", 1, 60.0);
    let k = kinds(&ev);
    eprintln!("{k:?}");
    let st = comms::stats(&sim).unwrap();
    eprintln!("{st:?}");
    // The plan went out as orders over the nets and arrived.
    assert!(k.get("c2.plan_issued").copied().unwrap_or(0) >= 2);
    assert!(k.get("c2.order_received").copied().unwrap_or(0) >= 15, "orders received: {:?}", k.get("c2.order_received"));
    // Contact reports reached HQs, were worked by the staff, fused into the COPs and forwarded up.
    assert!(st.delivered.get("spotrep").copied().unwrap_or(0) > 0);
    assert!(st.delivered.get("tracks").copied().unwrap_or(0) > 0, "company HQs forward tracks to the battalion");
    let lat = c2::cop_latencies(&sim);
    assert!(lat.iter().any(|l| l.top), "reports reached the top HQ's COP");
    // The planner sees the top HQ's COP and its units as reported.
    let sit = sim.hq_situation(Side::Blue, "test");
    assert!(!sit.cop.is_empty(), "blue battalion COP is empty");
    assert!(sit.post.is_some());
    // A CCIR or the battle rhythm started a plan, with planning time.
    assert!(k.get("c2.planning").copied().unwrap_or(0) >= 1);
}

#[test]
fn m4_drone_line_plays_the_drone_and_ew_fight() {
    let (sim, ev) = play("m4-drone-line", 2, 120.0);
    let k = kinds(&ev);
    eprintln!("{k:?}");
    for kind in ["drones.launch", "drones.found", "drones.strike", "drones.link_lost", "drones.shot_down", "ew.intercept", "ew.fix", "pnt.lost"] {
        assert!(k.get(kind).copied().unwrap_or(0) > 0, "no {kind} events");
    }
    // Strikes changed the world: a strike event carries its odds.
    let strike = ev.iter().find(|e| e.kind == "drones.strike").unwrap();
    assert!(strike.data["pHit"].as_f64().unwrap() > 0.0);
    // ESM fixes went over the nets into the side picture.
    let fixes = ew::fixes(&sim, Side::Blue).len() + ew::fixes(&sim, Side::Red).len();
    assert!(fixes > 0);
    let es = ew::stats(&sim).unwrap();
    assert!(es.intercepts > 0 && !es.time_to_fix_s.is_empty());
}

/// Detect-to-strike [P2] and FPV effectiveness [P9] from M4 games.
#[test]
fn detect_to_strike_and_fpv_effectiveness_emerge_in_band() {
    let mut d2s = Vec::new();
    let mut terminal = 0usize;
    let mut hits = 0usize;
    let mut kills = 0usize;
    let mut sorties = 0usize;
    let (mut e_hit, mut e_kill) = (0.0, 0.0);
    for seed in 1..=8 {
        let (sim, _) = play_planned("m4-drone-line", seed, 180.0);
        for r in drones::records(&sim) {
            if !matches!(r.kind, DroneType::FpvRf | DroneType::FpvFibre) {
                continue;
            }
            sorties += 1;
            if matches!(r.stage, StrikeStage::Missed | StrikeStage::HitNoKill | StrikeStage::Destroyed) {
                terminal += 1;
                e_hit += r.p_hit;
                e_kill += r.p_hit * r.p_kill;
                if let Some(c) = r.cued {
                    d2s.push((r.at - c).secs());
                }
            }
            if matches!(r.stage, StrikeStage::HitNoKill | StrikeStage::Destroyed) {
                hits += 1;
            }
            if r.stage == StrikeStage::Destroyed {
                kills += 1;
            }
        }
    }
    d2s.sort_by(f64::total_cmp);
    let median = d2s.get(d2s.len() / 2).copied().unwrap_or(f64::NAN);
    // Expected values over the attacks that happened (the dice add only noise).
    let hit_rate = e_hit / terminal.max(1) as f64;
    let fail = 1.0 - e_kill / e_hit.max(1e-9);
    eprintln!("FPV sorties {sorties}, terminal attacks {terminal}, hits {hits}, kills {kills}; rolled hits {:.2}, expected hit rate per attack {hit_rate:.2}, per sortie {:.2}; fail-to-destroy {fail:.2}; detect-to-strike median {median:.0} s over {}", hits as f64 / terminal.max(1) as f64, hits as f64 / sorties.max(1) as f64, d2s.len());
    assert!(terminal >= 10, "too few FPV attacks to measure ({terminal})");
    // P2: networked detect-to-strike 2–5 min. Pooled over every terminal
    // attack (re-strikes and teams' own sightings too) the median is about
    // 4.9 min (293 s over 45, seeds 1–8); the detect-to-strike vignettes
    // measure the HQ chain itself. The guard leaves room for that pooling.
    assert!((120.0..=330.0).contains(&median), "detect-to-strike median {median:.0} s outside 2–5.5 min");
    // P9: 30–50% of attacks hit (10% for novices); 60–80% of hits fail to destroy.
    assert!((0.25..=0.55).contains(&hit_rate), "hit rate {hit_rate:.2}");
    assert!((0.55..=0.9).contains(&fail), "fail-to-destroy {fail:.2}");
}

/// [P15] precision under jamming and [P16] GNSS denial radius, from M4's jammers.
#[test]
fn precision_fires_lose_accuracy_under_gnss_jamming() {
    let sc = scenario("m4-drone-line");
    let mut sim = Sim::new(&sc, Seed(3), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(30.0));
    let gun = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(id(&sim, "b-guns"))).unwrap().pos;
    let tgt = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(id(&sim, "r-inf-1"))).unwrap().pos;
    let lethal = sim.ctx.catalogue.munition("pgm155").map(|m| m.lethal_radius_m).unwrap_or(10.0).min(10.0);
    let flight = 60.0;
    let jammed = c4mod::precision_accuracy(&sim, gun, tgt, flight, Some(Side::Blue));
    // Both jammers off: GNSS everywhere.
    for j in ["b-jam", "r-jam"] {
        let jid = id(&sim, j);
        sim.world.resource_mut::<C4>().emcon.insert(jid, Emcon::Silent);
    }
    let t = sim.now();
    sim.run_with_rules(t + bgws_core::SimDuration::from_secs(2.0));
    let clear = c4mod::precision_accuracy(&sim, gun, tgt, flight, Some(Side::Blue));
    let (pk_j, pk_c) = (jammed.p_within(lethal), clear.p_within(lethal));
    eprintln!("Pk within {lethal} m: clear {pk_c:.2} (CEP {:.1} m), jammed {pk_j:.2} (CEP {:.1} m, {:.0} s inertial)", clear.cep_m, jammed.cep_m, jammed.inertial_s);
    assert!(pk_c > 0.5, "clear Pk {pk_c}");
    assert!(pk_j < 0.1, "jammed Pk {pk_j}");
}

/// [P8] the kill zone's depth falls out of the drone link budgets.
#[test]
fn kill_zone_depth_from_link_budgets() {
    let sc = scenario("m4-drone-line");
    let mut sim = Sim::new(&sc, Seed(5), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(5.0));
    let mut depths = Vec::new();
    for team in ["b-fpv", "r-fpv-1", "r-fpv-2"] {
        let tid = id(&sim, team);
        let me = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(tid)).unwrap().pos;
        let side = sim.world.get::<bgws_sim::components::Ident>(sim.ctx.entity(tid)).unwrap().side;
        let dir = if side == Side::Blue { 1.0 } else { -1.0 };
        let mut depth = 0.0;
        for k in 1..=60 {
            let d = k as f64 * 500.0;
            let p = bgws_core::Enu::new(me.e, me.n + dir * d, 0.0);
            let q = c4mod::link_at(&sim, tid, p, 150.0).unwrap().quality;
            if q >= 0.5 {
                depth = d;
            }
        }
        eprintln!("{team}: RF FPV link holds to {:.1} km", depth / 1000.0);
        depths.push(depth);
    }
    let deepest = depths.iter().copied().fold(0.0, f64::max);
    assert!((5_000.0..=20_000.0).contains(&deepest), "kill zone depth {deepest} m");
    // Fibre: the spool, not the jammer.
    let spool = sim.figures().get("drones.fpv_fibre.range_m");
    assert!((5_000.0..=41_000.0).contains(&spool));
}

#[test]
fn out_of_comms_units_keep_to_their_intent() {
    let sc = scenario("m3-battalion-attack");
    let mut sim = Sim::new(&sc, Seed(4), default_figures()).unwrap();
    sim.run_with_rules(SimTime::from_secs(120.0));
    let unit = id(&sim, "b-a-1");
    let e = sim.ctx.entity(unit);
    let mission0 = sim.world.get::<bgws_sim::components::Act>(e).unwrap().mission.clone();
    assert!(mission0.purpose.starts_with("attack"), "{:?}", mission0.purpose);
    let start = sim.world.get::<bgws_sim::components::Kin>(e).unwrap().pos;
    comms::cut_off(&mut sim, unit);
    sim.run_with_rules(SimTime::from_secs(60.0 * 60.0));
    let ev = sim.drain_events();
    let ooc = ev.iter().filter(|x| x.kind == "c2.out_of_comms" && x.entity == Some(unit)).count();
    let later_orders = ev.iter().filter(|x| x.kind == "c2.order_received" && x.entity == Some(unit) && x.t > SimTime::from_secs(120.0)).count();
    let a = sim.world.get::<bgws_sim::components::Act>(e).unwrap();
    let alive = !sim.world.get::<bgws_sim::components::Health>(e).unwrap().destroyed;
    let now = sim.world.get::<bgws_sim::components::Kin>(e).unwrap().pos;
    eprintln!("out-of-comms questions {ooc}, later orders {later_orders}, mission {:?}, moved {:.0} m, alive {alive}", a.mission.purpose, now.sub(start).horizontal());
    assert!(ooc >= 1, "the cut-off unit was never out of comms");
    assert_eq!(later_orders, 0, "orders reached a unit with no radios");
    // It keeps its mission (the intent) and has acted on it.
    assert_eq!(a.mission.purpose, mission0.purpose);
    assert!(now.sub(start).horizontal() > 300.0 || !alive, "it stood still");
    // Others still get the HQ's later orders and reports reach the COP.
    let others = ev.iter().filter(|x| x.kind == "c2.order_received" && x.entity != Some(unit)).count() + ev.iter().filter(|x| x.kind == "c2.plan_issued").count();
    assert!(others > 0 || c2::cop_latencies(&sim).len() > 1);
}

fn mean_latency(sim: &Sim, units: &[EntityId], after: f64) -> (f64, usize) {
    let l: Vec<f64> = c2::cop_latencies(sim).into_iter().filter(|x| units.contains(&x.observer) && x.at.secs() > after).map(|x| x.latency_s).collect();
    (l.iter().sum::<f64>() / l.len().max(1) as f64, l.len())
}

#[test]
fn losing_an_hq_slows_its_units_reports() {
    let sc = scenario("m3-battalion-attack");
    let run = |kill: bool| {
        let mut sim = Sim::new(&sc, Seed(1), default_figures()).unwrap();
        sim.run_with_rules(SimTime::from_secs(120.0));
        if kill {
            let post = sim.ctx.entity(sim.ctx.by_name["b-c-hq"]);
            let mut h = sim.world.get_mut::<bgws_sim::components::Health>(post).unwrap();
            h.fit = 0;
            h.destroyed = true;
        }
        sim.run_with_rules(SimTime::from_secs(50.0 * 60.0));
        let ev = sim.drain_events();
        let units = [sim.ctx.by_name["b-c-1"], sim.ctx.by_name["b-c-2"]];
        let (lat, n) = mean_latency(&sim, &units, 120.0);
        let k = kinds(&ev);
        (lat, n, k.get("c2.hq_lost").copied().unwrap_or(0), k.get("c2.handover").copied().unwrap_or(0))
    };
    let base = run(false);
    let lost = run(true);
    eprintln!("C company reports to a COP: baseline mean {:.0} s over {}; HQ lost mean {:.0} s over {} (lost {}, handovers {})", base.0, base.1, lost.0, lost.1, lost.2, lost.3);
    assert_eq!(base.2, 0);
    assert_eq!(lost.2, 1);
    assert_eq!(lost.3, 1, "the alternate took the seat");
    assert!(base.1 > 0 && lost.1 > 0);
    assert!(lost.0 > base.0 * 2.0 + 60.0, "HQ loss did not slow reporting: {:.0} vs {:.0}", lost.0, base.0);
}

#[test]
fn same_seed_same_game_and_threads_do_not_matter() {
    for (name, mins) in [("m3-battalion-attack", 20.0), ("m4-drone-line", 30.0)] {
        let sc = scenario(name);
        let run = |seed| {
            let mut sim = Sim::new(&sc, Seed(seed), default_figures()).unwrap();
            sim.run_with_rules(SimTime::from_secs(mins * 60.0));
            sim.fingerprint()
        };
        assert_eq!(run(3), run(3), "{name}");
        let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| run(7));
        let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(|| run(7));
        assert_eq!(one, many, "{name}: threads changed the game");
    }
}

#[test]
fn hq_situation_uses_the_cop_and_seats_answer() {
    let sc = scenario("m3-battalion-attack");
    let mut sim = Sim::new(&sc, Seed(2), default_figures()).unwrap();
    sim.set_seat(Side::Red, bgws_sim::SeatKind::Rules);
    sim.run_with_rules(SimTime::from_secs(40.0 * 60.0));
    let sit = sim.hq_situation(Side::Red, "test");
    assert_eq!(c2::hq_of(&sim, id(&sim, "r-0")).as_deref(), Some("red-hq"));
    // Units as reported: a reported position, not necessarily the truth.
    assert!(sit.units.iter().any(|u| u.alive));
    let ev = sim.drain_events();
    assert!(ev.iter().any(|e| e.kind == "c2.planning"), "no CCIR or rhythm plan in 40 min");
}

#[test]
fn air_defence_takes_on_fixed_wing_drones_beyond_counter_drone_range() {
    // M8: medium SAM batteries with radars reach ISR drones that the
    // short-range counter-drone sensors (sim.drones.cuas_range_m) cannot.
    let cuas = default_figures().get("sim.drones.cuas_range_m");
    let mut far = 0;
    for seed in 1..=3 {
        let (_, ev) = play("m8-joint-strike", seed, 15.0);
        far += ev
            .iter()
            .filter(|e| e.kind == "drones.shot_down")
            .filter(|e| e.data["means"] == "SAM" && e.data["rangeM"].as_f64().unwrap_or(0.0) > cuas)
            .count();
    }
    assert!(far > 0, "no SAM engaged a drone beyond {cuas} m");
}
