//! A shipped scenario on a real-ground pack (`bgws terrain`): the ground
//! comes from the pack, its roads are on the map, and moving vehicles keep to
//! them. Needs a pack built over Kaliningrad, e.g.
//! `bgws terrain /tmp/kgd --bbox 20.35,54.65,20.60,54.76`, then
//! `BGWS_TEST_PACK=/tmp/kgd cargo test -p bgws-run --test terrain_pack -- --ignored`.

use bgws_core::{Geodetic, SimTime};
use bgws_run::{load_scenario, Host, HostOptions};
use bgws_sim::components::{Ident, Kin};
use std::path::PathBuf;

#[test]
#[ignore]
fn units_move_on_the_packs_roads() {
    let pack = PathBuf::from(std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap();
    // Move it from the Baltic coast onto the pack, south-west of the city,
    // with an origin that is not the pack's, so the ground is re-framed.
    let (dlat, dlon) = (54.672 - sc.theatre.origin.lat, 20.43 - sc.theatre.origin.lon);
    let shift = |g: Geodetic| Geodetic::new(g.lat + dlat, g.lon + dlon, g.h);
    sc.theatre.origin = shift(sc.theatre.origin);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack.display().to_string());
    for f in &mut sc.forces {
        f.position = shift(f.position);
    }
    let mut host = Host::new(&sc, sc.seed, bgws_run::default_figures(), &HostOptions::default()).unwrap();
    assert!(host.sim.ctx.ground.roads.is_some(), "the pack's roads are on the ground");
    let (mut moving, mut on_road) = (0usize, 0usize);
    // Ten-second steps, sampling every vehicle on the move after each.
    for step in 1..=120 {
        let over = host
            .play_until(SimTime::from_secs(step as f64 * 10.0), |s| {
                for &e in &s.ctx.order {
                    let (Some(i), Some(k)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e)) else { continue };
                    if i.platform.mover.kind == bgws_catalogue::MoverKind::Foot || k.vel.horizontal() < 1.0 {
                        continue;
                    }
                    moving += 1;
                    on_road += usize::from(s.ctx.ground.road_at(k.pos.e, k.pos.n).is_some());
                }
            })
            .unwrap();
        if over.is_some() {
            break;
        }
    }
    let share = on_road as f64 / moving.max(1) as f64;
    // How much of the ground is road at all: the share by chance.
    let r = host.sim.ctx.ground.roads.as_ref().unwrap();
    let chance = r.data.iter().filter(|&&v| v > 0).count() as f64 / r.data.len() as f64;
    println!("moving vehicle-steps: {moving}, on a road: {:.0}% (road covers {:.0}% of the ground)", share * 100.0, chance * 100.0);
    assert!(moving > 100, "vehicles moved");
    assert!(share > 2.0 * chance, "moving vehicles keep to the roads ({:.0}% against {:.0}% by chance)", share * 100.0, chance * 100.0);
}
