//! Synthetic scenarios for the LOD tests and the performance benches (§17):
//! forces placed in metres on the theatre's ENU frame, and the synthetic
//! brigade of §13 (2,000 land entities, two sides facing each other).

use bgws_core::{EnuFrame, Geodetic, Side};
use bgws_scenario::v4::ScenarioV4;
use serde_json::{json, Value};

/// The theatre origin every synthetic scenario uses.
pub const ORIGIN: Geodetic = Geodetic { lat: 54.2, lon: 18.6, h: 0.0 };

/// A point `e`, `n` metres from the origin, `h` metres up.
pub fn geo(e: f64, n: f64, h: f64) -> Geodetic {
    let g = EnuFrame::new(ORIGIN).enu_to_geodetic(bgws_core::Enu::new(e, n, 0.0));
    Geodetic::new(g.lat, g.lon, h)
}

/// One force, placed in metres.
#[derive(Clone, Debug)]
pub struct SynthForce {
    pub id: String,
    pub side: Side,
    pub platform: String,
    pub count: u32,
    pub e: f64,
    pub n: f64,
    /// Altitude (aircraft) or 0.
    pub h: f64,
    pub parent: Option<String>,
}

impl SynthForce {
    pub fn new(id: &str, side: Side, platform: &str, count: u32, e: f64, n: f64) -> Self {
        SynthForce { id: id.into(), side, platform: platform.into(), count, e, n, h: 0.0, parent: None }
    }
}

/// A scenario built in code.
#[derive(Clone, Debug)]
pub struct SynthScenario {
    pub name: String,
    pub forces: Vec<SynthForce>,
    pub duration_min: f64,
    /// `scope.detail.aggregateAwayFromContact`.
    pub aggregate: bool,
    /// `scope.time.skipQuiet`.
    pub skip_quiet: bool,
    pub flat: bool,
    /// HQ planners (the rules plan: blue advances on the objective, red holds).
    pub planners: bool,
    /// The scenario's `victory` array.
    pub victory: Value,
}

impl SynthScenario {
    pub fn new(name: &str, forces: Vec<SynthForce>) -> Self {
        SynthScenario { name: name.into(), forces, duration_min: 240.0, aggregate: false, skip_quiet: false, flat: true, planners: true, victory: json!([{ "kind": "breakpoint", "pct": 70.0 }]) }
    }

    pub fn to_json(&self) -> Value {
        let forces: Vec<Value> = self
            .forces
            .iter()
            .map(|f| {
                let mut v = json!({ "id": f.id, "label": f.id, "side": f.side.as_str(), "platform": f.platform, "count": f.count, "quality": "regular", "position": geo(f.e, f.n, f.h), "layer": "unit" });
                if let Some(p) = &f.parent {
                    v["parent"] = json!(p);
                }
                v
            })
            .collect();
        let off = "off";
        json!({
            "format": "bgws.scenario", "version": 4, "name": self.name, "description": "synthetic", "savedAt": "2026-10-25T00:00:00.000Z", "seed": 1,
            "theatre": { "origin": ORIGIN, "ground": if self.flat { "flat" } else { "generated" }, "groundSeed": format!("{}-ground", self.name) },
            "scope": {
                "type": "custom", "durationMin": self.duration_min, "area": { "widthKm": 100.0, "depthKm": 100.0 }, "echelons": { "highest": "brigade", "lowest": "platoon" },
                "domains": { "land": "simulated", "drones": off, "fires": off, "ew": off, "air": off, "airDefence": off, "naval": off, "space": off, "cyber": off, "logistics": off, "medical": off },
                "planners": if self.planners { json!(["tactical"]) } else { json!([]) },
                "detail": { "aggregateAwayFromContact": self.aggregate, "jev": "all" },
                "time": { "startHour": 10.0, "weather": "clear", "maxCompression": 100.0, "skipQuiet": self.skip_quiet }
            },
            "startUtc": "2026-06-01T08:00:00Z", "weather": { "source": "preset", "weather": "clear" }, "environment": {}, "comms": "radio", "world": {},
            "command": { "sides": { "blue": { "units": "rules", "hqs": "rules", "directive": "" }, "red": { "units": "rules", "hqs": "rules", "directive": "" } }, "model": "rules" },
            "behaviours": { "assign": {} }, "emitters": { "default": { "blue": "free", "red": "free" } },
            "victory": self.victory, "forces": forces
        })
    }

    pub fn build(&self) -> ScenarioV4 {
        ScenarioV4::from_json(&self.to_json().to_string()).expect("a synthetic scenario is valid")
    }
}

/// The synthetic brigade (§13): `entities` land units, half a side, in a
/// block `width_m` wide on each side of a gap of `gap_m`, a mix of tank,
/// infantry-fighting-vehicle and infantry platoons, generated ground.
pub fn brigade(entities: usize, width_m: f64, depth_m: f64, gap_m: f64) -> SynthScenario {
    let per_side = entities / 2;
    let cols = ((per_side as f64 * width_m / depth_m).sqrt().ceil() as usize).max(1);
    let rows = per_side.div_ceil(cols);
    let mut forces = Vec::with_capacity(entities);
    for side in [Side::Blue, Side::Red] {
        let dir = if side == Side::Blue { -1.0 } else { 1.0 };
        for k in 0..per_side {
            let (c, r) = (k % cols, k / cols);
            let e = -width_m / 2.0 + (c as f64 + 0.5) * width_m / cols as f64;
            let n = dir * (gap_m / 2.0 + (r as f64 + 0.5) * depth_m / rows as f64);
            let (platform, count) = match k % 5 {
                0 | 1 => ("generic/mbt", 4),
                2 | 3 => ("generic/ifv", 3),
                _ => ("generic/infantrySection", 3),
            };
            forces.push(SynthForce::new(&format!("{}-{k}", side.as_str()), side, platform, count, e, n));
        }
    }
    let mut s = SynthScenario::new("synthetic-brigade", forces);
    s.flat = false;
    s
}
