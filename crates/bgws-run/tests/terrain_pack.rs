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

#[test]
#[ignore]
/// Plans on real ground reach their destination, or as near as they can get: never a straight line through what the vehicle cannot cross.
fn plans_on_real_ground_never_fall_back_to_a_straight_line() {
    let pack = std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap();
    let (dlat, dlon) = (54.672 - sc.theatre.origin.lat, 20.43 - sc.theatre.origin.lon);
    let shift = |g: Geodetic| Geodetic::new(g.lat + dlat, g.lon + dlon, g.h);
    sc.theatre.origin = shift(sc.theatre.origin);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack);
    for f in &mut sc.forces { f.position = shift(f.position); }
    let mut host = Host::new(&sc, sc.seed, bgws_run::default_figures(), &HostOptions::default()).unwrap();
    let s = host.sim.ctx.ground.terrain.spec;
    let mut rng = 12345u64;
    let mut next = || { rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (rng >> 33) as f64 / (1u64 << 31) as f64 };
    for rivers in [true, false] {
        if !rivers { host.sim.ctx.ground.rivers = None; }
        let gg = &host.sim.ctx.ground;
        let (mut fall, mut total) = (0, 0);
        let (mut road, mut open) = (0.0, 0.0);
        for _ in 0..60 {
            let a = bgws_core::Enu::new(s.origin_e + next() * s.cell * s.width as f64, s.origin_n + next() * s.cell * s.height as f64, 0.0);
            let b = bgws_core::Enu::new(a.e + (next() - 0.5) * 6000.0, a.n + (next() - 0.5) * 6000.0, 0.0);
            if !gg.contains(b) || gg.land_cover(a.e, a.n) == bgws_env::LandCover::Water || gg.land_cover(b.e, b.n) == bgws_env::LandCover::Water { continue; }
            let r = bgws_sim::routing::RouteRequest { from: a, to: b, class: bgws_env::VehicleClass::Tracked, mode: bgws_sim::components::MoveModeS::Tactical, road_kmh: 80.0, cross_kmh: 40.0, threats: &[], obstacles: &[], urgent: false };
            let p = gg.plan(&r, &gg.route_params);
            road += p.road_m;
            open += p.open_m;
            total += 1;
            if p.waypoints.len() == 1 && a.sub(b).horizontal() > 500.0 {
                fall += 1;
            }
        }
        println!("rivers {rivers}: {fall} of {total} plans fell back to a straight line; road share {:.2}", road / (road + open));
        if rivers {
            assert_eq!(fall, 0, "plans fell back to a straight line");
            assert!(total > 20);
        }
    }
}

#[test]
#[ignore = "needs a theatre pack: BGWS_TEST_PACK=<dir> cargo test --release -p bgws-run --test terrain_pack -- --ignored"]
/// Long moves by the road graph take about as long as the full grid search
/// finds, in a fraction of the planning time.
fn long_moves_by_the_road_graph_are_as_good_as_the_grid() {
    let pack = std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap();
    let (dlat, dlon) = (54.672 - sc.theatre.origin.lat, 20.43 - sc.theatre.origin.lon);
    let shift = |g: Geodetic| Geodetic::new(g.lat + dlat, g.lon + dlon, g.h);
    sc.theatre.origin = shift(sc.theatre.origin);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack);
    for f in &mut sc.forces { f.position = shift(f.position); }
    let host = Host::new(&sc, sc.seed, bgws_run::default_figures(), &HostOptions::default()).unwrap();
    let gg = &host.sim.ctx.ground;
    assert!(gg.road_net.is_some(), "the pack's road graph");
    let s = gg.terrain.spec;
    let mut graph_p = gg.route_params.clone();
    graph_p.long_m = 8_000.0;
    let mut grid_p = gg.route_params.clone();
    grid_p.long_m = f64::INFINITY;
    let mut rng = 777u64;
    let mut next = || { rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (rng >> 33) as f64 / (1u64 << 31) as f64 };
    let (mut t_graph, mut t_grid, mut ms_graph, mut ms_grid, mut n) = (0.0, 0.0, 0.0, 0.0, 0);
    while n < 15 {
        let a = bgws_core::Enu::new(s.origin_e + next() * s.cell * s.width as f64, s.origin_n + next() * s.cell * s.height as f64, 0.0);
        let b = bgws_core::Enu::new(s.origin_e + next() * s.cell * s.width as f64, s.origin_n + next() * s.cell * s.height as f64, 0.0);
        if a.sub(b).horizontal() < 10_000.0 || gg.land_cover(a.e, a.n) == bgws_env::LandCover::Water || gg.land_cover(b.e, b.n) == bgws_env::LandCover::Water { continue; }
        let r = bgws_sim::routing::RouteRequest { from: a, to: b, class: bgws_env::VehicleClass::Wheeled, mode: bgws_sim::components::MoveModeS::March, road_kmh: 80.0, cross_kmh: 30.0, threats: &[], obstacles: &[], urgent: false };
        let t0 = std::time::Instant::now();
        let pg = gg.plan(&r, &graph_p);
        let t1 = std::time::Instant::now();
        let pd = gg.plan(&r, &grid_p);
        let t2 = std::time::Instant::now();
        t_graph += pg.time_s;
        t_grid += pd.time_s;
        ms_graph += (t1 - t0).as_secs_f64() * 1e3;
        ms_grid += (t2 - t1).as_secs_f64() * 1e3;
        n += 1;
    }
    println!("{n} moves: by graph {:.0} min ({:.0} ms each), on the grid {:.0} min ({:.0} ms each)", t_graph / 60.0, ms_graph / n as f64, t_grid / 60.0, ms_grid / n as f64);
    assert!(t_graph < t_grid * 1.25, "the graph's routes are not much slower to drive");
}

#[test]
#[ignore = "needs a theatre pack: BGWS_TEST_PACK=<dir> cargo test --release -p bgws-run --test terrain_pack -- --ignored"]
/// A scenario that names a pack but was never moved onto it says so, and how to fix it.
fn a_scenario_off_its_pack_is_told_how_to_fix_it() {
    let pack = std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/m4-drone-line.json")).unwrap();
    sc.theatre.pack = Some(pack);
    let e = bgws_run::host::load_pack(&sc).map(|_| ()).expect_err("an error").to_string();
    assert!(e.contains("none of the scenario's forces are on it") && e.contains("Move onto the pack"), "{e}");
}

/// The city's blocks come out of the pack's buildings, split by its
/// streets: many of them, of every material, and a heavy shell brings part
/// of one down and lowers it for sight lines. Needs a pack with a sight
/// surface (built since buildings were added: `surface.bin`).
#[test]
#[ignore = "needs a real pack: BGWS_TEST_PACK"]
fn the_city_is_cut_into_blocks() {
    let pack = PathBuf::from(std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap();
    // Onto the city centre.
    let (dlat, dlon) = (54.712 - sc.theatre.origin.lat, 20.51 - sc.theatre.origin.lon);
    let shift = |g: Geodetic| Geodetic::new(g.lat + dlat, g.lon + dlon, g.h);
    sc.theatre.origin = shift(sc.theatre.origin);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack.display().to_string());
    for f in &mut sc.forces {
        f.position = shift(f.position);
    }
    let t0 = std::time::Instant::now();
    let mut host = Host::new(&sc, sc.seed, bgws_run::default_figures(), &HostOptions::default()).unwrap();
    let g = &host.sim.ctx.ground;
    let u = g.urban.as_ref().expect("the pack's town is in blocks");
    let n = u.blocks.len();
    let mut by = [0usize; 3];
    for b in &u.blocks {
        by[b.material as usize] += 1;
    }
    let big: Vec<_> = u.blocks.iter().filter(|b| b.area_m2 >= 5_000.0).collect();
    let largest = u.blocks.iter().map(|b| b.area_m2).fold(0.0, f64::max);
    eprintln!("{n} blocks (light {}, masonry {}, concrete {}), {} over 5,000 m², largest {:.0} m², loaded in {:.1} s", by[0], by[1], by[2], big.len(), largest, t0.elapsed().as_secs_f64());
    assert!(n > 200, "a city has hundreds of blocks");
    assert!(by.iter().all(|&c| c > 0), "light, masonry and concrete all occur");
    assert!(largest < 2.0e6, "streets cut the city up: no block is the whole town");
    // A built-over block near the centre: shell it.
    let b = big.iter().min_by(|a, b| a.e.hypot(a.n).total_cmp(&b.e.hypot(b.n))).unwrap();
    let (id, e, n0) = (b.id, b.e, b.n);
    let cell = host.sim.ctx.ground.urban.as_ref().unwrap().cells[id as usize][0] as usize;
    let spec = host.sim.ctx.ground.terrain.spec;
    let (ce, cn) = spec.pos(cell % spec.width, cell / spec.width);
    let sight = host.sim.ctx.ground.sight.clone().unwrap();
    // The tallest roof sample in that cell, before and after.
    let tallest = |v: &bgws_env::SightView| (0..36).map(|k| v.at(ce - 15.0 + (k % 6) as f64 * 5.0 + 2.5, cn - 15.0 + (k / 6) as f64 * 5.0 + 2.5).0).fold(0.0, f64::max);
    let before = tallest(&sight);
    let mut left = 1.0;
    for _ in 0..40 {
        left = host.sim.ctx.ground.strike_building(ce, cn).map(|x| x.1).unwrap_or(left);
    }
    let after = tallest(host.sim.ctx.ground.sight.as_ref().unwrap());
    eprintln!("block {id} at ({e:.0}, {n0:.0}): {:.0}% standing after 40 rounds; roofs {before:.0} m → {after:.1} m", left * 100.0);
    assert!(left < 1.0 && after < before, "shelling brings it down");
}

/// The terrain appreciation: the pack has one (worked out on first opening
/// if need be), the city is dead ground beside open farmland, and HQs are
/// briefed on key terrain they can name in orders.
#[test]
#[ignore = "needs a real pack: BGWS_TEST_PACK"]
fn the_pack_has_a_terrain_appreciation() {
    let pack = PathBuf::from(std::env::var("BGWS_TEST_PACK").expect("BGWS_TEST_PACK: a pack over Kaliningrad"));
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut sc = load_scenario(&root.join("data/scenarios/skirmish-advance-to-contact-v1.json")).unwrap();
    let (dlat, dlon) = (54.672 - sc.theatre.origin.lat, 20.43 - sc.theatre.origin.lon);
    let shift = |g: Geodetic| Geodetic::new(g.lat + dlat, g.lon + dlon, g.h);
    sc.theatre.origin = shift(sc.theatre.origin);
    sc.theatre.bounds = None;
    sc.theatre.pack = Some(pack.display().to_string());
    for f in &mut sc.forces {
        f.position = shift(f.position);
    }
    let host = Host::new(&sc, sc.seed, bgws_run::default_figures(), &HostOptions::default()).unwrap();
    let g = &host.sim.ctx.ground;
    let a = g.appreciation.as_ref().expect("the pack has a terrain appreciation");
    let count = |k: bgws_env::KeyKind| a.key.iter().filter(|x| x.kind == k).count();
    eprintln!("observation {}, chokepoints {}, crossings {}", count(bgws_env::KeyKind::Observation), count(bgws_env::KeyKind::Chokepoint), count(bgws_env::KeyKind::Crossing));
    assert!(count(bgws_env::KeyKind::Crossing) > 5, "Kaliningrad's rivers have bridges");
    assert!(count(bgws_env::KeyKind::Observation) > 3);
    // The whole scenario area: exposure is a share, and both dead and open ground occur.
    let s = g.terrain.spec;
    let xs: Vec<f64> = (0..s.len()).step_by(37).filter_map(|k| g.exposure(s.pos(k % s.width, k / s.width).0, s.pos(k % s.width, k / s.width).1)).collect();
    let dead = xs.iter().filter(|&&x| x < 0.1).count() as f64 / xs.len() as f64;
    let open = xs.iter().filter(|&&x| x >= 0.4).count() as f64 / xs.len() as f64;
    eprintln!("{} samples: dead ground {:.0}%, open to view {:.0}%", xs.len(), dead * 100.0, open * 100.0);
    assert!(dead > 0.05 && open > 0.05);
    // The HQ's brief names key terrain, usable as reference points.
    let sit = host.sim.hq_situation(bgws_core::Side::Blue, "test");
    eprintln!("{:#?}", sit.terrain);
    assert!(!sit.terrain.is_empty(), "the HQ is briefed on key terrain");
    assert!(sit.terrain.iter().all(|t| sit.reference_points.contains_key(&t.label)));
}
