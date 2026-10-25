//! The platform record (RUST_ENGINE.md §5.1, §14.1; MULTI_DOMAIN_PLAN.md §4.1):
//! a platform is built AFSIM-style from parts — a mover, a body, signatures,
//! sensors, weapon mounts, radios, emitters, stores, carried loads and crew
//! defaults — each with more physics than the TypeScript `CatalogueRecord`
//! it ports (RCS by aspect and band, IR by thermal state, acoustic level by
//! speed, radar and sonar parameters, armour by aspect for KE and CE).
//!
//! These are the **resolved** types: what a record looks like after its
//! class defaults, its parent record and the derivation rules (`derive.rs`)
//! have filled every field. Records on disk are partial JSON; see
//! `catalogue.rs` for the resolution order. JSON field names are camelCase
//! with the unit in the name (`roadKmh`, `rangeMaxM`, `ptW`), as in the TS
//! catalogue, so the React UI can read them unchanged.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Which medium a platform moves in. `Node` is a fixed installation;
/// `Munition` flies once; `Cyber` has no physical presence on the map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    Land,
    Surface,
    Subsurface,
    Air,
    Space,
    Node,
    Munition,
    Cyber,
}

/// Where a value came from (W14 provenance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Provenance {
    /// A plausible figure chosen to make the simulation work.
    #[default]
    Synthetic,
    /// From the research reference ([P..]/[R..]).
    Reference,
    /// From a dataset or document.
    Sourced,
}

/// Where one field's value came from, when better than the record's default.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSource {
    pub kind: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
}

/// Command levels, lowest first (scope.ts `ECHELONS`, extended down to team).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Echelon {
    Team,
    Section,
    Platoon,
    Company,
    Battalion,
    Brigade,
    Division,
    Corps,
    Component,
    Joint,
}

impl Echelon {
    /// The levels the TS scope knows (platoon → joint).
    pub const SCOPE: [Echelon; 8] = [
        Echelon::Platoon,
        Echelon::Company,
        Echelon::Battalion,
        Echelon::Brigade,
        Echelon::Division,
        Echelon::Corps,
        Echelon::Component,
        Echelon::Joint,
    ];
}

/// The Claude planners (scope.ts `PLANNERS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Planner {
    Overall,
    Tactical,
    Land,
    Air,
    Maritime,
    SpaceCyber,
    Sustainment,
}

/// MIL-STD-2525 symbol per side.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Sidc {
    pub blue: String,
    pub red: String,
}

impl Sidc {
    /// From a friendly code: padded to 15, the hostile one swaps the
    /// affiliation letter (as generic.ts `sidc()`).
    pub fn from_friendly(friendly: &str) -> Sidc {
        let mut code: String = friendly.chars().take(15).collect();
        while code.chars().count() < 15 {
            code.push('-');
        }
        let red: String = code.chars().enumerate().map(|(i, c)| if i == 1 { 'H' } else { c }).collect();
        Sidc { blue: code, red }
    }
}

// ── Mover ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MoverKind {
    Foot,
    Wheeled,
    Tracked,
    Rotary,
    FixedWing,
    Multirotor,
    /// Cruise, ballistic and pursuit flyers (the TS `cruise`/`ballistic`/`pursuit`).
    #[serde(alias = "cruise", alias = "ballistic", alias = "pursuit")]
    Missile,
    Surface,
    Sub,
    Orbit,
    Static,
}

impl MoverKind {
    pub fn is_ground(self) -> bool {
        matches!(self, MoverKind::Foot | MoverKind::Wheeled | MoverKind::Tracked)
    }
    pub fn is_air(self) -> bool {
        matches!(self, MoverKind::Rotary | MoverKind::FixedWing | MoverKind::Multirotor | MoverKind::Missile)
    }
}

/// How a platform moves, and its performance. Zero means "does not apply".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mover {
    pub kind: MoverKind,
    /// Best speed, km/h: on road, in level flight, on the surface, submerged.
    pub max_kmh: f64,
    /// Usual working speed, km/h.
    pub cruise_kmh: f64,
    /// Ground: sustained road speed, km/h.
    #[serde(default)]
    pub road_kmh: f64,
    /// Ground: sustained cross-country speed on firm going, km/h.
    #[serde(default)]
    pub cross_country_kmh: f64,
    /// Fuel in the platform's own tanks, litres per platform.
    #[serde(default)]
    pub fuel_capacity_l: f64,
    /// Fuel burnt per platform moving (across country, in cruise), L/h.
    #[serde(default)]
    pub burn_moving_lph: f64,
    /// Fuel burnt per platform idling or on station, L/h.
    #[serde(default)]
    pub burn_idle_lph: f64,
    /// Battery for propulsion (drones, robots, a submarine's battery), Wh per platform.
    #[serde(default)]
    pub battery_wh: f64,
    /// Average electrical draw moving, W.
    #[serde(default)]
    pub battery_draw_w: f64,
    /// Endurance on one load, hours (derived from fuel or battery when absent).
    #[serde(default)]
    pub endurance_h: f64,
    /// Air: rate of climb, m/s.
    #[serde(default)]
    pub climb_ms: f64,
    /// Air: service ceiling, m.
    #[serde(default)]
    pub ceiling_m: f64,
    /// Sub: maximum operating depth, m (positive).
    #[serde(default)]
    pub max_depth_m: f64,
    /// Sustained turn rate, degrees per second.
    #[serde(default)]
    pub turn_rate_dps: f64,
    /// Ground: steepest slope it can climb, degrees.
    #[serde(default)]
    pub max_slope_deg: f64,
    /// Ground: water it can ford unprepared, m.
    #[serde(default)]
    pub fording_m: f64,
    /// Ground: power-to-weight, hp/t.
    #[serde(default)]
    pub power_to_weight_hp_t: f64,
    /// Ground: nominal ground pressure, kPa (soft-going trafficability).
    #[serde(default)]
    pub ground_pressure_kpa: f64,
    /// Orbit: altitude, km.
    #[serde(default)]
    pub orbit_alt_km: f64,
    /// Orbit: inclination, degrees.
    #[serde(default)]
    pub inclination_deg: f64,
}

// ── Body ────────────────────────────────────────────────────────────────

/// How a platform is hurt: what a round must do to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vulnerability {
    Armoured,
    Light,
    Soft,
    Personnel,
    Aircraft,
    Ship,
    Structure,
    Missile,
    Satellite,
}

/// Armour by aspect, mm RHA equivalent.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Aspects {
    #[serde(default)]
    pub front: f64,
    #[serde(default)]
    pub side: f64,
    #[serde(default)]
    pub rear: f64,
    #[serde(default)]
    pub top: f64,
}

/// The aspect a round arrives from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Aspect {
    Front,
    Side,
    Rear,
    Top,
}

impl Aspects {
    pub fn get(&self, a: Aspect) -> f64 {
        match a {
            Aspect::Front => self.front,
            Aspect::Side => self.side,
            Aspect::Rear => self.rear,
            Aspect::Top => self.top,
        }
    }
    pub fn max(&self) -> f64 {
        self.front.max(self.side).max(self.rear).max(self.top)
    }
}

/// One zone's protection (hull, turret) against kinetic and chemical energy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ZoneArmour {
    pub ke: Aspects,
    pub ce: Aspects,
    /// Share of hits that land on this zone, 0–1.
    #[serde(default)]
    pub hit_share: f64,
}

/// Armour, mm RHA, against kinetic-energy (KE) and chemical-energy (CE)
/// penetrators, by aspect; optionally by zone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Armour {
    #[serde(default)]
    pub ke: Aspects,
    #[serde(default)]
    pub ce: Aspects,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub zones: BTreeMap<String, ZoneArmour>,
}

/// An active protection system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Aps {
    /// "hard" kill (Trophy-like) or "soft" (jammers, smoke).
    pub kind: String,
    /// Chance it defeats one incoming ATGM or RPG it engages, 0–1.
    pub pk: f64,
    /// Engagements before it needs reloading.
    pub charges: u32,
}

/// A sub-platform: a vehicle, aircraft, person or ship within the element.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubPlatform {
    pub name: String,
    pub count: u32,
    /// People per sub-platform.
    pub crew: u32,
    /// Components that can be damaged separately (§5.1: mobility, firepower, sensors, comms, crew).
    #[serde(default)]
    pub components: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Body {
    /// Platforms in the element: 4 tanks, 2 aircraft, 1 ship.
    pub count: u32,
    pub vulnerability: Vulnerability,
    /// People per platform.
    pub crew: u32,
    /// Ships, structures: standard 200 kg-warhead hits it takes before it is out of action.
    #[serde(default = "one")]
    pub staying_power: f64,
    #[serde(default)]
    pub mass_t: f64,
    #[serde(default)]
    pub length_m: f64,
    #[serde(default)]
    pub width_m: f64,
    #[serde(default)]
    pub height_m: f64,
    #[serde(default)]
    pub armour: Armour,
    /// Explosive reactive armour fitted (only tandem warheads care).
    #[serde(default)]
    pub era: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aps: Option<Aps>,
    /// Derived as one entry of `count` platforms when the record gives none.
    #[serde(default)]
    pub sub_platforms: Vec<SubPlatform>,
}

fn one() -> f64 {
    1.0
}

// ── Signatures ──────────────────────────────────────────────────────────

/// Apparent visual contrast (0–1) by posture.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PostureContrast {
    pub moving: f64,
    pub halted: f64,
    pub hull_down: f64,
    pub dug_in: f64,
    pub camouflaged: f64,
}

/// Posture for signature look-ups.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Posture {
    Moving,
    Halted,
    HullDown,
    DugIn,
    Camouflaged,
}

impl PostureContrast {
    pub fn get(&self, p: Posture) -> f64 {
        match p {
            Posture::Moving => self.moving,
            Posture::Halted => self.halted,
            Posture::HullDown => self.hull_down,
            Posture::DugIn => self.dug_in,
            Posture::Camouflaged => self.camouflaged,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Visual {
    /// Critical dimension (√(height × width) of the presented area), m: what
    /// resolved cycles are counted across (ACQUIRE [P22]).
    pub critical_dim_m: f64,
    pub contrast: PostureContrast,
}

/// Thermal state for the IR signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThermalState {
    Cold,
    EngineOn,
    Moved,
    Fired,
}

/// Apparent temperature difference against the background, K, by state.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThermalStates {
    pub cold: f64,
    pub engine_on: f64,
    pub moved: f64,
    pub fired: f64,
}

impl ThermalStates {
    pub fn get(&self, s: ThermalState) -> f64 {
        match s {
            ThermalState::Cold => self.cold,
            ThermalState::EngineOn => self.engine_on,
            ThermalState::Moved => self.moved,
            ThermalState::Fired => self.fired,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Ir {
    #[serde(rename = "deltaK")]
    pub delta_k: ThermalStates,
}

/// Radar cross-section by aspect, m².
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AspectRcs {
    pub front: f64,
    pub side: f64,
    pub rear: f64,
    pub top: f64,
}

impl AspectRcs {
    pub fn get(&self, a: Aspect) -> f64 {
        match a {
            Aspect::Front => self.front,
            Aspect::Side => self.side,
            Aspect::Rear => self.rear,
            Aspect::Top => self.top,
        }
    }
}

/// A level at a speed: a table row for the acoustic signature.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedLevel {
    pub kmh: f64,
    pub db: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Acoustic {
    /// Underwater radiated noise, dB re 1 µPa at 1 m, by speed (sonar).
    #[serde(default)]
    pub water_db: Vec<SpeedLevel>,
    /// Airborne noise, dB(A) at 1 m, by speed (aural and acoustic detection).
    #[serde(default)]
    pub air_dba: Vec<SpeedLevel>,
}

/// Linear interpolation in a speed table, clamped at its ends; `None` if empty.
pub fn level_at(table: &[SpeedLevel], kmh: f64) -> Option<f64> {
    let first = table.first()?;
    if kmh <= first.kmh {
        return Some(first.db);
    }
    for w in table.windows(2) {
        let (a, b) = (w[0], w[1]);
        if kmh <= b.kmh {
            let t = if b.kmh > a.kmh { (kmh - a.kmh) / (b.kmh - a.kmh) } else { 1.0 };
            return Some(a.db + t * (b.db - a.db));
        }
    }
    table.last().map(|l| l.db)
}

impl Acoustic {
    pub fn water_db_at(&self, kmh: f64) -> Option<f64> {
        level_at(&self.water_db, kmh)
    }
    pub fn air_dba_at(&self, kmh: f64) -> Option<f64> {
        level_at(&self.air_dba, kmh)
    }
}

/// The TypeScript engine's dimensionless multipliers (1 = a tank in the
/// open), carried over so the ported land detection model can be compared.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SignatureIndex {
    pub visual: f64,
    pub ir: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signatures {
    pub visual: Visual,
    pub ir: Ir,
    /// RCS by radar band id ("x", "s", "l"…), by aspect. "x" is always present.
    pub rcs: BTreeMap<String, AspectRcs>,
    #[serde(default)]
    pub acoustic: Acoustic,
    /// A decoy: looks like the real thing to sensors.
    #[serde(default)]
    pub decoy: bool,
    #[serde(default)]
    pub index: SignatureIndex,
}

impl Signatures {
    /// RCS in a band at an aspect, falling back to X band when the band is not tabulated.
    pub fn rcs_m2(&self, band: &str, aspect: Aspect) -> f64 {
        self.rcs.get(band).or_else(|| self.rcs.get("x")).map(|r| r.get(aspect)).unwrap_or(0.0)
    }
}

// ── Sensors ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SensorKind {
    Eye,
    Optic,
    ImageIntensifier,
    Thermal,
    Radar,
    Esm,
    Rwr,
    SonarPassive,
    SonarActive,
    Acoustic,
    Sar,
    EoSat,
    /// A turret or gimbal EO/IR camera (drones, aircraft).
    Eo,
    /// Counter-battery / weapon-locating radar.
    Cbr,
    Irst,
    MissileWarning,
}

impl SensorKind {
    /// Does it transmit (an emitter while on)?
    pub fn is_active(self) -> bool {
        matches!(self, SensorKind::Radar | SensorKind::SonarActive | SensorKind::Sar | SensorKind::Cbr)
    }
    /// Is it an electro-optical/IR imager that the ACQUIRE model applies to?
    pub fn is_imaging_eo(self) -> bool {
        matches!(
            self,
            SensorKind::Eye | SensorKind::Optic | SensorKind::ImageIntensifier | SensorKind::Thermal | SensorKind::Eo | SensorKind::EoSat | SensorKind::Irst
        )
    }
}

/// How far up the classification ladder a sensor can get.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Classification {
    Track,
    Class,
    Type,
    Identity,
}

/// Johnson/ACQUIRE N50 cycles for each task [P22].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct N50 {
    pub detect: f64,
    pub recognise: f64,
    pub identify: f64,
}

/// An imager's resolving power.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Optics {
    /// Limiting resolution, cycles per milliradian (narrow field).
    pub resolution_cyc_per_mrad: f64,
    #[serde(default)]
    pub aperture_mm: f64,
    /// Narrow field of view, degrees.
    #[serde(default)]
    pub fov_deg: f64,
    pub n50: N50,
}

/// Radar-equation parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RadarParams {
    /// Peak transmit power, W.
    pub pt_w: f64,
    /// Antenna gain, dB.
    pub gain_db: f64,
    pub freq_hz: f64,
    /// Receiver noise bandwidth, Hz.
    pub bandwidth_hz: f64,
    pub noise_figure_db: f64,
    /// System losses, dB.
    pub losses_db: f64,
    /// Pulses integrated per look.
    #[serde(default = "one")]
    pub pulses_integrated: f64,
    /// Detection threshold, single-look SNR, dB.
    pub snr_required_db: f64,
    #[serde(default)]
    pub beamwidth_deg: f64,
    /// Doppler/MTI processing: rejects clutter and slow targets.
    #[serde(default)]
    pub mti: bool,
}

/// Sonar-equation parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SonarParams {
    /// Directivity index, dB.
    pub di_db: f64,
    /// Detection threshold, dB.
    pub dt_db: f64,
    pub freq_hz: f64,
    /// Active: source level, dB re 1 µPa at 1 m.
    #[serde(default)]
    pub source_level_db: f64,
    /// Self-noise at working speed, dB re 1 µPa (spectrum level in band).
    #[serde(default)]
    pub self_noise_db: f64,
    #[serde(default)]
    pub towed: bool,
}

/// Electronic-support receiver parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EsmParams {
    /// Weakest signal it detects, dBm at the receiver.
    pub sensitivity_dbm: f64,
    pub freq_min_hz: f64,
    pub freq_max_hz: f64,
    /// Bearing accuracy, degrees RMS.
    pub df_rms_deg: f64,
    /// Antenna gain, dBi.
    #[serde(default)]
    pub gain_dbi: f64,
}

/// Imaging radar or satellite parameters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Imaging {
    /// Ground sample distance, m.
    pub gsd_m: f64,
    pub swath_km: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sensor {
    pub id: String,
    pub kind: SensorKind,
    /// A band id from the bands table ("visible", "lwir", "x", "acoustic"…).
    pub band: String,
    #[serde(default)]
    pub range_min_m: f64,
    /// Against the standard target of its kind (TS `rangeM`).
    pub range_max_m: f64,
    /// Field of regard in azimuth, degrees.
    pub for_deg: f64,
    /// How often it looks again, s.
    pub scan_s: f64,
    pub classifies: Classification,
    pub sees_air: bool,
    /// Transmits while on.
    pub active: bool,
    /// Antenna, mast or eye height above the platform's reference, m.
    pub height_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optics: Option<Optics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radar: Option<RadarParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sonar: Option<SonarParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub esm: Option<EsmParams>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imaging: Option<Imaging>,
}

// ── Weapons ─────────────────────────────────────────────────────────────

/// The kind of weapon a mount is (TS `WeaponKind`, plus `vls`, `smallArms`, `cannon`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WeaponKind {
    DirectFire,
    Cannon,
    SmallArms,
    LightAt,
    Atgm,
    Mortar,
    Howitzer,
    Rocket,
    SamLong,
    SamMedium,
    SamShort,
    GunAd,
    Aam,
    Agm,
    GlideBomb,
    Bomb,
    Arm,
    Ascm,
    Cruise,
    Ballistic,
    Torpedo,
    Ciws,
    Fpv,
    Loitering,
    Owa,
    InterceptorDrone,
    Shotgun,
    DroneDrop,
    /// Vertical launch cells holding a mix of missiles.
    Vls,
}

/// The quality of a mount's fire control.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FireControl {
    /// 0 (none) – 1 (modern stabilised, laser-ranged, ballistic computer).
    pub quality: f64,
    #[serde(default)]
    pub stabilised: bool,
    #[serde(default)]
    pub rangefinder: bool,
    /// Time to lay on a new target, s.
    #[serde(default)]
    pub lay_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    /// Stable mount id (the TS weapon id where it carries over: `tankGun`, `samLong`).
    pub id: String,
    pub kind: WeaponKind,
    /// Munition ids it fires; the first is the default.
    pub munitions: Vec<String>,
    pub fire_control: FireControl,
    /// Rounds per engagement (a salvo, a volley).
    pub salvo: u32,
    /// Rounds ready on the mount before reloading from stores (cells for a VLS).
    pub magazine: u32,
    /// Time between salvoes, s.
    pub reload_s: f64,
    /// Time from a halt or a cue to the first shot (emplace, warm up, unfold), s.
    #[serde(default)]
    pub ready_s: f64,
    /// Traverse, degrees (360 for a turret).
    #[serde(default)]
    pub arc_deg: f64,
    /// Sustained rate of fire, rounds per minute.
    #[serde(default)]
    pub rof_rpm: f64,
}

// ── Radios and emitters ─────────────────────────────────────────────────

/// A way of passing messages (W4 bearers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Bearer {
    Hf,
    Vhf,
    Uhf,
    SatcomMil,
    SatcomLeo,
    /// Tactical data link (Link 16-like) [P6]; the TS `datalink`.
    #[serde(alias = "datalink")]
    Tdl,
    Mesh,
    Wire,
    Fibre,
    Courier,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Radio {
    pub id: String,
    pub bearer: Bearer,
    /// A band id from the bands table.
    pub band: String,
    /// Centre frequency, Hz (0 for wire, fibre and courier).
    #[serde(default)]
    pub freq_hz: f64,
    pub power_w: f64,
    pub antenna_height_m: f64,
    #[serde(default)]
    pub antenna_gain_dbi: f64,
    pub data_rate_bps: f64,
    /// Time added by encryption and key handling per message, s.
    pub crypto_delay_s: f64,
    /// Fixed latency of the bearer (satellite hops, relays), s.
    #[serde(default)]
    pub latency_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmitterKind {
    Radar,
    Radio,
    Datalink,
    Jammer,
    Beacon,
    DroneLink,
    Satcom,
    Gnss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternKind {
    Omni,
    Sector,
    Rotating,
    Pencil,
}

/// An antenna pattern, as far as intercept and jamming need it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pattern {
    pub kind: PatternKind,
    /// Main-lobe width, degrees (360 for omni).
    pub beamwidth_deg: f64,
    /// Side-lobe level below the main lobe, dB (positive).
    #[serde(default)]
    pub sidelobe_db: f64,
    /// Rotating or scanning: one revolution, s.
    #[serde(default)]
    pub scan_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Emitter {
    pub id: String,
    pub kind: EmitterKind,
    pub band: String,
    pub freq_hz: f64,
    pub bandwidth_hz: f64,
    /// Effective radiated power, W.
    pub erp_w: f64,
    /// Fraction of time on the air while switched on.
    pub duty_cycle: f64,
    pub pattern: Pattern,
    /// Jammers: the bands it denies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jams: Vec<String>,
    /// Jammers: radius it denies against a typical victim link, m (TS figure; the
    /// physics model computes J/S from `erpW`).
    #[serde(default)]
    pub jam_radius_m: f64,
    /// Jammers: the radius by band, where it differs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub jam_radii_m: BTreeMap<String, f64>,
}

// ── Stores, carried, crew ───────────────────────────────────────────────

/// Supplies held as stores (beyond the fuel in the platforms' own tanks).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stores {
    /// Bulk fuel carried or stocked, litres per element.
    #[serde(default)]
    pub fuel_l: f64,
    /// Ammunition by munition id, rounds per element.
    #[serde(default)]
    pub ammo: BTreeMap<String, f64>,
    /// Batteries for radios, optics and drones, Wh per element.
    #[serde(default)]
    pub battery_wh: f64,
    #[serde(default)]
    pub water_l: f64,
    /// Ration packs (person-days).
    #[serde(default)]
    pub food_rations: f64,
    /// Casualty treatments.
    #[serde(default)]
    pub medical: f64,
    /// Repairs.
    #[serde(default)]
    pub spares: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedSections {
    pub archetype: String,
    pub sections: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CarriedCount {
    pub archetype: String,
    pub count: u32,
}

/// Troops, drones and aircraft carried, and cargo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Carried {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismounts: Option<CarriedSections>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drones: Option<CarriedCount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aircraft: Option<CarriedCount>,
    #[serde(default)]
    pub cargo_t: f64,
}

impl Carried {
    /// Every archetype it names.
    pub fn archetypes(&self) -> Vec<&str> {
        let mut v = Vec::new();
        if let Some(d) = &self.dismounts {
            v.push(d.archetype.as_str());
        }
        if let Some(d) = &self.drones {
            v.push(d.archetype.as_str());
        }
        if let Some(d) = &self.aircraft {
            v.push(d.archetype.as_str());
        }
        v
    }
}

/// Crew defaults for a fresh element of this type (§8.10 starts from these).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrewDefaults {
    /// Troop quality, 1 (conscript) – 8 (elite), as forceList `TROOP_QUALITY`.
    pub quality: u8,
    /// Time from a decision to its first effect, s (the Question's reaction time).
    pub reaction_s: f64,
    #[serde(default)]
    pub night_trained: bool,
}

// ── The record ──────────────────────────────────────────────────────────

/// A resolved platform record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    /// "generic/mbt", "var_11_default". Real systems inherit with `parent`.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The class whose defaults filled this record.
    pub class: String,
    pub domain: Domain,
    pub role: String,
    pub sidc: Sidc,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echelon: Option<Echelon>,
    /// Behaviour template id (`core/armour`).
    pub template: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_by: Option<Planner>,
    pub mover: Mover,
    pub body: Body,
    pub signatures: Signatures,
    #[serde(default)]
    pub sensors: Vec<Sensor>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub radios: Vec<Radio>,
    #[serde(default)]
    pub emitters: Vec<Emitter>,
    #[serde(default)]
    pub stores: Stores,
    #[serde(default)]
    pub carried: Carried,
    pub crew: CrewDefaults,
    /// One platform, US dollars (cost-exchange report).
    pub cost_usd: f64,
    /// How the guesses were made.
    pub rationale: String,
    /// The record's main source note ([P..]/[R..], a document, or "synthetic").
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub provenance: Provenance,
    /// Field path → where it came from.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sources: BTreeMap<String, FieldSource>,
    /// Other ids that name this record (TS force-list keys).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

impl Platform {
    pub fn mount(&self, id: &str) -> Option<&Mount> {
        self.mounts.iter().find(|m| m.id == id)
    }
    pub fn sensors_of(&self, kind: SensorKind) -> impl Iterator<Item = &Sensor> {
        self.sensors.iter().filter(move |s| s.kind == kind)
    }
    /// Total people in the element.
    pub fn people(&self) -> u32 {
        if self.body.sub_platforms.is_empty() {
            self.body.count * self.body.crew
        } else {
            self.body.sub_platforms.iter().map(|s| s.count * s.crew).sum()
        }
    }
    /// Fuel the element holds at start: own tanks × count plus bulk stores.
    pub fn fuel_at_start_l(&self) -> f64 {
        self.mover.fuel_capacity_l * self.body.count as f64 + self.stores.fuel_l
    }
    pub fn emits_when_on(&self) -> bool {
        !self.emitters.is_empty() || self.sensors.iter().any(|s| s.active)
    }
}

/// A frequency band (the bands table, §7.5).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Band {
    pub id: String,
    pub name: String,
    /// "radio", "radar", "optical", "acoustic" or "none".
    pub kind: String,
    pub f_min_hz: f64,
    pub f_max_hz: f64,
    #[serde(default)]
    pub source: String,
}

impl Band {
    pub fn contains(&self, f: f64) -> bool {
        f >= self.f_min_hz && f <= self.f_max_hz
    }
    pub fn centre_hz(&self) -> f64 {
        if self.f_min_hz > 0.0 {
            (self.f_min_hz * self.f_max_hz).sqrt()
        } else {
            self.f_max_hz / 2.0
        }
    }
}
