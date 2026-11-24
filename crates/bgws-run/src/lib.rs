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

/// The scenario with the two sides' places exchanged on the same ground: every
/// position (forces, objectives, obstacles) is reflected through the point
/// midway between blue's and red's forces, so blue starts where red did and red
/// where blue did, each layout turned round; bearings and each force's facing
/// turn by 180°. The ground is the same map (generated ground and packs depend
/// only on where a point is). Played against the original on the same ground,
/// it separates the advantage of a place from the advantage of a side. Play
/// both with `SimOptions::ground_half_m` set to the larger of their
/// `bgws_sim::ground_half_extent`s: generated ground's grid scales with its
/// extent, so the two are on one map only on one extent.
pub fn sides_swapped(sc: &bgws_scenario::v4::ScenarioV4) -> anyhow::Result<bgws_scenario::v4::ScenarioV4> {
    use serde_json::Value;
    let mut v = serde_json::to_value(sc)?;
    let centroid = |side: &str| -> Option<(f64, f64)> {
        let pts: Vec<(f64, f64)> = v["forces"].as_array()?.iter().filter(|f| f["side"] == side).filter_map(|f| Some((f["position"]["lat"].as_f64()?, f["position"]["lon"].as_f64()?))).collect();
        (!pts.is_empty()).then(|| (pts.iter().map(|p| p.0).sum::<f64>() / pts.len() as f64, pts.iter().map(|p| p.1).sum::<f64>() / pts.len() as f64))
    };
    let (b, r) = (centroid("blue").ok_or_else(|| anyhow::anyhow!("no blue forces"))?, centroid("red").ok_or_else(|| anyhow::anyhow!("no red forces"))?);
    let (clat, clon) = ((b.0 + r.0) / 2.0, (b.1 + r.1) / 2.0);
    fn walk(v: &mut Value, clat: f64, clon: f64) {
        match v {
            Value::Object(m) => {
                if let (Some(la), Some(lo)) = (m.get("lat").and_then(Value::as_f64), m.get("lon").and_then(Value::as_f64)) {
                    m.insert("lat".into(), Value::from(2.0 * clat - la));
                    m.insert("lon".into(), Value::from(2.0 * clon - lo));
                }
                for (k, x) in m.iter_mut() {
                    if k == "bearingDeg" {
                        if let Some(d) = x.as_f64() {
                            *x = Value::from((d + 180.0).rem_euclid(360.0));
                        }
                    } else {
                        walk(x, clat, clon);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, clat, clon)),
            _ => {}
        }
    }
    for (k, x) in v.as_object_mut().ok_or_else(|| anyhow::anyhow!("a scenario is an object"))?.iter_mut() {
        if k != "theatre" {
            walk(x, clat, clon);
        }
    }
    // Each force faces the other way: its own heading, else its side's default (as the sim sets it), turned round.
    for f in v["forces"].as_array_mut().into_iter().flatten() {
        let default = match f["side"].as_str() {
            Some("blue") => 0.0,
            Some("red") => 180.0,
            _ => 90.0,
        };
        let h = f.get("headingDeg").and_then(Value::as_f64).unwrap_or(default);
        f["headingDeg"] = Value::from((h + 180.0).rem_euclid(360.0));
    }
    Ok(bgws_scenario::v4::ScenarioV4::from_value(v)?)
}

/// How a battle splits into the advantage of a place and of a side, per ground:
/// with g = blue − red strength left, the original gives g = side + place
/// (place: what blue's starting ground is worth), the swapped game gives
/// g = side − place. So side = (g + g')/2 and place = (g − g')/2. Each entry
/// is (ground seed, the original's batch, the swapped batch), same game seeds.
pub fn side_swap_markdown(rows: &[(String, bgws_analysis::batch::Batch<bgws_analysis::report::RunOutcome>, bgws_analysis::batch::Batch<bgws_analysis::report::RunOutcome>)]) -> String {
    use bgws_core::Side;
    let gap = |b: &bgws_analysis::batch::Batch<bgws_analysis::report::RunOutcome>| {
        let v: Vec<f64> = b.replicates.iter().filter_map(|r| Some(r.output.strength.get(&Side::Blue)? - r.output.strength.get(&Side::Red)?)).collect();
        if v.is_empty() { f64::NAN } else { v.iter().sum::<f64>() / v.len() as f64 }
    };
    let mut md = String::from("\n## Sides swapped on the same ground\n\n| Ground | Blue − red | Swapped | Side advantage (blue) | Place advantage (blue's start) |\n|---|---|---|---|---|\n");
    let (mut sides, mut places) = (Vec::new(), Vec::new());
    for (g, a, b) in rows {
        let (ga, gb) = (gap(a), gap(b));
        let (side, place) = ((ga + gb) / 2.0, (ga - gb) / 2.0);
        md.push_str(&format!("| {g} | {ga:+.3} | {gb:+.3} | {side:+.3} | {place:+.3} |\n"));
        sides.push(side);
        places.push(place);
    }
    let stats = |v: &[f64]| {
        let k = v.len() as f64;
        let m = v.iter().sum::<f64>() / k;
        let sd = (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (k - 1.0).max(1.0)).sqrt();
        let mag = v.iter().map(|x| x.abs()).sum::<f64>() / k;
        (m, sd / k.sqrt(), mag)
    };
    let (sm, sse, smag) = stats(&sides);
    let (pm, pse, pmag) = stats(&places);
    md.push_str(&format!(
        "\nSide advantage (blue): mean {sm:+.3} ± {sse:.3}, typical size {smag:.3}. Place advantage (blue's start): mean {pm:+.3} ± {pse:.3}, typical size {pmag:.3}, over {} grounds. ± is one standard error over grounds; typical size is the mean absolute value on one ground.\n",
        rows.len()
    ));
    md
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
