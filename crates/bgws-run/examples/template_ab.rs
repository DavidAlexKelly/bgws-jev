//! A doctrine A/B: a template patch on one side only, against the doctrine
//! everywhere, paired by scenario, seed and side (the rules decide for every
//! unit, the heuristic planner plans).
//!
//! `cargo run --release -p bgws-run --example template_ab -- <patch.json> [seeds] [scenario,...]`
//!
//! The patch is a JSON object of template nodes (`{"D10": {...}, ...}`). The
//! side under test has each of its units' templates replaced by a child of it
//! carrying those nodes, so everything else the unit does is unchanged.
use bgws_core::{Seed, Side, SimTime};
use bgws_run::{default_figures, DeciderChoice, Host, HostOptions};
use rayon::prelude::*;
use serde_json::{json, Value};

/// Result (+1 win, -1 loss) and exchange (enemy share lost - own share lost) for `side`.
fn play(sc: &bgws_scenario::v4::ScenarioV4, seed: u64, minutes: f64, side: Side) -> (f64, f64) {
    let mut h = Host::new(sc, Seed(seed), default_figures(), &HostOptions { deciders: DeciderChoice::Rules, ..Default::default() }).unwrap();
    let (own0, enemy0) = (h.sim.side_fit(side), h.sim.side_fit(side.enemy()));
    let over = h.play_until(SimTime::from_secs(minutes * 60.0), |_| {}).unwrap();
    let frac = |a: u32, b: u32| if a > 0 { a.saturating_sub(b) as f64 / a as f64 } else { 0.0 };
    let exchange = frac(enemy0, h.sim.side_fit(side.enemy())) - frac(own0, h.sim.side_fit(side));
    let result = match over.and_then(|o| o.winner) {
        Some(w) if w == side => 1.0,
        Some(_) => -1.0,
        None => 0.0,
    };
    (result, exchange)
}

/// The scenario with `side`'s forces on children of their templates carrying `patch`.
fn patched(sc: &bgws_scenario::v4::ScenarioV4, side: Side, patch: &Value) -> bgws_scenario::v4::ScenarioV4 {
    let sim = bgws_sim::Sim::new(sc, Seed(1), default_figures()).unwrap();
    let mut out = sc.clone();
    let mut made = std::collections::BTreeSet::new();
    for &e in &sim.ctx.order {
        let i = sim.world.get::<bgws_sim::components::Ident>(e).unwrap();
        if i.side != side {
            continue;
        }
        let Some(f) = sc.forces.iter().find(|f| f.id == i.name || f.label == i.name) else { continue };
        let base = sc.behaviours.assign.get(&f.id).cloned().unwrap_or_else(|| i.platform.template.clone());
        let child = format!("custom/ab-{}", base.replace('/', "-"));
        if made.insert(child.clone()) {
            out.behaviours.templates.push(json!({ "id": child, "version": 1, "parent": base, "nodes": patch }));
        }
        out.behaviours.assign.insert(f.id.clone(), child);
    }
    if std::env::var("AB_DEBUG").is_ok() {
        eprintln!("patched {} forces onto {:?}", out.behaviours.assign.len(), made);
        if let Ok(p) = std::env::var("AB_DUMP") {
            std::fs::write(format!("{p}-{side:?}.json"), serde_json::to_vec_pretty(&out).unwrap()).unwrap();
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let patch: Value = serde_json::from_slice(&std::fs::read(&args[1]).unwrap()).unwrap();
    let seeds: Vec<u64> = match args.get(2).map(|s| s.split_once('-')) {
        Some(Some((a, b))) => (a.parse().unwrap()..=b.parse().unwrap()).collect(),
        _ => (1..=4).collect(),
    };
    let entries: Vec<bgws_run::suite::Entry> = match args.get(3) {
        Some(list) => list.split(',').map(|s| bgws_run::suite::Entry { scenario: s.into(), minutes: None, note: String::new(), held_out: false }).collect(),
        None => bgws_run::suite::load_entries().unwrap(),
    };
    let jobs: Vec<(usize, u64, Side)> = (0..entries.len()).flat_map(|i| seeds.iter().flat_map(move |&k| [Side::Blue, Side::Red].map(|s| (i, k, s)))).collect();
    let scenarios: Vec<_> = entries.iter().map(|e| bgws_run::load_scenario(&bgws_run::scenario_file(&e.scenario)).unwrap()).collect();
    let rows: Vec<(usize, f64, f64)> = jobs
        .par_iter()
        .map(|&(i, k, s)| {
            let sc = &scenarios[i];
            let m = entries[i].minutes.unwrap_or(sc.scope.duration_min);
            let base = play(sc, k, m, s);
            let test = play(&patched(sc, s, &patch), k, m, s);
            (i, test.0 - base.0, test.1 - base.1)
        })
        .collect();
    let stat = |v: &[f64]| {
        let n = v.len().max(1) as f64;
        let mean = v.iter().sum::<f64>() / n;
        let var = if v.len() > 1 { v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0) } else { 0.0 };
        format!("{mean:+.2}±{:.2}", (var / n).sqrt())
    };
    let line = |name: &str, r: &[&(usize, f64, f64)]| {
        let res: Vec<f64> = r.iter().map(|x| x.1).collect();
        let exch: Vec<f64> = r.iter().map(|x| x.2).collect();
        let better = r.iter().filter(|x| x.1 > 0.0 || (x.1 == 0.0 && x.2 > 0.005)).count();
        let worse = r.iter().filter(|x| x.1 < 0.0 || (x.1 == 0.0 && x.2 < -0.005)).count();
        println!("| {name} | {} | {better} / {worse} | {} | {} |", r.len(), stat(&res), stat(&exch));
    };
    println!("| scenario | pairs | better / worse | result Δ | exchange Δ |\n|---|---|---|---|---|");
    for (i, e) in entries.iter().enumerate() {
        line(&e.scenario, &rows.iter().filter(|r| r.0 == i).collect::<Vec<_>>());
    }
    line("all", &rows.iter().collect::<Vec<_>>());
}
