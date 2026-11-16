//! Two of our simulations as HLA federates through the Federate Protocol and
//! the RPR FOM (docs/design/INTEROP_PLAN.md phase 3), on the mock RTI: one
//! plays blue, the other red, each owning its side.
//!
//! - Time-managed: they fight in lock step; each judges damage to its own,
//!   nothing is owned twice, no detonation is judged twice; the whole
//!   federation run twice with the same seeds gives the same two games; and
//!   each record replays exactly with no RTI.
//! - Real time: each sees the other's units.

use bgws_core::{Seed, Side, SimTime};
use bgws_interop::hla::link::{HlaLink, HlaSettings};
use bgws_interop::hla::mock::MockRti;
use bgws_run::{DeciderChoice, HlaLinkAdapter, Host, HostOptions};
use bgws_scenario::record::GameRecord;
use bgws_scenario::v4::{InteropSpec, ScenarioV4};
use std::collections::BTreeSet;

/// The company attack with only one side's forces (and its HQ).
fn half(side: &str) -> ScenarioV4 {
    let path = format!("{}/../../data/scenarios/gen-company-attack.json", env!("CARGO_MANIFEST_DIR"));
    let sc = ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    bgws_run::one_side(&sc, if side == "blue" { Side::Blue } else { Side::Red }).unwrap()
}

fn rules() -> HostOptions {
    HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }
}

fn spec(site: u16) -> InteropSpec {
    InteropSpec { exercise: 7, site, application: 1, ..Default::default() }
}

struct Played {
    record: GameRecord,
    fingerprint: String,
    own: usize,
    remote_sites: BTreeSet<u16>,
    remote: usize,
    lost: f64,
    sent: u64,
}

/// One side as a time-managed federate, to `minutes`.
fn federate(addr: String, side: &'static str, site: u16, minutes: f64) -> Played {
    let mut host = Host::new(&half(side), Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let mut s = HlaSettings::new(&addr, "two-federates", side);
    s.lookahead = Some(1.0);
    s.grant_wait = std::time::Duration::from_secs(120);
    s.federates = 2;
    let link = HlaLink::join(s, spec(site)).unwrap();
    let own = host.sim.ctx.order.len();
    let mut adapter = HlaLinkAdapter::new(link, |_: &bgws_sim::Sim| {});
    host.play_lockstep(SimTime::from_secs(minutes * 60.0), &mut adapter).unwrap();
    assert!(adapter.errors.is_empty(), "{side}: {:?}", adapter.errors);
    adapter.link.leave().unwrap();
    let side_ = if side == "blue" { Side::Blue } else { Side::Red };
    Played {
        fingerprint: host.report().fingerprint,
        own,
        remote_sites: host.sim.ctx.remote.keys().map(|k| k.site).collect(),
        remote: host.sim.ctx.remote.len(),
        lost: 1.0 - host.sim.strength().get(&side_).copied().unwrap_or(1.0),
        sent: adapter.link.sent,
        record: host.finished_record(),
    }
}

fn federation(minutes: f64) -> (Played, Played) {
    let rti = MockRti::start().unwrap();
    let (a, b) = (rti.address(), rti.address());
    let blue = std::thread::spawn(move || federate(a, "blue", 1, minutes));
    let red = std::thread::spawn(move || federate(b, "red", 2, minutes));
    (blue.join().unwrap(), red.join().unwrap())
}

#[test]
fn blue_and_red_federates_fight_in_lock_step_the_same_way_every_time() {
    let (blue, red) = federation(12.0);
    assert_eq!(blue.remote_sites, BTreeSet::from([2]), "blue's remote entities are red's");
    assert_eq!(red.remote_sites, BTreeSet::from([1]), "red's remote entities are blue's");
    assert!(blue.remote >= red.own - 2, "blue sees {} of red's {}", blue.remote, red.own);
    assert!(red.remote >= blue.own - 2, "red sees {} of blue's {}", red.remote, blue.own);
    let judged = |p: &Played| p.record.external.iter().filter(|x| x.input["input"] == "detonation").count();
    assert!(judged(&blue) + judged(&red) > 0, "no detonation crossed between the federates");
    for p in [&blue, &red] {
        let mut seen = BTreeSet::new();
        for x in p.record.external.iter().filter(|x| x.input["input"] == "detonation") {
            assert!(seen.insert((x.input["event"].to_string(), x.input["target"].to_string(), x.input["at"].to_string())), "the same detonation came in twice");
        }
        // Inputs apply only at whole seconds.
        assert!(p.record.external.iter().all(|x| x.at.secs().fract() == 0.0));
    }
    eprintln!(
        "blue sees {} remote, red sees {}; detonations judged: blue {}, red {}; losses: blue {:.0}%, red {:.0}%; sent: blue {}, red {}",
        blue.remote,
        red.remote,
        judged(&blue),
        judged(&red),
        100.0 * blue.lost,
        100.0 * red.lost,
        blue.sent,
        red.sent
    );

    // The same federation again: the same two games.
    let (blue2, red2) = federation(12.0);
    assert_eq!(blue.fingerprint, blue2.fingerprint, "blue's game differs between runs");
    assert_eq!(red.fingerprint, red2.fingerprint, "red's game differs between runs");

    // Each replays exactly with no RTI.
    for p in [blue, red] {
        let mut replay = Host::replay(p.record.clone(), bgws_run::default_figures()).unwrap();
        replay.play_lockstep(p.record.ends_at(), &mut bgws_run::OnTick(|_| {})).unwrap();
        replay.verify_replay().expect("a federate's game replays exactly");
        assert_eq!(replay.report().fingerprint, p.fingerprint);
    }
}

#[test]
fn real_time_federates_see_each_other() {
    let rti = MockRti::start().unwrap();
    let join = |side: &str, site: u16| HlaLinkAdapter::new(HlaLink::join(HlaSettings::new(&rti.address(), "real-time", side), spec(site)).unwrap(), |_: &bgws_sim::Sim| {});
    let mut blue = Host::new(&half("blue"), Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let mut red = Host::new(&half("red"), Seed(1), bgws_run::default_figures(), &rules()).unwrap();
    let (mut bl, mut rl) = (join("blue", 1), join("red", 2));
    for t in 1..=60u32 {
        let at = SimTime::from_secs(t as f64);
        blue.play_realtime(at, None, &mut bl).unwrap();
        red.play_realtime(at, None, &mut rl).unwrap();
        // Let the RTI pass the second's updates on.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(bl.errors.is_empty() && rl.errors.is_empty(), "{:?} {:?}", bl.errors, rl.errors);
    let (own_blue, own_red) = (half("blue").forces.len(), half("red").forces.len());
    assert!(blue.sim.ctx.remote.len() >= own_red - 2, "blue sees {} of red's {own_red}", blue.sim.ctx.remote.len());
    assert!(red.sim.ctx.remote.len() >= own_blue - 2, "red sees {} of blue's {own_blue}", red.sim.ctx.remote.len());
}
