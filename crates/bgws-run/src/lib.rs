//! The host side of the engine (§3.3): the loop that advances a `Sim`,
//! gathers its questions, sends them to the deciders (Jev, Claude, a local
//! model, a person or the rules) and hands the answers back; the game
//! record; and the experiments that run the simulation for the analysis
//! crate (batches, vignettes, sensitivity).

pub mod aar;
pub mod behaviour;
pub mod cases;
pub mod correlate;
pub mod editor;
pub mod eventlog;
pub mod experiments;
pub mod geotiff;
pub mod host;
pub mod lab;
pub mod federate;
pub mod foundry;
pub mod partner;
pub mod scengen;
pub mod suite;
pub mod tune;
pub mod workbench;

pub use host::{load_scenario, router_for, DeciderChoice, DisLinkAdapter, HlaLinkAdapter, Host, HostOptions, Link, OnTick, PlayReport};

/// The shipped data (scenarios, catalogue, templates, cases): `$BGWS_DATA`, else `data/` at the repository root.
pub fn data_dir() -> std::path::PathBuf {
    std::env::var_os("BGWS_DATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data"))
}

/// Whether a scenario plays on generated ground (not flat, not a theatre pack),
/// so that another ground seed gives it other ground.
pub fn on_generated_ground(sc: &bgws_scenario::v4::ScenarioV4) -> bool {
    sc.theatre.pack.is_none() && sc.theatre.ground != "flat"
}

/// The scenario on its `k`-th generated ground: the same forces, orders and
/// weather on other hills, woods and towns. Ground 0 is the scenario's own;
/// ground k > 0 has the ground seed `<its seed>~g<k>`. A scenario on flat
/// ground or a theatre pack is returned as it is. Results that hold on one
/// ground can reverse on another (a 3-against-3 tank duel did), so a
/// comparison should be played over several.
pub fn on_ground(sc: &bgws_scenario::v4::ScenarioV4, k: u64) -> bgws_scenario::v4::ScenarioV4 {
    let mut out = sc.clone();
    if k > 0 && on_generated_ground(sc) {
        let base = if sc.theatre.ground_seed.is_empty() { sc.name.clone() } else { sc.theatre.ground_seed.clone() };
        out.theatre.ground_seed = format!("{base}~g{k}");
    }
    out
}

/// How a batch's result varies with the ground: per ground, the games, blue's
/// and red's mean strength left and the share blue won; then the spread of
/// the blue − red gap across the grounds and how many grounds each side came
/// out ahead on. Each entry is (ground seed, that ground's batch).
pub fn ground_spread_markdown(by_ground: &[(String, bgws_analysis::batch::Batch<bgws_analysis::report::RunOutcome>)]) -> String {
    use bgws_core::Side;
    let mean = |b: &bgws_analysis::batch::Batch<bgws_analysis::report::RunOutcome>, side: Side| {
        let v: Vec<f64> = b.replicates.iter().filter_map(|r| r.output.strength.get(&side).copied()).collect();
        if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    let mut md = String::from("\n## By ground\n\n| Ground | Games | Blue left | Red left | Blue − red | Blue won |\n|---|---|---|---|---|---|\n");
    let mut gaps = Vec::new();
    for (g, b) in by_ground {
        let (bl, rd) = (mean(b, Side::Blue), mean(b, Side::Red));
        let won = b.replicates.iter().filter(|r| r.output.over.as_deref().is_some_and(|o| o.starts_with("blue wins"))).count();
        md.push_str(&format!("| {g} | {} | {bl:.3} | {rd:.3} | {:+.3} | {won} |\n", b.replicates.len(), bl - rd));
        gaps.push(bl - rd);
    }
    let k = gaps.len() as f64;
    let m = gaps.iter().sum::<f64>() / k;
    let sd = (gaps.iter().map(|g| (g - m).powi(2)).sum::<f64>() / (k - 1.0).max(1.0)).sqrt();
    let (lo, hi) = gaps.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &g| (a.min(g), b.max(g)));
    let ahead = gaps.iter().filter(|g| **g > 0.0).count();
    md.push_str(&format!(
        "\nBlue − red strength left over {} grounds: mean {m:+.3} ± {:.3} (standard error over grounds, the uncertainty that counts when the ground varies), SD between grounds {sd:.3}, range {lo:+.3} to {hi:+.3}. Blue ahead on {ahead}, red on {}.{}\n",
        gaps.len(),
        sd / k.sqrt(),
        gaps.iter().filter(|g| **g < 0.0).count(),
        if ahead > 0 && ahead < gaps.len() { " **The side ahead depends on the ground.**" } else { "" }
    ));
    md
}

/// A scenario by path, or by the name of a shipped one (`data/scenarios/<name>.json`).
pub fn scenario_file(s: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(s);
    if p.exists() {
        return p;
    }
    let named = data_dir().join("scenarios").join(if s.ends_with(".json") { s.to_string() } else { format!("{s}.json") });
    if named.exists() {
        named
    } else {
        p
    }
}

/// Jev's wording (`$BGWS_JEV_WORDING`, else `data/jev/wording.json`, from `bgws lab tune --accept`),
/// else the engine's own. A file that does not read is reported and ignored.
pub fn jev_wording() -> bgws_decide::jev::Wording {
    // `BGWS_JEV_WORDING`: another wording file, to compare one without installing it.
    let p = std::env::var_os("BGWS_JEV_WORDING").map(std::path::PathBuf::from).unwrap_or_else(|| data_dir().join("jev").join("wording.json"));
    match std::fs::read(&p) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            eprintln!("{}: {e}; Jev keeps its own wording", p.display());
            Default::default()
        }),
        Err(_) => Default::default(),
    }
}

/// Where theatre packs live: `$BGWS_PACKS`, else `packs/` at the repository root.
pub fn packs_dir() -> std::path::PathBuf {
    std::env::var_os("BGWS_PACKS").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs"))
}

/// A scenario's `theatre.pack`: a path, or a folder name under `packs_dir()`,
/// else one of the small fixture packs committed under `data/packs/` (which
/// CI, with no built packs, plays the real-ground vignettes on).
pub fn resolve_pack(name: &str) -> std::path::PathBuf {
    let p = std::path::PathBuf::from(name);
    if p.is_absolute() || p.exists() {
        return p;
    }
    let built = packs_dir().join(name);
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/packs").join(name);
    if !built.exists() && fixture.exists() {
        fixture
    } else {
        built
    }
}

/// Every figure the whole engine declares: the sim's crates plus the
/// deciders and the analysis settings.
pub fn all_figures() -> Vec<bgws_core::Figure> {
    let mut v = bgws_sim::all_figures();
    v.extend(bgws_decide::figures());
    v.extend(bgws_analysis::figures());
    v
}

/// The default registry for a host.
pub fn default_figures() -> bgws_core::Figures {
    bgws_core::Figures::from_list(all_figures()).expect("the shipped figures are consistent")
}

/// One side of a scenario, for a federation where each simulation owns a
/// side: only that side's forces and HQs (HQs keep only their own
/// subordinates). Victory keeps only the objectives, which give each side
/// its mission; a strength breakpoint would end the game at once, since a
/// simulation counts only its own side's strength.
pub fn one_side(sc: &bgws_scenario::v4::ScenarioV4, side: bgws_core::Side) -> anyhow::Result<bgws_scenario::v4::ScenarioV4> {
    let side = serde_json::to_value(side)?;
    let mut v = serde_json::to_value(sc)?;
    let forces = v["forces"].as_array().cloned().unwrap_or_default();
    let gone: Vec<String> = forces.iter().filter(|f| f["side"] != side).filter_map(|f| f["id"].as_str().map(String::from)).collect();
    if forces.len() == gone.len() {
        anyhow::bail!("the scenario has no {side} forces");
    }
    if let Some(f) = v["forces"].as_array_mut() {
        f.retain(|f| f["side"] == side);
    }
    if let Some(hqs) = v.get_mut("command").and_then(|c| c.get_mut("hqs")).and_then(|h| h.as_array_mut()) {
        hqs.retain(|h| h["side"] == side);
        for h in hqs.iter_mut() {
            if let Some(subs) = h.get_mut("subordinates").and_then(|x| x.as_array_mut()) {
                subs.retain(|x| !gone.iter().any(|g| x == g.as_str()));
            }
        }
    }
    if let Some(vc) = v.get_mut("victory").and_then(|x| x.as_array_mut()) {
        vc.retain(|c| c["kind"] == "objective");
    }
    bgws_scenario::v4::ScenarioV4::from_json(&v.to_string()).map_err(|e| anyhow::anyhow!("{e}"))
}
