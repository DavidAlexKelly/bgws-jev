//! Scratch: one game with fire and movement on one side's planner, one without.
use bgws_core::{Seed, Side, SimTime};
use bgws_run::{default_figures, DeciderChoice, Host, HostOptions};
use bgws_sim::components::{Act, Activity, Health, Ident, Kin};
use std::sync::Arc;
fn main() {
    let name = std::env::args().nth(1).unwrap();
    let seed: u64 = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(1);
    let side = if std::env::args().nth(3).as_deref() == Some("red") { Side::Red } else { Side::Blue };
    let trace = std::env::var("TRACE").ok();
    let sc = bgws_run::load_scenario(&bgws_run::scenario_file(&name)).unwrap();
    let f = default_figures();
    let mut old = f.clone();
    old.set("decide.heuristic.fire_and_movement", 0.0).unwrap();
    for (label, figs) in [("F&M", &f), ("plain", &old)] {
        let mut h = Host::new(&sc, Seed(seed), f.clone(), &HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }).unwrap();
        for s in [Side::Blue, Side::Red] {
            let p: Arc<dyn bgws_decide::Planner> = Arc::new(bgws_decide::HeuristicPlanner::from_figures(if s == side { figs } else { &old }));
            h.router.set_planner(s, bgws_decide::Route::Rules, p);
        }
        let mut over = None;
        for minute in 1..=sc.scope.duration_min as u64 {
            over = h.play_until(SimTime::from_secs(minute as f64 * 60.0), |_| {}).unwrap();
            let sim = &h.sim;
            for &e in &sim.ctx.order {
                let (i, hh, a, k) = (sim.world.get::<Ident>(e).unwrap(), sim.world.get::<Health>(e).unwrap(), sim.world.get::<Act>(e).unwrap(), sim.world.get::<Kin>(e).unwrap());
                if i.side != side {
                    continue;
                }
                if let Some(who) = &trace {
                    if label == "F&M" && i.name.ends_with(who.as_str()) && minute <= 40 {
                        let (n, first, last) = match &a.activity {
                            Activity::Move { route, .. } => (route.len(), route.first().map(|p| (p.e as i64, p.n as i64)), route.last().map(|p| (p.e as i64, p.n as i64))),
                            _ => (0, None, None),
                        };
                        println!("    {minute:>3}m {} {}/{} at ({:.0},{:.0}) phase {}/{} {} route {n} next {first:?} last {last:?}", i.name, hh.fit, hh.start, k.pos.e, k.pos.n, a.mission.phase, a.mission.phases.len(), a.activity.word());
                    }
                }
            }
            if minute % 15 == 0 && trace.is_none() {
                let mut line = format!("  {minute:>3}m");
                for &e in &sim.ctx.order {
                    let (i, hh, a, k) = (sim.world.get::<Ident>(e).unwrap(), sim.world.get::<Health>(e).unwrap(), sim.world.get::<Act>(e).unwrap(), sim.world.get::<Kin>(e).unwrap());
                    if i.side == side && !i.name.contains("HQ") {
                        line += &format!(" {}:{}/{} {} ({:.0},{:.0})", i.name.split_whitespace().last().unwrap_or(""), hh.fit, hh.start, a.activity.word(), k.pos.e, k.pos.n);
                    }
                }
                println!("{line}");
            }
            if over.is_some() {
                break;
            }
        }
        println!("{label}: {:?}", over.map(|o| (o.at.secs() / 60.0, o.reason)));
    }
}
