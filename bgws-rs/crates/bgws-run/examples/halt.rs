//! Trace an attack second by second: blue's units, strength and activity.
use bgws_core::{Seed, Side, SimTime};
use bgws_sim::components::{Act, Health, Ident};
use bgws_sim::{Sim, SimOptions};
fn main() {
    let sc0 = bgws_run::load_scenario(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/scenarios/skirmish-advance-to-contact-v1.json"))).unwrap();
    let mut sc = sc0.clone();
    sc.victory.clear();
    let mut sim = Sim::with_options(&sc, Seed(1), bgws_run::default_figures(), SimOptions { flat_ground: true, ..Default::default() }).unwrap();
    for step in 1..=200 {
        sim.run_with_rules(SimTime::from_secs(step as f64 * 15.0));
        let mut line = format!("{} ", sim.now());
        for &e in &sim.ctx.order {
            let (i, h, a) = (sim.world.get::<Ident>(e).unwrap(), sim.world.get::<Health>(e).unwrap(), sim.world.get::<Act>(e).unwrap());
            if i.side == Side::Blue {
                line += &format!("{}:{}/{} {} | ", i.name, h.fit, h.start, a.activity.word());
            }
        }
        if step % 4 == 0 {
            println!("{line}");
        }
    }
}
