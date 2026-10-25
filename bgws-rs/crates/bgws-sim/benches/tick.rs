//! Tick benchmarks (RUST_ENGINE.md §13, §17): the synthetic brigade of
//! 2,000 land entities, whole base ticks and the land systems one by one,
//! plus a quiet stretch with and without quiet skipping.
//!
//! Run with `cargo bench -p bgws-sim --bench tick`. Before the criterion
//! groups it prints a per-system breakdown at a few points of the fight, the
//! numbers the §13 budget (100 ms per simulated second at ×10) is read
//! against.

use bgws_core::{Seed, SimDuration};
use bgws_sim::domains::lod::synthetic::brigade;
use bgws_sim::{default_figures, Sim, Step};
use criterion::{criterion_group, Criterion};
use std::time::{Duration, Instant};

/// Advance to `until`, answering every question with the rules.
fn play(sim: &mut Sim, until: bgws_core::SimTime, quiet: bool) {
    let step = |sim: &mut Sim| if quiet { sim.advance_quiet(until) } else { sim.advance(until) };
    while let Step::NeedsAnswers { questions, .. } = step(sim) {
        let a = sim.rules_answers(&questions);
        sim.answer(a).expect("rules answers");
    }
}

/// One simulated second.
fn second(sim: &mut Sim) {
    let until = sim.now() + SimDuration::from_secs(1.0);
    play(sim, until, false);
}

/// The brigade: 2,000 platoons, the fronts 4 km apart, blue advancing.
fn brigade_sim() -> Sim {
    let sc = brigade(2_000, 20_000.0, 10_000.0, 4_000.0).build();
    Sim::new(&sc, Seed(1), default_figures()).expect("brigade")
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Whole ticks and each land system (run once more on the state each whole
/// tick left) at a few points of the fight.
fn breakdown() {
    let t0 = Instant::now();
    let mut sim = brigade_sim();
    println!("brigade setup: {:.0} ms ({} entities), {} rayon threads", ms(t0.elapsed()), sim.ctx.order.len(), rayon::current_num_threads());
    let t0 = Instant::now();
    second(&mut sim);
    println!("first second (HQ plans, 1,000 routes): {:.0} ms", ms(t0.elapsed()));
    let mut detections = 0;
    let mut shots = 0;
    for checkpoint in [60.0, 300.0, 600.0, 900.0] {
        let t0 = Instant::now();
        let from = sim.now();
        play(&mut sim, bgws_core::SimTime::from_secs(checkpoint), false);
        let run_s = (sim.now() - from).secs();
        let per_s = ms(t0.elapsed()) / run_s.max(1.0);
        let n = 5;
        let names = ["posture", "movement", "sensing", "reports", "direct_fire", "people", "autopilot", "victory"];
        let mut acc = [Duration::ZERO; 8];
        let mut whole = Duration::ZERO;
        for _ in 0..n {
            let t = Instant::now();
            second(&mut sim);
            whole += t.elapsed();
            let mut time = |i: usize, f: &mut dyn FnMut(&mut Sim)| {
                let t = Instant::now();
                f(&mut sim);
                acc[i] += t.elapsed();
            };
            time(0, &mut bgws_sim::systems::posture);
            time(1, &mut |s| bgws_sim::systems::movement(s, 1.0));
            time(2, &mut |s| bgws_sim::systems::sensing(s, 1.0));
            time(3, &mut bgws_sim::systems::reports);
            time(4, &mut bgws_sim::systems::direct_fire);
            time(5, &mut |s| bgws_sim::systems::people(s, 1.0));
            time(6, &mut bgws_sim::systems::autopilot_events);
            time(7, &mut bgws_sim::systems::victory);
        }
        let events = sim.drain_events();
        detections += events.iter().filter(|e| e.kind == "sense.detect").count();
        shots += events.iter().filter(|e| e.kind.starts_with("fire.")).count();
        let tick = ms(whole) / n as f64;
        println!("t = {}: {per_s:.1} ms per simulated second over the last {run_s:.0} s; whole tick now {tick:.1} ms (×{:.1} real time); detections {detections}, shots {shots} so far", sim.now(), 1000.0 / tick);
        let parts: Vec<String> = names.iter().zip(acc).map(|(n, d)| format!("{n} {:.2}", ms(d) / 5.0)).collect();
        println!("    ms: {}", parts.join(", "));
    }
}

fn bench_brigade(c: &mut Criterion) {
    let mut g = c.benchmark_group("brigade_2000");
    g.sample_size(10).measurement_time(Duration::from_secs(20));
    let mut sim = brigade_sim();
    play(&mut sim, bgws_core::SimTime::from_secs(300.0), false);
    g.bench_function("tick_1s", |b| b.iter(|| second(&mut sim)));
    g.bench_function("sensing", |b| b.iter(|| bgws_sim::systems::sensing(&mut sim, 1.0)));
    g.bench_function("movement", |b| b.iter(|| bgws_sim::systems::movement(&mut sim, 1.0)));
    g.bench_function("people", |b| b.iter(|| bgws_sim::systems::people(&mut sim, 1.0)));
    g.bench_function("direct_fire", |b| b.iter(|| bgws_sim::systems::direct_fire(&mut sim)));
    g.finish();
}

fn bench_quiet(c: &mut Criterion) {
    // The fronts 40 km apart, blue advancing: nothing can be seen for a long while.
    let sc = brigade(2_000, 20_000.0, 10_000.0, 40_000.0).build();
    let mut g = c.benchmark_group("quiet_2000");
    g.sample_size(10).measurement_time(Duration::from_secs(20));
    for quiet in [false, true] {
        let mut sim = Sim::new(&sc, Seed(1), default_figures()).expect("brigade");
        play(&mut sim, bgws_core::SimTime::from_secs(5.0), false);
        g.bench_function(if quiet { "advance_quiet_10s" } else { "advance_10s" }, |b| {
            b.iter(|| {
                let until = sim.now() + SimDuration::from_secs(10.0);
                play(&mut sim, until, quiet)
            })
        });
    }
    g.finish();
}

criterion_group!(benches, bench_brigade, bench_quiet);

fn main() {
    breakdown();
    benches();
    Criterion::default().configure_from_args().final_summary();
}
