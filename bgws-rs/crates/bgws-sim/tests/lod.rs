//! Level of detail, aggregated combat, quiet skipping and world victory
//! conditions in real games (RUST_ENGINE.md §6.3, §6.4, §8.13, §18 phase 7).

use bgws_core::{LogEvent, Seed, Side, SimDuration, SimTime};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::components::{Health, Lod};
use bgws_sim::domains::lod::{self, synthetic::*};
use bgws_sim::{default_figures, Outcome, Sim, Step};
use serde_json::json;

fn mins(m: f64) -> SimTime {
    SimTime::from_secs(m * 60.0)
}

/// Play to `limit` with the rules, quiet skipping or not.
fn play(sim: &mut Sim, limit: SimTime, quiet: bool) -> Option<Outcome> {
    loop {
        let step = if quiet { sim.advance_quiet(limit) } else { sim.advance(limit) };
        match step {
            Step::Advanced => return None,
            Step::Over(o) => return Some(o),
            Step::NeedsAnswers { questions, .. } => {
                let a = sim.rules_answers(&questions);
                sim.answer(a).expect("rules answers");
            }
        }
    }
}

fn count(ev: &[LogEvent], kind: &str) -> usize {
    ev.iter().filter(|e| e.kind == kind).count()
}

fn platoons(side: Side, prefix: &str, platform: &str, n: usize, e0: f64, n0: f64) -> Vec<SynthForce> {
    (0..n).map(|k| SynthForce::new(&format!("{prefix}-{k}"), side, platform, 4, e0 + 400.0 * k as f64, n0)).collect()
}

/// Two formations 18 km apart: both aggregate; blue advances on red's start
/// and they meet while aggregated.
fn meeting(aggregate: bool) -> ScenarioV4 {
    let mut f = platoons(Side::Blue, "b", "generic/mbt", 3, -400.0, -9_000.0);
    f.extend(platoons(Side::Red, "r", "generic/mbt", 3, -400.0, 9_000.0));
    let mut s = SynthScenario::new("lod-meeting", f);
    s.aggregate = aggregate;
    s.build()
}

const FAST_TABLE: &str = r#"{ "source": "test: fast attrition", "default": { "k_a_on_b": 3.0, "k_b_on_a": 3.0 } }"#;

#[test]
fn aggregated_formations_fight_and_the_round_trip_keeps_losses() {
    let sc = meeting(true);
    let mut sim = Sim::new(&sc, Seed(5), default_figures()).unwrap();
    lod::load_attrition_table(&mut sim, FAST_TABLE).unwrap();
    // Run until the aggregate fight has taken some vehicles.
    let mut t = mins(1.0);
    let mut fought = false;
    while t <= mins(120.0) {
        play(&mut sim, t, false);
        let fs = lod::formations(&sim);
        if fs.iter().filter_map(|f| f.agg.as_ref()).any(|a| a.formation.losses() >= 2) {
            fought = true;
            break;
        }
        t += SimDuration::from_secs(60.0);
    }
    let ev = sim.drain_events();
    assert!(count(&ev, "lod.aggregate") >= 2, "both formations aggregate");
    assert!(fought && count(&ev, "lod.attrition") > 0, "aggregate fight: {}", count(&ev, "lod.attrition"));
    assert_eq!(count(&ev, "sense.detect"), 0, "aggregated units do not sense");
    // While aggregated the losses are already on the members' Health.
    let health = |sim: &Sim, id: bgws_core::EntityId| sim.world.get::<Health>(sim.ctx.entity(id)).unwrap().clone();
    let fs = lod::formations(&sim);
    for f in fs.iter().filter(|f| f.aggregated()) {
        let a = f.agg.as_ref().unwrap();
        let lost: u32 = f.members.iter().map(|&m| 4 - health(&sim, m).fit).sum();
        assert_eq!(lost, a.formation.losses(), "{}: Health shows the formation's losses", f.name);
    }
    let before: Vec<(bgws_core::EntityId, u32, bool)> = fs.iter().flat_map(|f| f.members.clone()).map(|m| (m, health(&sim, m).fit, health(&sim, m).destroyed)).collect();
    // Round trip: disaggregate both now, and the losses stay exactly where they were.
    for f in &fs {
        assert!(lod::disaggregate(&mut sim, f.id, "test focus"));
    }
    for (m, fit, dead) in &before {
        let h = health(&sim, *m);
        assert_eq!((h.fit, h.destroyed), (*fit, *dead), "member {m:?} unchanged by disaggregation");
        assert_eq!(sim.world.get::<Lod>(sim.ctx.entity(*m)).copied(), Some(Lod::Entity));
    }
    let ev = sim.drain_events();
    assert_eq!(count(&ev, "lod.disaggregate"), 2);
    let d = ev.iter().find(|e| e.kind == "lod.disaggregate").unwrap();
    assert_eq!(d.data["killedNow"], json!(0), "nothing new dies on the way back: {}", d.data);
    // And over the rest of the game nobody comes back to life.
    let later = sim.now() + SimDuration::from_secs(600.0);
    play(&mut sim, later, false);
    for (m, fit, dead) in &before {
        let h = health(&sim, *m);
        assert!(h.fit <= *fit && (h.destroyed || !*dead), "killed stays killed: {m:?}");
    }
}

#[test]
fn a_far_formation_aggregates_and_disaggregates_on_approach() {
    // Red holds 10 km north; a blue command post 1 km south keeps red at
    // entity level; the blue main body starts 25 km south and advances.
    let mut f = platoons(Side::Blue, "main", "generic/mbt", 3, -400.0, -25_000.0);
    f.push(SynthForce::new("post", Side::Blue, "generic/cp-company", 1, 0.0, -1_000.0));
    f.extend((0..3).map(|k| SynthForce::new(&format!("r-{k}"), Side::Red, "generic/infantrySection", 3, -600.0 + 600.0 * k as f64, 10_000.0)));
    let mut s = SynthScenario::new("lod-approach", f);
    s.aggregate = true;
    let sc = s.build();
    let mut sim = Sim::new(&sc, Seed(9), default_figures()).unwrap();
    let main: Vec<_> = (0..3).map(|k| sim.ctx.by_name[&format!("main-{k}")]).collect();
    let lod_of = |sim: &Sim, id| sim.world.get::<Lod>(sim.ctx.entity(id)).copied().unwrap();
    play(&mut sim, mins(1.0), false);
    assert!(main.iter().all(|&m| lod_of(&sim, m) == Lod::Aggregated), "the far main body aggregates");
    let red = sim.ctx.by_name["r-0"];
    assert_eq!(lod_of(&sim, red), Lod::Entity, "red is near an entity-level enemy");
    let mut t = mins(2.0);
    while t <= mins(150.0) && main.iter().all(|&m| lod_of(&sim, m) == Lod::Aggregated) {
        play(&mut sim, t, false);
        t += SimDuration::from_secs(30.0);
    }
    let ev = sim.drain_events();
    let agg = ev.iter().find(|e| e.kind == "lod.aggregate" && e.entity == Some(main[0])).expect("aggregated");
    let dis = ev.iter().find(|e| e.kind == "lod.disaggregate" && e.entity == Some(main[0])).expect("disaggregated on approach");
    eprintln!("aggregate at {}: {}\ndisaggregate at {}: {}", agg.t, agg.text, dis.t, dis.text);
    assert!(dis.t > agg.t);
    assert!(main.iter().all(|&m| lod_of(&sim, m) == Lod::Entity));
    // It came back before any red unit could have seen it (6 km sensors).
    let p = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(main[0])).unwrap().pos;
    let r = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(red)).unwrap().pos;
    let d = p.sub(r).horizontal();
    assert!(d > 8_000.0 && d < 13_000.0, "disaggregated at {d:.0} m");
    // No main-body sighting while it was aggregated.
    assert!(!ev.iter().any(|e| e.kind == "sense.detect" && main.contains(&e.entity.unwrap()) && e.t < dis.t));
}

#[test]
fn lod_runs_are_deterministic() {
    let sc = meeting(true);
    let run = || {
        let mut sim = Sim::new(&sc, Seed(21), default_figures()).unwrap();
        lod::load_attrition_table(&mut sim, FAST_TABLE).unwrap();
        play(&mut sim, mins(40.0), false);
        sim.fingerprint()
    };
    let a = run();
    assert_eq!(a, run());
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(run);
    let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(run);
    assert_eq!(one, many);
    assert_eq!(a, one);
}

#[test]
fn m6_with_aggregation_is_deterministic_across_threads() {
    let path = format!("{}/../../data/scenarios/m6-sustained-offensive.json", env!("CARGO_MANIFEST_DIR"));
    let sc = ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert!(sc.scope.detail.aggregate_away_from_contact);
    let run = || {
        let mut sim = Sim::new(&sc, Seed(3), default_figures()).unwrap();
        play(&mut sim, mins(15.0), false);
        let ev = sim.drain_events();
        (sim.fingerprint(), count(&ev, "lod.aggregate"))
    };
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(run);
    let many = rayon::ThreadPoolBuilder::new().num_threads(4).build().unwrap().install(run);
    assert_eq!(one, many);
    assert!(one.1 > 0, "m6 aggregates its far formations");
}

/// Two small forces 16 km apart, blue advancing: a long quiet approach, then a fight.
fn approach(skip_quiet: bool) -> ScenarioV4 {
    let mut f = platoons(Side::Blue, "b", "generic/mbt", 3, -400.0, -8_000.0);
    f.extend(platoons(Side::Red, "r", "generic/ifv", 3, -400.0, 8_000.0));
    let mut s = SynthScenario::new("quiet-approach", f);
    s.skip_quiet = skip_quiet;
    s.build()
}

#[test]
fn quiet_skipping_never_changes_what_happens() {
    let stepped = {
        let mut sim = Sim::new(&approach(false), Seed(4), default_figures()).unwrap();
        play(&mut sim, mins(75.0), false);
        let ev = sim.drain_events();
        (sim.fingerprint(), ev)
    };
    let mut sim = Sim::new(&approach(false), Seed(4), default_figures()).unwrap();
    assert_eq!(sim.next_interesting_time(), None, "the HQ questions are waiting");
    play(&mut sim, SimTime::from_secs(5.0), true);
    assert!(sim.next_interesting_time().is_some(), "quiet once the plans are in");
    // The next interesting time may be a domain's own timer; what matters for
    // skipping sensing is the detection horizon.
    let h = lod::quiet::detection_horizon(&sim, &lod::LodParams::from_figures(&sim.ctx.figures));
    assert!(h > sim.now() + SimDuration::from_secs(60.0), "nothing can be seen for a while: {h}");
    play(&mut sim, mins(75.0), true);
    let (skipped, horizons) = lod::quiet::stats(&sim);
    let ev = sim.drain_events();
    let first = |ev: &[LogEvent]| ev.iter().find(|e| e.kind == "sense.detect").map(|e| e.t);
    eprintln!("skipped {skipped} sensing passes, {horizons} horizons; first contact {:?}", first(&ev));
    assert!(skipped > 300, "a long quiet stretch was skipped: {skipped}");
    assert!(first(&stepped.1).is_some(), "the forces meet");
    assert_eq!(first(&ev), first(&stepped.1), "same first contact");
    assert_eq!(count(&ev, "fire.kill"), count(&stepped.1, "fire.kill"));
    assert_eq!(sim.fingerprint(), stepped.0, "the same game, event for event");
    // The scope switch does the same through `advance`.
    let mut sim = Sim::new(&approach(true), Seed(4), default_figures()).unwrap();
    play(&mut sim, mins(75.0), false);
    assert!(lod::quiet::stats(&sim).0 > 300);
    let ev2 = sim.drain_events();
    assert_eq!(ev2.iter().filter(|e| e.kind != "sim.start").map(|e| (e.t, e.kind.clone(), e.text.clone())).collect::<Vec<_>>(), ev.iter().filter(|e| e.kind != "sim.start").map(|e| (e.t, e.kind.clone(), e.text.clone())).collect::<Vec<_>>());
}

#[test]
fn no_quiet_in_contact() {
    let mut f = platoons(Side::Blue, "b", "generic/mbt", 2, 0.0, -1_000.0);
    f.extend(platoons(Side::Red, "r", "generic/mbt", 2, 0.0, 1_000.0));
    let mut s = SynthScenario::new("contact", f);
    s.planners = false;
    let mut sim = Sim::new(&s.build(), Seed(1), default_figures()).unwrap();
    // 2 km apart: well inside reach, so the horizon is now.
    let h = sim.next_interesting_time().expect("no contact yet");
    assert_eq!(h, sim.now());
    play(&mut sim, mins(3.0), true);
    let ev = sim.drain_events();
    assert!(count(&ev, "sense.detect") > 0);
    assert_eq!(sim.next_interesting_time(), None, "units that hold contacts are not quiet");
}

// ───────────────────────── victory conditions ─────────────────────────

fn victory_sim(forces: Vec<SynthForce>, victory: serde_json::Value) -> Sim {
    let mut s = SynthScenario::new("victory", forces);
    s.planners = false;
    s.victory = victory;
    Sim::new(&s.build(), Seed(2), default_figures()).unwrap()
}

fn kill(sim: &mut Sim, name: &str) {
    let e = sim.ctx.entity(sim.ctx.by_name[name]);
    let mut h = sim.world.get_mut::<Health>(e).unwrap();
    h.fit = 0;
    h.destroyed = true;
}

#[test]
fn target_set_destroyed_wins() {
    let forces = vec![
        SynthForce::new("b-1", Side::Blue, "generic/mbt", 4, 0.0, -30_000.0),
        SynthForce::new("r-sam", Side::Red, "generic/samMedium", 1, 0.0, 30_000.0),
        SynthForce::new("r-cp", Side::Red, "generic/cp-battalion", 1, 2_000.0, 30_000.0),
        SynthForce::new("r-1", Side::Red, "generic/mbt", 4, 0.0, 32_000.0),
    ];
    let mut sim = victory_sim(forces, json!([{ "kind": "targetSet", "side": "blue", "targets": ["r-sam", "r-cp"], "share": 0.5, "label": "Air defence down" }]));
    assert_eq!(play(&mut sim, mins(1.0), false), None);
    kill(&mut sim, "r-sam");
    let o = play(&mut sim, mins(2.0), false).expect("won");
    assert_eq!(o.winner, Some(Side::Blue));
    assert!(o.reason.contains("Air defence down") && o.reason.contains("1 of 2"), "{}", o.reason);
    assert!(o.at <= SimTime::from_secs(71.0), "judged within 10 s: {}", o.at);
}

#[test]
fn air_superiority_must_be_held() {
    let at = geo(0.0, 0.0, 0.0);
    let mut fighter = SynthForce::new("r-jet", Side::Red, "generic/fighter4", 1, 3_000.0, 2_000.0);
    fighter.h = 6_000.0;
    let forces = vec![SynthForce::new("b-1", Side::Blue, "generic/mbt", 4, 0.0, -40_000.0), fighter, SynthForce::new("r-1", Side::Red, "generic/mbt", 4, 0.0, 40_000.0)];
    let mut sim = victory_sim(forces, json!([{ "kind": "airSuperiority", "side": "blue", "at": at, "radiusM": 20000.0, "holdS": 300.0, "label": "Air over the front" }]));
    assert_eq!(play(&mut sim, mins(2.0), false), None, "an enemy jet overhead");
    kill(&mut sim, "r-jet");
    let o = play(&mut sim, mins(20.0), false).expect("won after holding");
    let ev = sim.drain_events();
    let met = ev.iter().find(|e| e.kind == "victory.met").expect("met logged");
    assert_eq!(o.winner, Some(Side::Blue));
    assert!((o.at - met.t).secs() >= 300.0 && (o.at - met.t).secs() < 311.0, "held 5 min: met {} won {}", met.t, o.at);
    assert!(met.t <= SimTime::from_secs(130.0));
}

#[test]
fn sea_control_and_through_and_all() {
    // Sea control: own frigate in the area, no enemy vessel.
    let forces = vec![SynthForce::new("b-ffg", Side::Blue, "generic/frigate", 1, 0.0, 0.0), SynthForce::new("r-1", Side::Red, "generic/mbt", 4, 0.0, 40_000.0)];
    let mut sim = victory_sim(forces, json!([{ "kind": "seaControl", "side": "blue", "at": geo(0.0, 0.0, 0.0), "radiusM": 10000.0, "holdS": 60.0 }]));
    let o = play(&mut sim, mins(5.0), false).expect("sea control held");
    assert_eq!(o.winner, Some(Side::Blue));
    assert!(o.at.secs() >= 70.0 && o.at.secs() <= 81.0, "{}", o.at);
    // A red sub in the area denies it.
    let forces = vec![SynthForce::new("b-ffg", Side::Blue, "generic/frigate", 1, 0.0, 0.0), SynthForce::new("r-ssk", Side::Red, "generic/ssk", 1, 3_000.0, 0.0), SynthForce::new("r-1", Side::Red, "generic/mbt", 4, 0.0, 40_000.0)];
    let mut sim = victory_sim(forces, json!([{ "kind": "seaControl", "side": "blue", "at": geo(0.0, 0.0, 0.0), "radiusM": 10000.0, "holdS": 60.0 }]));
    assert_eq!(play(&mut sim, mins(5.0), false), None);
    // Through and all-of: red's tank is already in the area, the target falls later.
    let forces = vec![
        SynthForce::new("b-1", Side::Blue, "generic/mbt", 4, 0.0, -40_000.0),
        SynthForce::new("r-1", Side::Red, "generic/mbt", 4, 0.0, 30_000.0),
        SynthForce::new("b-cp", Side::Blue, "generic/cp-battalion", 1, 0.0, -42_000.0),
    ];
    let through = json!({ "kind": "through", "side": "red", "ids": ["r-1"], "at": geo(0.0, 30_500.0, 0.0), "radiusM": 2000.0, "count": 1 });
    let target = json!({ "kind": "targetSet", "side": "red", "targets": ["b-cp"], "share": 1.0 });
    let mut sim = victory_sim(forces.clone(), json!([{ "kind": "all", "side": "red", "of": [through, target], "label": "Through and the post down" }]));
    assert_eq!(play(&mut sim, mins(2.0), false), None, "only half the aims");
    kill(&mut sim, "b-cp");
    let o = play(&mut sim, mins(4.0), false).expect("all met");
    assert_eq!(o.winner, Some(Side::Red));
    assert!(o.reason.contains("1 of 1 through"), "{}", o.reason);
    let mut sim = victory_sim(forces, json!([through]));
    let o = play(&mut sim, mins(1.0), false).expect("through at once");
    assert_eq!((o.winner, o.at), (Some(Side::Red), SimTime::from_secs(10.0)));
}

// ─────────────── phase 7: aggregate models agree with entity runs ───────────────

/// Fit Lanchester coefficients from entity-level runs of a symmetric tank
/// fight, then check the aggregate model with those coefficients predicts
/// the entity model's mean losses (the meta-model of §8.13).
#[test]
fn aggregate_attrition_agrees_with_entity_runs() {
    use bgws_ops::aggregate::{attrition_step, fit_attrition, EngagementEnv, FireType, Law, MemberRecord, Posture, TerrainClass};
    let mut f = platoons(Side::Blue, "b", "generic/mbt", 3, -400.0, -1_100.0);
    f.extend(platoons(Side::Red, "r", "generic/mbt", 3, -400.0, 1_100.0));
    let mut s = SynthScenario::new("fit-vignette", f);
    s.planners = false;
    let sc = s.build();
    let minutes = 6.0;
    let mut samples = Vec::new();
    let mut entity_losses = Vec::new();
    for seed in 0..10u64 {
        let mut sim = Sim::new(&sc, Seed(100 + seed), default_figures()).unwrap();
        // Sample strengths each 30 s for the mean.
        let fit = |sim: &Sim, side: Side| -> f64 { sim.ctx.order.iter().filter_map(|&e| Some((sim.world.get::<bgws_sim::components::Ident>(e)?, sim.world.get::<Health>(e)?))).filter(|(i, _)| i.side == side).map(|(_, h)| h.fit as f64).sum() };
        let (mut sa, mut sb, mut n) = (0.0, 0.0, 0.0);
        let mut t = SimTime::ZERO;
        while t < mins(minutes) {
            sa += fit(&sim, Side::Blue);
            sb += fit(&sim, Side::Red);
            n += 1.0;
            t += SimDuration::from_secs(30.0);
            play(&mut sim, t, false);
        }
        let (la, lb) = (12.0 - fit(&sim, Side::Blue), 12.0 - fit(&sim, Side::Red));
        entity_losses.push(la + lb);
        samples.push(bgws_ops::aggregate::EngagementSample { losses_a: la as u64, losses_b: lb as u64, strength_a: sa / n, strength_b: sb / n, initial_a: 12.0, initial_b: 12.0, hours: minutes / 60.0 });
    }
    let fitted = fit_attrition(&samples, Law::Square).expect("fit");
    let k = fitted.coeffs();
    eprintln!("fitted k: {k:?}; entity losses per run {entity_losses:?}");
    assert!(k.k_a_on_b > 0.0 && k.k_b_on_a > 0.0, "the vignette produces kills");
    // The aggregate model over the same time, many draws.
    let figures = default_figures();
    let rec = |i: u32| MemberRecord { entity: bgws_core::EntityId(i), class: "armour".into(), alive: true, stores: 1.0, fatigue: 0.0 };
    let recs: Vec<_> = (0..12).map(rec).collect();
    let env = EngagementEnv { terrain: TerrainClass::Open, supply: (1.0, 1.0) };
    let mut total = 0.0;
    let draws = 2_000;
    for d in 0..draws {
        let mut a = bgws_ops::aggregate::aggregate(1, Side::Blue, &recs, 1, FireType::Aimed, Posture::Attack);
        let mut b = bgws_ops::aggregate::aggregate(2, Side::Red, &recs, 1, FireType::Aimed, Posture::Attack);
        let mut rng = Seed(d).stream(bgws_core::StreamKey::new("test.agg", 0, 0, 0));
        for _ in 0..(minutes * 6.0) as usize {
            attrition_step(&mut a, &mut b, &k, &env, 10.0 / 3600.0, &figures, &mut rng);
        }
        total += (a.losses() + b.losses()) as f64;
    }
    let agg_mean = total / draws as f64;
    let ent_mean = entity_losses.iter().sum::<f64>() / entity_losses.len() as f64;
    let sd = (entity_losses.iter().map(|x| (x - ent_mean).powi(2)).sum::<f64>() / (entity_losses.len() as f64 - 1.0)).sqrt();
    let se = sd / (entity_losses.len() as f64).sqrt();
    eprintln!("mean losses: entity {ent_mean:.2} (se {se:.2}), aggregate {agg_mean:.2}");
    assert!((agg_mean - ent_mean).abs() <= (3.0 * se).max(0.25 * ent_mean), "aggregate {agg_mean:.2} vs entity {ent_mean:.2} ± {se:.2}");
}
