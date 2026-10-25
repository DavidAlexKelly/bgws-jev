use bgws_core::{Seed, SimTime};
use bgws_scenario::v4::ScenarioV4;
//! Print a scenario's state at a few moments: `cargo run -p bgws-sim --example debug -- <scenario>` (FLAT=1 for flat ground).
use bgws_sim::{default_figures, Viewer};
fn main() {
    let name = std::env::args().nth(1).unwrap_or("skirmish-advance-to-contact-v1".into());
    let path = format!("{}/../../data/scenarios/{name}.json", env!("CARGO_MANIFEST_DIR"));
    let sc = ScenarioV4::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
    let flat = std::env::var("FLAT").is_ok();
    let mut sim = bgws_sim::Sim::with_options(&sc, Seed(7), default_figures(), bgws_sim::SimOptions { flat_ground: flat, ..Default::default() }).unwrap();
    for m in [0.0, 1.0, 5.0, 10.0, 20.0, 40.0] {
        sim.run_with_rules(SimTime::from_secs(m * 60.0 + 1.0));
        let f = sim.view(Viewer::Umpire);
        println!("t={}", f.clock);
        for e in &f.entities { println!("  {} {:?} {:.5},{:.5} {} {} fit {}/{}", e.name, e.side, e.lat, e.lon, e.activity, e.status, e.fit, e.total); }
        for ev in sim.drain_events().iter().filter(|e| !e.kind.starts_with("decide.q")).take(12) { println!("    [{}] {} {}", ev.t, ev.kind, ev.text); }
    }
}
