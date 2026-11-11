//! Two of our simulations over DIS on localhost (docs/design/INTEROP_PLAN.md
//! phase 2): one plays blue, the other red, each owning its own side. Each
//! sees the other's units as remote entities and fights them; each judges
//! the damage to its own; nothing is owned twice and no detonation is judged
//! twice; and blue's game replays exactly from its record, with no network.

use bgws_core::{Seed, Side, SimTime};
use bgws_interop::dis::DisPublisher;
use bgws_interop::dr::DrSettings;
use bgws_interop::link::{DisLink, DisReceiver};
use bgws_run::{DeciderChoice, DisLinkAdapter, Host, HostOptions};
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use std::collections::BTreeSet;

/// The company attack with only one side's forces (and its HQ).
fn half(side: &str) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/gen-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let gone: Vec<String> = v["forces"].as_array().unwrap().iter().filter(|f| f["side"] != side).map(|f| f["id"].as_str().unwrap().to_string()).collect();
    v["forces"].as_array_mut().unwrap().retain(|f| f["side"] == side);
    if let Some(hqs) = v.get_mut("command").and_then(|c| c.get_mut("hqs")).and_then(|h| h.as_array_mut()) {
        hqs.retain(|h| h["side"] == side);
        for h in hqs.iter_mut() {
            if let Some(subs) = h.get_mut("subordinates").and_then(|x| x.as_array_mut()) {
                subs.retain(|x| !gone.iter().any(|g| x == g.as_str()));
            }
        }
    }
    // The objectives give the missions (blue attacks, red defends); no
    // breakpoint ends it, since each simulation sees only its own side's strength.
    v["victory"].as_array_mut().unwrap().retain(|c| c["kind"] == "objective");
    ScenarioV4::from_json(&v.to_string()).unwrap()
}

fn rules() -> HostOptions {
    HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }
}

#[test]
fn blue_and_red_in_two_simulations_fight_over_dis() {
    let spec = |site: u16| InteropSpec { exercise: 7, site, application: 1, ..Default::default() };
    let (rx_blue, rx_red) = (DisReceiver::new("127.0.0.1:0", None).unwrap(), DisReceiver::new("127.0.0.1:0", None).unwrap());
    let (to_blue, to_red) = (rx_blue.local_addr().unwrap().to_string(), rx_red.local_addr().unwrap().to_string());
    let mut blue = Host::new(&half("blue"), Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let mut red = Host::new(&half("red"), Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let mut blue_link = DisLinkAdapter::new(DisLink::from_parts(DisPublisher::new(&to_red, spec(1), DrSettings::default()).unwrap(), Some(rx_blue)), |_: &bgws_sim::Sim| {});
    let mut red_link = DisLinkAdapter::new(DisLink::from_parts(DisPublisher::new(&to_blue, spec(2), DrSettings::default()).unwrap(), Some(rx_red)), |_: &bgws_sim::Sim| {});
    let own_blue = blue.sim.ctx.order.len();
    let own_red = red.sim.ctx.order.len();

    // In lockstep, a second at a time; each hears what the other sent the second before.
    let minutes = 12.0;
    for t in 1..=(minutes * 60.0) as u32 {
        let at = SimTime::from_secs(t as f64);
        blue.play_realtime(at, None, &mut blue_link).unwrap();
        red.play_realtime(at, None, &mut red_link).unwrap();
    }

    // Each sees the other's units, as remote entities with the other's site.
    let remote = |h: &Host, site: u16| -> usize {
        assert!(h.sim.ctx.remote.keys().all(|k| k.site == site), "a remote entity from the wrong simulation");
        h.sim.ctx.remote.len()
    };
    assert!(remote(&blue, 2) >= own_red - 2, "blue sees {} of red's {own_red}", remote(&blue, 2));
    assert!(remote(&red, 1) >= own_blue - 2, "red sees {} of blue's {own_blue}", remote(&red, 1));
    // Nothing owned twice: a simulation's own entities are never remote there.
    for h in [&blue, &red] {
        let rem: BTreeSet<_> = h.sim.ctx.remote.values().collect();
        assert!(rem.iter().all(|id| id.index() >= h.record.scenario.forces.len()), "an own entity taken as remote");
    }
    // They fought: each side's shots reached the other as detonations it judged.
    let judged = |h: &Host| h.record.external.iter().filter(|x| x.input["input"] == "detonation").count();
    let (blue_judged, red_judged) = (judged(&blue), judged(&red));
    assert!(blue_judged + red_judged > 0, "no detonation crossed between the simulations");
    for h in [&blue, &red] {
        let mut events = BTreeSet::new();
        for x in h.record.external.iter().filter(|x| x.input["input"] == "detonation") {
            let key = (x.input["event"].to_string(), x.input["target"].to_string(), x.input["at"].to_string());
            assert!(events.insert(key), "the same detonation came in twice");
        }
    }
    let lost = |h: &Host, side: Side| 1.0 - h.sim.strength().get(&side).copied().unwrap_or(1.0);
    eprintln!(
        "blue sees {} remote, red sees {}; detonations judged: blue {blue_judged}, red {red_judged}; losses: blue {:.0}%, red {:.0}%",
        blue.sim.ctx.remote.len(),
        red.sim.ctx.remote.len(),
        100.0 * lost(&blue, Side::Blue),
        100.0 * lost(&red, Side::Red)
    );

    // Blue's game replays exactly from its record (its inputs and answers at their times).
    let rec = blue.finished_record();
    assert!(!rec.external.is_empty(), "the record holds what came in");
    let mut replay = Host::replay(rec.clone(), bgws_run::default_figures()).unwrap();
    replay.play_realtime(rec.ends_at(), None, &mut bgws_run::OnTick(|_| {})).unwrap();
    replay.verify_replay().expect("blue's two-simulation game replays exactly");
}
