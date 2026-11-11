//! Generated scenarios for testing: engagements built from a few settings,
//! so the suite can cover attacks, defences, meeting engagements, ambushes,
//! withdrawals, a river crossing and a fight into a town, at company and
//! battalion size, by day and night, on generated ground and on a real
//! theatre pack.
//!
//! Each scenario is built on M3's settings (domains, comms, the HQ tree) and
//! given an `objective` victory condition: the side that must take ground
//! wins by holding it for ten minutes with no enemy on it, and fails if it
//! has not by the time limit. Its HQs are told their mission (attack or
//! defend) and plan with whatever planner the side has.
//!
//! The standard set (`standard()`) is written to `data/scenarios/gen-*.json`
//! by `bgws scenarios generate`; the files are deterministic from their specs.

use anyhow::{anyhow, Result};
use bgws_core::{Enu, EnuFrame, Geodetic};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Blue attacks a prepared red position.
    Attack,
    /// Red attacks a blue position.
    Defence,
    /// Both advance on the same ground; whoever holds it wins.
    Meeting,
    /// Blue advances through an area red has laid an ambush in.
    Ambush,
    /// Blue, outnumbered, must get back to a rear position with red in pursuit.
    Withdrawal,
    /// Blue attacks across the widest river near the pack's centre (pack only).
    RiverCrossing,
    /// Blue attacks into the largest town on the pack (pack only).
    UrbanAttack,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Echelon {
    Company,
    Battalion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum GroundSpec {
    Flat,
    /// Generated ground (hills, woods, towns from a seed).
    Generated,
    /// A theatre pack, by name (`packs/<name>`).
    Pack { name: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spec {
    /// The file name (without .json): `gen-…`.
    pub name: String,
    pub kind: Kind,
    pub echelon: Echelon,
    /// The attacker's manoeuvre elements per defending element, roughly
    /// (attacking platoons are tanks and mechanised infantry; defending ones
    /// infantry and anti-tank teams).
    pub ratio: f64,
    /// Between the forces at the start, km.
    pub gap_km: f64,
    /// Local solar hour at the start (5 dawn, 12 day, 20 dusk, 1 night).
    pub hour: f64,
    pub weather: String,
    pub ground: GroundSpec,
    pub minutes: f64,
    pub seed: u64,
}

/// One element: (id, label, platform, count, east m, north m, concealed), in the scenario's frame before rotation.
type El = (String, String, &'static str, u32, f64, f64, bool);

struct Force {
    side: &'static str,
    elements: Vec<El>,
    hqs: Vec<Value>,
}

/// The attacker's manoeuvre platoons alternate tanks and mechanised infantry.
fn attackers(side: &'static str, pre: &str, n: usize, y: f64, front: f64) -> Vec<El> {
    (0..n)
        .map(|k| {
            let x = if n > 1 { (k as f64 / (n - 1) as f64 - 0.5) * front } else { 0.0 };
            if k % 2 == 0 {
                (format!("{pre}-{}", k + 1), format!("{} {} tanks", side_word(side), k + 1), "generic/mbt", 4, x, y, false)
            } else {
                (format!("{pre}-{}", k + 1), format!("{} {} mech", side_word(side), k + 1), "generic/ifv", 3, x, y, false)
            }
        })
        .collect()
}

fn side_word(side: &str) -> &'static str {
    if side == "blue" {
        "Blue"
    } else {
        "Red"
    }
}

/// A defence: infantry platoons across the front, anti-tank teams behind them.
fn defenders(side: &'static str, pre: &str, n: usize, y: f64, front: f64, concealed: bool) -> Vec<El> {
    (0..n)
        .map(|k| {
            let x = if n > 1 { (k as f64 / (n - 1) as f64 - 0.5) * front } else { 0.0 };
            if k % 2 == 0 {
                (format!("{pre}-{}", k + 1), format!("{} {} platoon", side_word(side), k + 1), "generic/infantrySection", 3, x, y, concealed)
            } else {
                (format!("{pre}-at-{}", k + 1), format!("{} ATGM {}", side_word(side), k + 1), "generic/atgmTeam", 2, x, y + 250.0, concealed)
            }
        })
        .collect()
}

/// HQs over a force's elements: a company HQ over all of them; or for a
/// battalion, a company HQ per three manoeuvre elements under a battalion HQ,
/// which also has the mortars.
fn command(side: &'static str, pre: &str, els: &mut Vec<El>, rear_y: f64, toward: f64, company: bool) -> Vec<Value> {
    let support: Vec<String> = els.iter().filter(|e| e.2 == "generic/mortar120").map(|e| e.0.clone()).collect();
    let fighting: Vec<String> = els.iter().filter(|e| e.2 != "generic/mortar120").map(|e| e.0.clone()).collect();
    let groups: Vec<Vec<String>> = if company { vec![els.iter().map(|e| e.0.clone()).collect()] } else { fighting.chunks(3).map(|c| c.to_vec()).collect() };
    let mut hqs = Vec::new();
    let back = |d: f64| rear_y - toward * d;
    if groups.len() == 1 {
        let post = format!("{pre}-hq");
        els.push((post.clone(), format!("{} company HQ", side_word(side)), "generic/cp-company", 2, 0.0, back(600.0), false));
        hqs.push(json!({ "id": format!("{side}-coy"), "side": side, "echelon": "company", "postId": post, "subordinates": groups[0], "planner": "land", "decider": "rules" }));
        return hqs;
    }
    let bn = format!("{pre}-bn");
    els.push((bn.clone(), format!("{} battalion HQ", side_word(side)), "generic/cp-battalion", 3, 0.0, back(1_500.0), false));
    let mut subs = Vec::new();
    for (k, g) in groups.iter().enumerate() {
        let id = format!("{side}-coy{}", k + 1);
        let post = format!("{pre}-hq{}", k + 1);
        let x = els.iter().filter(|e| g.contains(&e.0)).map(|e| e.4).sum::<f64>() / g.len() as f64;
        els.push((post.clone(), format!("{} company {} HQ", side_word(side), k + 1), "generic/cp-company", 2, x, back(600.0), false));
        hqs.push(json!({ "id": id, "side": side, "echelon": "company", "postId": post, "parent": format!("{side}-bn"), "alternate": format!("{side}-bn"), "subordinates": g, "planner": "land", "decider": "rules" }));
        subs.push(id);
    }
    subs.extend(support);
    hqs.insert(0, json!({ "id": format!("{side}-bn"), "side": side, "echelon": "battalion", "postId": bn, "subordinates": subs, "planner": "land", "decider": "rules" }));
    hqs
}

/// Where a pack puts the fight: (centre in the pack's frame, axis bearing rad, objective offset along the axis m).
fn pack_site(name: &str, kind: Kind) -> Result<(EnuFrame, Enu, f64)> {
    let dir = crate::resolve_pack(name);
    let meta: bgws_env::PackMeta = serde_json::from_slice(&std::fs::read(dir.join("pack.json")).map_err(|e| anyhow!("pack {name}: {e}"))?)?;
    let frame = EnuFrame::new(meta.frame_origin);
    let s = meta.spec;
    let centre = Enu::new(s.origin_e + s.cell * (s.width - 1) as f64 / 2.0, s.origin_n + s.cell * (s.height - 1) as f64 / 2.0, 0.0);
    match kind {
        Kind::UrbanAttack => {
            let towns: Vec<bgws_env::Town> = meta.towns.as_ref().and_then(|f| std::fs::read(dir.join(f)).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            let t = towns.iter().max_by(|a, b| a.radius_m.total_cmp(&b.radius_m)).ok_or_else(|| anyhow!("pack {name} has no towns"))?;
            // Attack from the south into the town's edge, not its far side.
            Ok((frame, Enu::new(t.e, t.n - t.radius_m * 0.6, 0.0), 0.0))
        }
        Kind::RiverCrossing => {
            let rivers: Vec<bgws_env::River> = meta.rivers.as_ref().and_then(|f| std::fs::read(dir.join(f)).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
            // The widest river's point nearest the centre, crossed at right angles.
            let mut best: Option<(f64, Enu, f64)> = None;
            for r in rivers.iter().filter(|r| r.width_m >= 15.0) {
                for w in r.line.windows(2) {
                    let m = Enu::new((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0, 0.0);
                    let d = m.sub(centre).horizontal();
                    let score = d - r.width_m * 50.0;
                    if best.as_ref().is_none_or(|b| score < b.0) {
                        let along = (w[1].0 - w[0].0).atan2(w[1].1 - w[0].1);
                        best = Some((score, m, along + std::f64::consts::FRAC_PI_2));
                    }
                }
            }
            let (_, at, axis) = best.ok_or_else(|| anyhow!("pack {name} has no river wide enough"))?;
            Ok((frame, at, axis))
        }
        _ => Ok((frame, centre, 0.0)),
    }
}

/// Build a scenario from a spec.
pub fn build(spec: &Spec) -> Result<Value> {
    let tpl_path = crate::data_dir().join("scenarios").join("m3-battalion-attack.json");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&tpl_path)?)?;
    let company = spec.echelon == Echelon::Company;
    let front = if company { 1_200.0 } else { 2_800.0 };
    // A company attack meets a platoon-sized position of three elements; a
    // company defence holds with four; a battalion's with six.
    let def_n = match (company, spec.kind) {
        (false, _) => 6,
        (true, Kind::Defence | Kind::Withdrawal) => 4,
        (true, _) => 3,
    };
    let att_n = ((def_n as f64 * spec.ratio).round() as usize).clamp(2, if company { 8 } else { 15 });
    let half = spec.gap_km * 1_000.0 / 2.0;
    // Blue south of red; the objective near red's line (or between them).
    let (mut blue, mut red): (Vec<El>, Vec<El>);
    let mut objectives: Vec<(&str, f64, f64)> = Vec::new();
    // A prepared defence starts dug in.
    let mut prepared = false;
    match spec.kind {
        Kind::Attack | Kind::RiverCrossing | Kind::UrbanAttack => {
            blue = attackers("blue", "b", att_n, -half, front);
            red = defenders("red", "r", def_n, half, front, false);
            prepared = true;
            if !company {
                red.push(("r-tanks".into(), "Red counter-attack tanks".into(), "generic/mbt", 4, 0.0, half + 1_200.0, false));
            }
            objectives.push(("blue", 0.0, half + 100.0));
        }
        Kind::Defence => {
            red = attackers("red", "r", att_n, half, front);
            blue = defenders("blue", "b", def_n, -half, front, false);
            prepared = true;
            objectives.push(("red", 0.0, -half - 100.0));
        }
        Kind::Meeting => {
            let n = 4;
            blue = attackers("blue", "b", n, -half, front);
            red = attackers("red", "r", n, half, front);
            objectives.push(("blue", 0.0, 0.0));
            objectives.push(("red", 0.0, 0.0));
        }
        Kind::Ambush => {
            blue = attackers("blue", "b", att_n, -half, front * 0.5);
            // Hidden on the flank of the way, half way along.
            red = defenders("red", "r", 4, 0.0, 600.0, true).into_iter().map(|mut e| {
                e.4 += 900.0;
                e
            }).collect();
            objectives.push(("blue", 0.0, half));
        }
        Kind::Withdrawal => {
            // A mechanised blue screen falls back to a rear position and must
            // deny it to red until the time limit. Blue has a start on the
            // pursuit: red begins the whole gap away, out of direct-fire reach.
            blue = attackers("blue", "b", 3, 0.0, front);
            red = attackers("red", "r", att_n, 2.0 * half, front);
            objectives.push(("red", 0.0, -1.5 * half));
        }
    }
    let (blue_n, red_n) = (blue.len(), red.len());
    // Mortars behind each side.
    blue.push(("b-mortar".into(), "Blue mortars".into(), "generic/mortar120", 2, 0.0, blue.iter().map(|e| e.5).fold(f64::INFINITY, f64::min) - 1_500.0, false));
    red.push(("r-mortar".into(), "Red mortars".into(), "generic/mortar120", 2, 0.0, red.iter().map(|e| e.5).fold(f64::NEG_INFINITY, f64::max) + 1_500.0, false));
    let blue_y = blue.iter().map(|e| e.5).fold(f64::INFINITY, f64::min);
    let red_y = red.iter().map(|e| e.5).fold(f64::NEG_INFINITY, f64::max);
    // The defender of a battalion attack is a reinforced company.
    let red_company = company || matches!(spec.kind, Kind::Attack | Kind::RiverCrossing | Kind::UrbanAttack | Kind::Ambush);
    let blue_company = company || matches!(spec.kind, Kind::Defence | Kind::Withdrawal);
    let mut blue_hqs = command("blue", "b", &mut blue, blue_y, 1.0, blue_company);
    let mut red_hqs = command("red", "r", &mut red, red_y, -1.0, red_company);
    let forces = [Force { side: "blue", elements: blue, hqs: std::mem::take(&mut blue_hqs) }, Force { side: "red", elements: red, hqs: std::mem::take(&mut red_hqs) }];
    // Where it is: a frame and a rotation (packs put the axis across a river, into a town).
    let (frame, centre, axis) = match &spec.ground {
        GroundSpec::Pack { name } => pack_site(name, spec.kind)?,
        _ => (EnuFrame::new(Geodetic::new(54.2, 18.6, 0.0)), Enu::new(0.0, 0.0, 0.0), 0.0),
    };
    // Packs: the river or the town edge is where the objective's approach crosses.
    let shift = match spec.kind {
        Kind::RiverCrossing => half * 0.5,
        Kind::UrbanAttack => -half - 100.0,
        _ => 0.0,
    };
    let place = |x: f64, y: f64| -> Value {
        let y = y + shift;
        let (s, c) = axis.sin_cos();
        // The local frame's north is the axis.
        let p = Enu::new(centre.e + x * c + y * s, centre.n - x * s + y * c, 0.0);
        let g = frame.enu_to_geodetic(p);
        json!({ "lat": g.lat, "lon": g.lon, "h": 0.0 })
    };
    let mut list = Vec::new();
    let mut hqs = Vec::new();
    for f in &forces {
        for e in &f.elements {
            let mut j = json!({ "id": e.0, "label": e.1, "side": f.side, "platform": e.2, "count": e.3, "quality": "regular", "position": place(e.4, e.5), "layer": "unit" });
            if e.6 {
                j["concealed"] = json!(true);
            }
            let defending = matches!((spec.kind, f.side), (Kind::Defence, "blue") | (Kind::Attack | Kind::RiverCrossing | Kind::UrbanAttack, "red"));
            if prepared && defending && matches!(e.2, "generic/infantrySection" | "generic/atgmTeam") {
                j["dugIn"] = json!(true);
            }
            if e.2.starts_with("generic/cp-") {
                j["commandRating"] = json!(if e.2.ends_with("battalion") { 3.0 } else { 2.0 });
            }
            list.push(j);
        }
        hqs.extend(f.hqs.iter().cloned());
    }
    let radius = if company { 600.0 } else { 900.0 };
    let victory: Vec<Value> = objectives.iter().map(|(side, x, y)| json!({ "kind": "objective", "side": side, "at": place(*x, *y), "radiusM": radius, "holdS": 600.0, "label": format!("{side}'s objective") })).chain(std::iter::once(json!({ "kind": "breakpoint", "pct": 60.0 }))).collect();
    let centre_g = frame.enu_to_geodetic(centre);
    let utc = (spec.hour - centre_g.lon / 15.0).rem_euclid(24.0);
    let (hh, mm) = (utc.floor() as u32, ((utc.fract()) * 60.0).floor() as u32);
    v["name"] = json!(spec.name);
    v["description"] = json!(describe(spec, blue_n, red_n));
    v["seed"] = json!(spec.seed);
    v["seedText"] = json!(spec.name);
    v["startUtc"] = json!(format!("2026-06-01T{hh:02}:{mm:02}:00Z"));
    v["weather"] = json!({ "source": "preset", "weather": spec.weather });
    v["scope"]["durationMin"] = json!(spec.minutes);
    v["scope"]["echelons"]["highest"] = json!(if company { "company" } else { "battalion" });
    v["scope"]["planners"] = json!(["land"]);
    v["theatre"] = match &spec.ground {
        GroundSpec::Flat => json!({ "origin": centre_g, "ground": "flat" }),
        GroundSpec::Generated => json!({ "origin": centre_g, "ground": "generated", "groundSeed": format!("{}-ground", spec.name) }),
        GroundSpec::Pack { name } => json!({ "origin": centre_g, "ground": "generated", "pack": name }),
    };
    v["forces"] = Value::Array(list);
    v["command"]["hqs"] = Value::Array(hqs);
    v["world"] = json!({ "obstacles": [] });
    v["victory"] = Value::Array(victory);
    if let Some(o) = v.as_object_mut() {
        o.remove("convertedFrom");
    }
    Ok(v)
}

fn describe(spec: &Spec, blue: usize, red: usize) -> String {
    let what = match spec.kind {
        Kind::Attack => "Blue attacks a prepared red position",
        Kind::Defence => "Red attacks a blue position",
        Kind::Meeting => "A meeting engagement: both advance on the same ground",
        Kind::Ambush => "Blue advances through ground red has laid an ambush in",
        Kind::Withdrawal => "Blue, outnumbered, falls back to a rear position with red in pursuit and must deny it to red",
        Kind::RiverCrossing => "Blue attacks across a river",
        Kind::UrbanAttack => "Blue attacks into a town",
    };
    format!(
        "Generated ({:?}, {:?}): {what}. {blue} blue against {red} red manoeuvre elements (dug in where the defence is prepared), {:.1} km apart, at {:02.0}:00 local, {}; {:.0} min. A side with an objective wins by taking it and holding it 10 minutes, and loses if it has not by the time limit.",
        spec.kind, spec.echelon, spec.gap_km, spec.hour, spec.weather, spec.minutes
    )
}

/// The standard set the suite plays.
pub fn standard() -> Vec<Spec> {
    let s = |name: &str, kind: Kind, echelon: Echelon, ratio: f64, gap_km: f64, hour: f64, weather: &str, ground: GroundSpec, minutes: f64, seed: u64| Spec { name: name.into(), kind, echelon, ratio, gap_km, hour, weather: weather.into(), ground, minutes, seed };
    let gen = GroundSpec::Generated;
    let kgd = GroundSpec::Pack { name: "kaliningrad-big".into() };
    vec![
        s("gen-company-attack", Kind::Attack, Echelon::Company, 3.0, 4.0, 10.0, "clear", gen.clone(), 90.0, 101),
        s("gen-company-attack-night", Kind::Attack, Echelon::Company, 3.0, 4.0, 1.0, "clear", gen.clone(), 90.0, 102),
        s("gen-company-attack-even", Kind::Attack, Echelon::Company, 1.5, 4.0, 10.0, "overcast", gen.clone(), 90.0, 103),
        s("gen-battalion-attack", Kind::Attack, Echelon::Battalion, 2.0, 6.0, 9.0, "clear", gen.clone(), 150.0, 104),
        s("gen-company-defence", Kind::Defence, Echelon::Company, 1.5, 4.0, 14.0, "clear", gen.clone(), 90.0, 105),
        s("gen-battalion-defence-fog", Kind::Defence, Echelon::Battalion, 1.5, 6.0, 7.0, "fog", gen.clone(), 150.0, 106),
        s("gen-meeting-engagement", Kind::Meeting, Echelon::Company, 2.0, 6.0, 12.0, "clear", gen.clone(), 90.0, 107),
        s("gen-ambush", Kind::Ambush, Echelon::Company, 1.5, 5.0, 16.0, "clear", gen.clone(), 90.0, 108),
        s("gen-withdrawal", Kind::Withdrawal, Echelon::Company, 1.5, 6.0, 19.0, "rain", gen, 90.0, 109),
        s("gen-kgd-company-attack", Kind::Attack, Echelon::Company, 3.0, 4.0, 10.0, "clear", kgd.clone(), 90.0, 110),
        s("gen-kgd-river-crossing", Kind::RiverCrossing, Echelon::Company, 3.0, 4.0, 11.0, "clear", kgd.clone(), 90.0, 111),
        s("gen-kgd-urban-attack", Kind::UrbanAttack, Echelon::Battalion, 2.0, 5.0, 9.0, "overcast", kgd, 150.0, 112),
    ]
}

/// Write a spec's scenario to `data/scenarios/<name>.json`.
pub fn write(spec: &Spec) -> Result<std::path::PathBuf> {
    let v = build(spec)?;
    let p = crate::data_dir().join("scenarios").join(format!("{}.json", spec.name));
    std::fs::write(&p, serde_json::to_vec_pretty(&v)?)?;
    Ok(p)
}
