//! The v3 → v4 converter (§14.3: "A converter reads the TypeScript v3 files").
//!
//! **The geometry assumption.** TS scenarios were written in local metres —
//! east and north of an origin — and stored as lat/lng by a flat-earth
//! projection (lib/forceBuilder.ts `offsetToLatLng`: 111,320 m per degree of
//! latitude, and that times cos(origin latitude) per degree of longitude).
//! That is exact to a metre over a skirmish board and off by hundreds of
//! metres to kilometres over a theatre. The converter therefore recovers
//! the authored metres by inverting that projection about the origin, and
//! places them exactly on WGS84 through an ENU tangent frame at the origin
//! (`bgws_core::EnuFrame`): east and north are kept exactly in that frame,
//! and the up component is solved so the point sits at its stated height
//! above the ellipsoid (0 on land and sea, `altM` for aircraft, negative for
//! submarines). The origin is the TS
//! `DEFAULT_ORIGIN` (54.2° N, 18.6° E), which every TS preset and force list
//! used, unless the file's positions are clearly elsewhere — then their
//! centroid — or the caller names one. `Placement::KeepLatLon` keeps the v3
//! lat/lng as they were instead (what the TS engine measured with its
//! haversine distances). Each converted file records the choice in
//! `convertedFrom.notes`.
//!
//! **Time.** v3 has a start hour (local) but no date. The converter takes a
//! date (default 2026-06-01) and reads the hour as local mean solar time at
//! the origin, so the sun is where the author expected it.

use crate::scope::Weather;
use crate::time::{days_from_civil, format_utc};
use crate::v3::{LatLng, ScenarioV3, V3_FORMAT};
use crate::v4::*;
use bgws_catalogue::Catalogue;
use bgws_core::rng::fnv1a;
use bgws_core::{Enu, EnuFrame, Geodetic, Seed, Side};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use thiserror::Error;

/// The TS board origin (lib/forceBuilder.ts `DEFAULT_ORIGIN`).
pub const TS_DEFAULT_ORIGIN: Geodetic = Geodetic { lat: 54.2, lon: 18.6, h: 0.0 };
/// lib/forceBuilder.ts `METRES_PER_DEGREE_LAT`.
pub const TS_METRES_PER_DEGREE_LAT: f64 = 111_320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Placement {
    /// Recover the authored east/north metres and place them exactly (the default).
    #[default]
    AuthoredMetres,
    /// Keep the v3 lat/lng unchanged.
    KeepLatLon,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvertOptions {
    /// The origin the v3 file's metres were measured from; `None` chooses (see the module notes).
    pub origin: Option<Geodetic>,
    pub placement: Placement,
    /// The start date (year, month, day) the v3 start hour is on.
    pub date: (i64, u32, u32),
    /// Margin added round the theatre bounds, as a share of their size.
    pub bounds_margin: f64,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions { origin: None, placement: Placement::AuthoredMetres, date: (2026, 6, 1), bounds_margin: 0.05 }
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum ConvertError {
    #[error("{field}: {message}")]
    Invalid { field: String, message: String },
}

fn invalid(field: impl Into<String>, message: impl Into<String>) -> ConvertError {
    ConvertError::Invalid { field: field.into(), message: message.into() }
}

/// The inverse of the TS flat projection: lat/lng to east/north metres about `origin`.
pub fn ts_offset_of(origin: Geodetic, p: LatLng) -> (f64, f64) {
    let per_lng = TS_METRES_PER_DEGREE_LAT * origin.lat.to_radians().cos();
    ((p.lng - origin.lon) * per_lng, (p.lat - origin.lat) * TS_METRES_PER_DEGREE_LAT)
}

/// The TS flat projection: east/north metres about `origin` to lat/lng.
pub fn ts_latlng_of(origin: Geodetic, east: f64, north: f64) -> LatLng {
    let per_lng = TS_METRES_PER_DEGREE_LAT * origin.lat.to_radians().cos();
    LatLng { lat: origin.lat + north / TS_METRES_PER_DEGREE_LAT, lng: origin.lon + east / per_lng }
}

/// Places v3 positions on WGS84.
pub struct Placer {
    pub origin: Geodetic,
    pub placement: Placement,
    frame: EnuFrame,
}

impl Placer {
    pub fn new(origin: Geodetic, placement: Placement) -> Self {
        Placer { origin, placement, frame: EnuFrame::new(origin) }
    }
    pub fn place(&self, p: LatLng, h: f64) -> Geodetic {
        match self.placement {
            Placement::KeepLatLon => Geodetic::new(p.lat, p.lng, h),
            Placement::AuthoredMetres => {
                // Keep east/north exactly in the origin's frame and find the
                // up component that puts the point at height `h` (Newton steps
                // on h(u), whose slope is ~1 at these ranges).
                let (e, n) = ts_offset_of(self.origin, p);
                let mut u = h - (e * e + n * n) / (2.0 * bgws_core::geo::EARTH_RADIUS);
                let mut g = self.frame.enu_to_geodetic(Enu::new(e, n, u));
                for _ in 0..8 {
                    let err = h - g.h;
                    if err.abs() < 1e-6 {
                        break;
                    }
                    u += err;
                    g = self.frame.enu_to_geodetic(Enu::new(e, n, u));
                }
                g.h = h;
                g
            }
        }
    }
    /// Replace every `{lat, lng}` object in a JSON tree with a placed `{lat, lon, h}`.
    pub fn place_json(&self, v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                if m.len() == 2 {
                    if let (Some(lat), Some(lng)) = (m.get("lat").and_then(Value::as_f64), m.get("lng").and_then(Value::as_f64)) {
                        let g = self.place(LatLng { lat, lng }, 0.0);
                        return json!({"lat": g.lat, "lon": g.lon, "h": g.h});
                    }
                }
                Value::Object(m.iter().map(|(k, x)| (k.clone(), self.place_json(x))).collect())
            }
            Value::Array(a) => Value::Array(a.iter().map(|x| self.place_json(x)).collect()),
            other => other.clone(),
        }
    }
}

fn side_of(s: &str, field: &str) -> Result<Side, ConvertError> {
    match s {
        "blue" => Ok(Side::Blue),
        "red" => Ok(Side::Red),
        "neutral" => Ok(Side::Neutral),
        _ => Err(invalid(field, format!("unknown side `{s}`"))),
    }
}

fn quality_of(q: &str) -> Option<TroopQuality> {
    match q {
        "conscript" => Some(TroopQuality::Conscript),
        "regular" => Some(TroopQuality::Regular),
        "veteran" => Some(TroopQuality::Veteran),
        "elite" => Some(TroopQuality::Elite),
        _ => None,
    }
}

fn typed<T: serde::de::DeserializeOwned>(v: Value, field: &str) -> Result<T, ConvertError> {
    serde_json::from_value(v).map_err(|e| invalid(field, e.to_string()))
}

/// Every v3 position, for choosing the origin.
fn v3_positions(v3: &ScenarioV3) -> Vec<LatLng> {
    let mut v: Vec<LatLng> = v3.forces.iter().map(|f| f.position).collect();
    if let Some(w) = &v3.world {
        v.extend(w.entities.iter().map(|e| e.at));
    }
    v
}

/// Choose the origin the v3 metres were measured from.
pub fn choose_origin(v3: &ScenarioV3) -> (Geodetic, String) {
    let pts = v3_positions(v3);
    if pts.is_empty() {
        return (TS_DEFAULT_ORIGIN, "no positions; origin is the TS DEFAULT_ORIGIN".into());
    }
    let n = pts.len() as f64;
    let c = LatLng { lat: pts.iter().map(|p| p.lat).sum::<f64>() / n, lng: pts.iter().map(|p| p.lng).sum::<f64>() / n };
    let (e, no) = ts_offset_of(TS_DEFAULT_ORIGIN, c);
    let reach = (v3.scope.area.width_km.max(v3.scope.area.depth_km) * 1000.0).max(50_000.0);
    if e.abs() <= reach && no.abs() <= reach {
        (TS_DEFAULT_ORIGIN, "origin is the TS DEFAULT_ORIGIN (54.2 N, 18.6 E), which the positions sit round".into())
    } else {
        (Geodetic::new(c.lat, c.lng, 0.0), format!("positions are far from the TS DEFAULT_ORIGIN; origin is their centroid ({:.5}, {:.5})", c.lat, c.lng))
    }
}

/// Convert a parsed v3 scenario. A catalogue, when given, supplies entity counts.
pub fn convert_v3(v3: &ScenarioV3, catalogue: Option<&Catalogue>, opts: &ConvertOptions) -> Result<ScenarioV4, ConvertError> {
    let (origin, origin_note) = match opts.origin {
        Some(o) => (o, format!("origin given by the caller ({:.5}, {:.5})", o.lat, o.lon)),
        None => choose_origin(v3),
    };
    let placer = Placer::new(origin, opts.placement);
    let mut notes = vec![
        origin_note,
        match opts.placement {
            Placement::AuthoredMetres => "positions: the TS flat projection inverted to authored east/north metres, then placed exactly through an ENU frame at the origin".into(),
            Placement::KeepLatLon => "positions: v3 lat/lng kept unchanged".into(),
        },
    ];
    let mut forces = Vec::new();
    let mut assign = v3.behaviours.clone();
    for (i, f) in v3.forces.iter().enumerate() {
        let field = format!("forces[{i}]");
        let quality = quality_of(&f.troop_quality).unwrap_or_else(|| {
            notes.push(format!("{}: troop quality `{}` unknown, read as regular", f.id, f.troop_quality));
            TroopQuality::Regular
        });
        forces.push(Force {
            id: f.id.clone(),
            label: f.label.clone(),
            side: side_of(&f.side, &field)?,
            platform: f.platform.clone(),
            count: f.platform_count.as_f64().unwrap_or(1.0).round().max(1.0) as u32,
            quality,
            position: placer.place(f.position, 0.0),
            heading_deg: None,
            layer: Layer::Unit,
            echelon: None,
            concealed: f.concealed.unwrap_or(false),
            command_rating: f.command_rating,
            dummy: f.is_dummy.unwrap_or(false),
            parent: None,
            task: None,
            orbit: None,
            provider: None,
            lane: Vec::new(),
            status: None,
        });
    }
    let world = v3.world.clone().unwrap_or_default();
    for (i, e) in world.entities.iter().enumerate() {
        let field = format!("world.entities[{i}]");
        let task = match &e.task {
            Some(t) => Some(typed::<EntityTask>(placer.place_json(t), &format!("{field}.task"))?),
            None => None,
        };
        let count = catalogue.and_then(|c| c.platform(&e.archetype)).map(|p| p.body.count).unwrap_or(1).max(1);
        if let Some(t) = &e.template {
            assign.insert(e.id.clone(), t.clone());
        }
        forces.push(Force {
            id: e.id.clone(),
            label: e.label.clone().unwrap_or_else(|| e.id.clone()),
            side: side_of(&e.owner, &field)?,
            platform: e.archetype.clone(),
            count,
            quality: TroopQuality::Regular,
            position: placer.place(e.at, e.alt_m.unwrap_or(0.0)),
            heading_deg: None,
            layer: Layer::Entity,
            echelon: None,
            concealed: false,
            command_rating: None,
            dummy: false,
            parent: e.parent.clone(),
            task,
            orbit: if e.orbit_phase_deg.is_some() || e.orbit_raan_deg.is_some() {
                Some(OrbitPlacement { phase_deg: e.orbit_phase_deg.unwrap_or(0.0), raan_deg: e.orbit_raan_deg.unwrap_or(0.0) })
            } else {
                None
            },
            provider: e.provider.clone(),
            lane: e.lane.iter().flatten().map(|p| placer.place(*p, 0.0)).collect(),
            status: e.status.clone(),
        });
    }
    // The world's other sections, typed after placing their positions.
    let obstacles: Vec<Obstacle> = typed(placer.place_json(&Value::Array(world.obstacles.clone())), "world.obstacles")?;
    let measures: Vec<ControlMeasure> = typed(placer.place_json(&Value::Array(world.measures.clone())), "world.measures")?;
    let cyber: Vec<CyberAccess> = typed(Value::Array(world.cyber.clone()), "world.cyber")?;
    let nais: Vec<NamedArea> = typed(placer.place_json(&Value::Array(world.nais.clone())), "world.nais")?;
    let sea: Option<SeaArea> = match &world.sea {
        Some(s) => Some(typed(placer.place_json(s), "world.sea")?),
        None => None,
    };
    let mut network = BTreeMap::new();
    for (k, v) in world.network.iter().flatten() {
        let n = match v.as_str() {
            "voice" => Network::Voice,
            "datalink" => Network::Datalink,
            other => return Err(invalid("world.network", format!("unknown network `{other}`"))),
        };
        network.insert(side_of(k, "world.network")?, n);
    }
    let hqs: Vec<Hq> = world
        .hqs
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let mut h = h.clone();
            if let Some(m) = h.as_object_mut() {
                if let Some(seat) = m.remove("seat") {
                    let d = match seat.as_str() {
                        Some("claude") => "claude",
                        Some("human") => "person",
                        _ => "rules",
                    };
                    m.insert("decider".into(), Value::from(d));
                }
            }
            typed::<Hq>(h, &format!("world.hqs[{i}]"))
        })
        .collect::<Result<_, _>>()?;
    let mut victory: Vec<VictoryCondition> = typed(placer.place_json(&Value::Array(world.victory.clone())), "world.victory")?;
    victory.push(VictoryCondition::Breakpoint { side: None, pct: v3.rules.breakpoint_pct });
    let events: Vec<ScriptedEvent> = typed(Value::Array(world.events.clone()), "world.events")?;

    // Environment, weather and the start instant.
    let env = world.environment.clone().unwrap_or_default();
    let get = |k: &str| env.get(k).and_then(Value::as_f64);
    let environment = EnvironmentSpec {
        visibility_km: get("visibilityKm"),
        cloud_base_m: get("cloudBaseM"),
        cloud_cover: get("cloudCover"),
        precipitation: get("precipitation"),
        wind_ms: get("windMs"),
        sea_state: get("seaState"),
    };
    let time = v3.scope.time_or_default();
    let start_hour = get("startHour").unwrap_or(time.start_hour);
    let (y, m, d) = opts.date;
    let utc_s = days_from_civil(y, m, d) * 86_400 + ((start_hour - origin.lon / 15.0) * 3600.0).round() as i64;
    notes.push(format!("start: {start_hour} h local mean solar time at the origin on {y:04}-{m:02}-{d:02} (v3 has no date)"));
    let weather = WeatherSource::Preset { weather: v3.scope.time.map(|t| t.weather).unwrap_or(Weather::Clear) };

    // Command.
    let mut sides = BTreeMap::new();
    for (s, dir) in [(Side::Blue, &v3.command.directives.blue), (Side::Red, &v3.command.directives.red)] {
        sides.insert(
            s,
            SideCommand {
                units: if v3.command.use_jev { Decider::Jev } else { Decider::Rules },
                hqs: if v3.command.use_commander { Decider::Claude } else { Decider::Rules },
                directive: dir.clone(),
            },
        );
    }
    let mut default_emcon = BTreeMap::new();
    default_emcon.insert(Side::Blue, Emcon::Free);
    default_emcon.insert(Side::Red, Emcon::Free);

    let mut s = ScenarioV4 {
        format: V4_FORMAT.into(),
        version: V4_VERSION,
        name: v3.name.clone(),
        description: String::new(),
        saved_at: v3.saved_at.clone(),
        // 53 bits, so the seed survives a JavaScript reader.
        seed: Seed(fnv1a(v3.seed.as_bytes()) & ((1u64 << 53) - 1)),
        seed_text: Some(v3.seed.clone()),
        theatre: Theatre { origin, bounds: None, pack: None, ground: v3.ground.choice.clone(), ground_seed: v3.ground.seed.clone() },
        scope: v3.scope.clone(),
        start_utc: format_utc(utc_s),
        weather,
        environment,
        comms: if v3.rules.comms == "perfect" { CommsRule::Perfect } else { CommsRule::Radio },
        forces,
        world: WorldSetup { obstacles, measures, cyber, nais, sea, network },
        command: Command { sides, model: v3.command.model.clone(), hqs },
        behaviours: Behaviours { assign, templates: v3.behaviour_templates.clone() },
        emitters: EmitterSettings { default: default_emcon, forces: BTreeMap::new() },
        victory,
        events,
        notes: v3.caveats.clone(),
        converted_from: None,
    };
    // The theatre: the scope's area round the origin, enlarged to hold every position.
    let frame = EnuFrame::new(origin);
    let (hw, hd) = (s.scope.area.width_km * 500.0, s.scope.area.depth_km * 500.0);
    let corners = [(-hw, -hd), (hw, -hd), (hw, hd), (-hw, hd)].map(|(e, n)| frame.enu_to_geodetic(Enu::new(e, n, 0.0)));
    let area = Bounds::around(corners).unwrap_or(Bounds { south: origin.lat, west: origin.lon, north: origin.lat, east: origin.lon });
    let all = Bounds::around(s.positions().into_iter().map(|(_, p)| p)).unwrap_or(area);
    let mut b = area.union(all);
    if b != area {
        notes.push("theatre bounds enlarged beyond the scope's area to hold every placed position".into());
    }
    let (dl, dn) = ((b.north - b.south) * opts.bounds_margin, (b.east - b.west) * opts.bounds_margin);
    b = Bounds { south: (b.south - dl).max(-90.0), north: (b.north + dl).min(90.0), west: (b.west - dn).max(-180.0), east: (b.east + dn).min(180.0) };
    s.theatre.bounds = Some(b);
    s.converted_from = Some(Provenance { from_format: V3_FORMAT.into(), from_version: v3.version, notes });
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn flat_projection_round_trips() {
        let p = ts_latlng_of(TS_DEFAULT_ORIGIN, 1234.0, -5678.0);
        let (e, n) = ts_offset_of(TS_DEFAULT_ORIGIN, p);
        assert_relative_eq!(e, 1234.0, epsilon = 1e-6);
        assert_relative_eq!(n, -5678.0, epsilon = 1e-6);
    }

    #[test]
    fn authored_metres_are_kept_exactly() {
        let placer = Placer::new(TS_DEFAULT_ORIGIN, Placement::AuthoredMetres);
        let frame = EnuFrame::new(TS_DEFAULT_ORIGIN);
        for (e, n) in [(0.0, 3200.0), (-25_000.0, 90_000.0), (150_000.0, -10_000.0), (0.0, -220_000.0)] {
            let g = placer.place(ts_latlng_of(TS_DEFAULT_ORIGIN, e, n), 0.0);
            let back = frame.geodetic_to_enu(g);
            // Authored metres kept exactly in the origin's frame, at height 0.
            assert!((back.e - e).abs() < 1e-3 && (back.n - n).abs() < 1e-3, "{e},{n}: {back:?}");
            assert!(g.h.abs() < 1e-9);
        }
        // Small boards: the flat projection and the exact frame agree to a metre.
        let flat = ts_latlng_of(TS_DEFAULT_ORIGIN, 500.0, 3000.0);
        let exact = placer.place(flat, 0.0);
        assert!(Geodetic::new(flat.lat, flat.lng, 0.0).surface_distance(exact) < 1.0);
        // Theatre distances: the difference grows to hundreds of metres.
        let far = ts_latlng_of(TS_DEFAULT_ORIGIN, 150_000.0, 150_000.0);
        let d = Geodetic::new(far.lat, far.lng, 0.0).surface_distance(placer.place(far, 0.0));
        assert!(d > 100.0, "{d}");
    }
}
