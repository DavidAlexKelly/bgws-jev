//! Direct fire: a unit fires the weapon most likely to knock its target out
//! (not its first mount), and a guided missile lands when it arrives.

use bgws_core::{Seed, SimTime};
use bgws_run::scengen::{build, standard, GroundSpec};
use bgws_scenario::v4::ScenarioV4;
use bgws_sim::{Sim, SimOptions};

fn game(name: &str, seed: u64, minutes: f64) -> Vec<bgws_core::LogEvent> {
    let mut spec = standard().into_iter().find(|s| s.name == name).unwrap();
    spec.ground = GroundSpec::Flat;
    let sc = ScenarioV4::from_json(&build(&spec).unwrap().to_string()).unwrap();
    let mut sim = Sim::with_options(&sc, Seed(seed), bgws_run::default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap();
    bgws_run::experiments::play_with_rules(&mut sim, SimTime::from_secs(minutes * 60.0));
    sim.drain_events()
}

fn num(e: &bgws_core::LogEvent, k: &str) -> f64 {
    e.data.get(k).and_then(serde_json::Value::as_f64).unwrap_or(f64::NAN)
}

#[test]
fn an_ifv_fires_its_missiles_beyond_its_cannons_reach_and_they_fly() {
    let mut beyond = 0;
    let mut checked = 0;
    for seed in 1..=3 {
        let ev = game("gen-company-attack", seed, 30.0);
        for (i, l) in ev.iter().enumerate().filter(|(_, e)| e.kind == "fire.launch") {
            let range = num(l, "rangeM");
            // The IFV's 30 mm cannon reaches 2 km; only its missile reaches farther.
            if l.text.contains("mech") && range > 2_000.0 {
                beyond += 1;
            }
            // The volley lands (a hit, a kill or a miss on that target) no sooner than its flight time.
            let flight = num(l, "flightS");
            if let Some(land) = ev[i + 1..].iter().find(|e| matches!(e.kind.as_str(), "fire.hit" | "fire.kill" | "fire.miss") && e.entity == l.entity && e.target == l.target && e.data.get("munition") == l.data.get("munition")) {
                let dt = (land.t - l.t).secs();
                assert!(dt + 1.0 >= flight, "landed {dt:.1} s after launch, flight {flight:.1} s");
                checked += 1;
            }
        }
    }
    assert!(beyond > 0, "no IFV fired a missile beyond its cannon's reach");
    assert!(checked > 0, "no missile volley landed");
}

/// A mount with two rounds loads the one most likely to knock out its
/// target: tanks load HEAT and IFVs HE for troops and soft targets.
#[test]
fn units_load_the_round_for_the_target() {
    let mut loads = std::collections::BTreeSet::new();
    for seed in 1..=3 {
        for e in game("gen-company-attack", seed, 30.0).iter().filter(|e| e.kind == "fire.load") {
            loads.insert(e.data["munition"].as_str().unwrap_or_default().to_string());
        }
    }
    assert!(loads.contains("he30") || loads.contains("heat120"), "no unit changed rounds: {loads:?}");
}

/// An IFV's infantry get out as a foot unit and fight; a side's strength
/// never goes up (troops aboard count from the start).
#[test]
fn an_ifvs_infantry_get_out_and_strength_never_rises() {
    use bgws_core::Side;
    // On its own (generated) ground: on flat open ground the fight is over
    // at long range before anyone closes.
    let spec = standard().into_iter().find(|s| s.name == "gen-company-attack").unwrap();
    assert!(!matches!(spec.ground, GroundSpec::Pack { .. }), "needs no theatre pack");
    let mut sc = ScenarioV4::from_json(&build(&spec).unwrap().to_string()).unwrap();
    // The fight, not who wins: the objectives stay, no breakpoint ends it.
    sc.victory.retain(|v| matches!(v, bgws_scenario::v4::VictoryCondition::Objective { .. }));
    let (mut out, mut busy) = (0, 0);
    for seed in 1..=3 {
        // Out at first contact (the mechanism, not the drill's distances).
        let mut f = bgws_run::default_figures();
        f.set("sim.dismount.contact_m", 3000.0).unwrap();
        let mut sim = Sim::new(&sc, Seed(seed), f).unwrap();
        let mut last = sim.strength();
        // Where each foot unit made in the game was first seen, and how far it got.
        let mut walked: std::collections::BTreeMap<String, (bgws_core::Enu, f64)> = Default::default();
        for t in 1..=180 {
            if bgws_run::experiments::play_with_rules(&mut sim, SimTime::from_secs(t as f64 * 10.0)).is_some() {
                break;
            }
            let now = sim.strength();
            for side in [Side::Blue, Side::Red] {
                let (a, b) = (last.get(&side).copied().unwrap_or(0.0), now.get(&side).copied().unwrap_or(0.0));
                assert!(b <= a + 1e-9, "seed {seed}: {side:?} strength rose from {a:.3} to {b:.3} at {t}0 s");
            }
            last = now;
            for (name, id) in sim.ctx.by_name.iter().filter(|(n, _)| n.ends_with("-dismounts")) {
                if let Some(k) = sim.world.get::<bgws_sim::components::Kin>(sim.ctx.entity(*id)) {
                    let w = walked.entry(name.clone()).or_insert((k.pos, 0.0));
                    w.1 = w.1.max(k.pos.sub(w.0).horizontal());
                }
            }
        }
        let ev = sim.drain_events();
        for d in ev.iter().filter(|e| e.kind == "unit.dismounted") {
            out += 1;
            let foot = d.target.expect("the foot unit");
            let name = sim.ctx.by_name.iter().find(|(_, v)| **v == foot).map(|(k, _)| k.clone()).unwrap();
            let moved = walked.get(&name).map(|w| w.1).unwrap_or(0.0);
            let acted = ev.iter().any(|e| e.entity == Some(foot) && e.t > d.t);
            if acted || moved > 50.0 {
                busy += 1;
            }
        }
    }
    assert!(out > 0, "no infantry got out");
    // Some get out just before the end; most go on with the fight.
    assert!(busy * 2 >= out, "{busy} of {out} foot units fired or moved");
}

/// Crews of one troop quality differ (skill and leadership are drawn for
/// each unit), and the same seed draws the same crews.
#[test]
fn crews_vary_and_replay_the_same() {
    let spec = standard().into_iter().find(|s| s.name == "gen-company-attack").unwrap();
    let sc = ScenarioV4::from_json(&build(&spec).unwrap().to_string()).unwrap();
    let crews = |seed: u64| {
        let sim = Sim::new(&sc, Seed(seed), bgws_run::default_figures()).unwrap();
        sim.ctx.order.iter().filter_map(|&e| sim.world.get::<bgws_sim::components::Crew>(e).map(|c| (c.skill.base, c.leadership))).collect::<Vec<_>>()
    };
    let (a, b, c) = (crews(1), crews(1), crews(2));
    assert_eq!(a, b, "the same seed, the same crews");
    assert_ne!(a, c, "another seed, other crews");
    let skills: std::collections::BTreeSet<u64> = a.iter().map(|x| x.0.to_bits()).collect();
    assert!(skills.len() > a.len() / 2, "crews of one quality all alike");
}
