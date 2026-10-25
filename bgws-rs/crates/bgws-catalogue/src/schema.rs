//! The platform record schema (§5.1, §14.1). A platform is built from parts,
//! AFSIM-style: a mover, a body, signatures, sensors, weapon mounts, radios,
//! emitters, stores and what it carries. Every part carries the physics its
//! models need; defaults per class keep a record short.
//!
//! The catalogue is synthetic and open-source-anchored. No record is the
//! real performance of a real system.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum PlatformDomain {
    Land,
    Surface,
    Subsurface,
    Air,
    Space,
    Node,
    Munition,
}

/// How a platform moves.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MoverKind {
    Tracked,
    Wheeled,
    Foot,
    Rotary,
    FixedWing,
    Multirotor,
    /// A guided or unguided munition flown as an entity: 3-DoF flyout.
    Missile,
    /// A ballistic missile: boost, midcourse arc, re-entry.
    Ballistic,
    /// An artillery or mortar shell: a ballistic trajectory, no thrust.
    Shell,
    Surface,
    Sub,
    Orbit,
    Static,
}

impl MoverKind {
    pub fn is_ground(self) -> bool {
        matches!(self, MoverKind::Tracked | MoverKind::Wheeled | MoverKind::Foot)
    }
    pub fn flies(self) -> bool {
        matches!(self, MoverKind::Rotary | MoverKind::FixedWing | MoverKind::Multirotor | MoverKind::Missile | MoverKind::Ballistic | MoverKind::Shell)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MoverSpec {
    pub kind: Option<MoverKind>,
    /// Top speed on a good road or in level flight, m/s.
    pub max_ms: f64,
    /// Speed across good, dry, open going, m/s (ground movers).
    pub cross_ms: f64,
    /// Vehicle cone index (NRMM-style): the soil strength it needs to keep moving. Lower is better.
    pub vci: f64,
    pub max_slope_deg: f64,
    pub ford_m: f64,
    /// Trench/gap it can cross unaided, m.
    pub gap_m: f64,
    /// Bridge load class needed (MLC).
    pub mlc: f64,
    /// Fuel: litres per km on a road; multiplied by going resistance off it.
    pub fuel_l_per_km: f64,
    /// Fuel idling or hovering, litres per hour.
    pub fuel_l_per_h_idle: f64,
    // ── air ──
    pub cruise_ms: f64,
    pub ceiling_m: f64,
    pub climb_ms: f64,
    /// Sustained turn limit, g.
    pub max_g: f64,
    /// Fuel or battery endurance at cruise, s (drones, helicopters).
    pub endurance_s: f64,
    /// Fuel burn at cruise, kg/h (fixed wing).
    pub fuel_kg_per_h: f64,
    /// Highest wind it can work in, m/s (small drones).
    pub max_wind_ms: f64,
    // ── missiles and shells ──
    pub mass_kg: f64,
    pub burnout_mass_kg: f64,
    pub thrust_n: f64,
    pub burn_s: f64,
    /// Sustainer (cruise missiles, drones): thrust that holds cruise speed after the boost.
    pub sustain: bool,
    pub cd: f64,
    pub area_m2: f64,
    pub cruise_alt_m: f64,
    pub max_range_m: f64,
    // ── ships and submarines ──
    pub max_sea_state: f64,
    /// Quiet speed (submarines), m/s.
    pub quiet_ms: f64,
    pub max_depth_m: f64,
}

/// Armour, millimetres of rolled homogeneous armour equivalent, by aspect.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Armour {
    pub front_ke: f64,
    pub front_ce: f64,
    pub side_ke: f64,
    pub side_ce: f64,
    pub rear: f64,
    pub top: f64,
    /// Explosive reactive armour: multiplier on shaped-charge penetration it defeats (0–1 of pen removed).
    pub era: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApsSpec {
    pub charges: u32,
    /// Chance to defeat an incoming guided round it engages.
    pub p_defeat: f64,
    /// Can it defeat kinetic rounds too?
    pub vs_ke: bool,
    /// Does it engage top attack (drones)?
    pub vs_top: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vulnerability {
    /// People in the open.
    Personnel,
    Soft,
    LightArmour,
    HeavyArmour,
    Aircraft,
    Ship,
    Structure,
    Munition,
    Satellite,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BodySpec {
    /// How many sub-platforms it is: vehicles in a platoon, aircraft in a flight, people in a team.
    pub count: u32,
    /// People per sub-platform.
    pub crew_each: u32,
    pub length_m: f64,
    pub width_m: f64,
    pub height_m: f64,
    pub vulnerability: Vulnerability,
    pub armour: Armour,
    pub aps: Option<ApsSpec>,
    /// Ships and structures: damage points before mission kill and before loss.
    pub mission_kill_pts: f64,
    pub loss_pts: f64,
    /// Displacement, tonnes (ships) — sets the signature and the damage scaling.
    pub tonnes: f64,
}

impl Default for BodySpec {
    fn default() -> Self {
        BodySpec {
            count: 1,
            crew_each: 1,
            length_m: 5.0,
            width_m: 2.5,
            height_m: 2.0,
            vulnerability: Vulnerability::Soft,
            armour: Armour::default(),
            aps: None,
            mission_kill_pts: 0.0,
            loss_pts: 0.0,
            tonnes: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SignatureSpec {
    /// Apparent contrast against a typical background, 0–1, before camouflage and posture.
    pub visual_contrast: f64,
    /// Temperature excess over background with engines running, K.
    pub ir_running_k: f64,
    /// Temperature excess when cold-soaked (off for hours), K.
    pub ir_cold_k: f64,
    /// Radar cross-section, m², by aspect: front, side, rear, top.
    pub rcs_m2: [f64; 4],
    /// Radiated noise source level at quiet speed, dB re 1 µPa @ 1 m (ships, subs) or dB SPL @ 1 m in air.
    pub acoustic_db: f64,
    /// Source level rise per m/s above quiet speed, dB.
    pub acoustic_db_per_ms: f64,
    /// Raises dust when moving on dry ground.
    pub dust: bool,
    /// Magnetic signature, relative (mines), 0–1.
    pub magnetic: f64,
}

impl Default for SignatureSpec {
    fn default() -> Self {
        SignatureSpec {
            visual_contrast: 0.5,
            ir_running_k: 10.0,
            ir_cold_k: 0.5,
            rcs_m2: [5.0, 10.0, 5.0, 10.0],
            acoustic_db: 80.0,
            acoustic_db_per_ms: 1.0,
            dust: false,
            magnetic: 0.0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum SensorKind {
    /// The naked eye, with binoculars.
    Eye,
    /// Day sights and cameras: visible band, ACQUIRE/TTP.
    Optic,
    /// Image intensifier (night vision): needs some light.
    Ii,
    /// Thermal imager (LWIR/MWIR).
    Thermal,
    /// IR search and track (air).
    Irst,
    Radar,
    /// Weapon-locating radar: sees shells and rockets in flight.
    Wlr,
    /// Electronic support: detects and direction-finds emitters.
    Esm,
    /// Radar warning receiver.
    Rwr,
    SonarPassive,
    SonarActive,
    /// Acoustic sensors in air: gunfire and drone detection.
    Acoustic,
    /// Synthetic aperture radar imaging (satellites, aircraft).
    Sar,
    /// RF detector for drone links.
    RfDetect,
}

impl SensorKind {
    pub fn is_imaging(self) -> bool {
        matches!(self, SensorKind::Eye | SensorKind::Optic | SensorKind::Ii | SensorKind::Thermal | SensorKind::Irst)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SensorSpec {
    pub kind: SensorKind,
    /// Instantaneous field of view, degrees (imaging) or beamwidth (radar).
    pub fov_deg: f64,
    /// Field of regard, degrees: 360 for an all-round sensor.
    pub regard_deg: f64,
    /// Angular resolution, milliradians per resolvable cycle (imaging).
    pub mrad_per_cycle: f64,
    /// Hard range limit, m (display, processing).
    pub max_range_m: f64,
    /// Mount height above the platform's reference, m (masts, turret tops).
    pub mast_m: f64,
    // ── radar ──
    pub peak_w: f64,
    pub gain_db: f64,
    pub freq_ghz: f64,
    pub bandwidth_mhz: f64,
    pub noise_fig_db: f64,
    pub losses_db: f64,
    /// SNR needed to declare a detection on one scan, dB.
    pub snr_detect_db: f64,
    /// Revisit time per scan, s.
    pub scan_s: f64,
    /// Moving-target indication: targets slower than this (radial, m/s) are lost in clutter.
    pub mti_min_ms: f64,
    /// Can track this many targets at once (fire-control radars).
    pub tracks: u32,
    /// Duty cycle: average/peak power, for ESM detectability.
    pub duty: f64,
    // ── ESM / RWR / RF detector ──
    pub sensitivity_dbm: f64,
    pub fmin_ghz: f64,
    pub fmax_ghz: f64,
    /// Direction-finding error, degrees RMS.
    pub df_rms_deg: f64,
    // ── sonar and acoustic ──
    pub di_db: f64,
    pub dt_db: f64,
    pub band_hz: f64,
    pub source_db: f64,
    // ── imaging satellites and SAR ──
    pub gsd_m: f64,
    pub swath_km: f64,
}

impl Default for SensorSpec {
    fn default() -> Self {
        SensorSpec {
            kind: SensorKind::Eye,
            fov_deg: 60.0,
            regard_deg: 360.0,
            mrad_per_cycle: 0.6,
            max_range_m: 10_000.0,
            mast_m: 0.0,
            peak_w: 0.0,
            gain_db: 0.0,
            freq_ghz: 10.0,
            bandwidth_mhz: 1.0,
            noise_fig_db: 4.0,
            losses_db: 6.0,
            snr_detect_db: 13.0,
            scan_s: 4.0,
            mti_min_ms: 0.0,
            tracks: 0,
            duty: 0.01,
            sensitivity_dbm: -70.0,
            fmin_ghz: 0.02,
            fmax_ghz: 18.0,
            df_rms_deg: 3.0,
            di_db: 10.0,
            dt_db: 10.0,
            band_hz: 1000.0,
            source_db: 220.0,
            gsd_m: 1.0,
            swath_km: 10.0,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum WeaponKind {
    /// Tank and autocannon guns: direct fire.
    Gun,
    /// Machine guns and rifles: area fire at people and soft targets.
    SmallArms,
    Atgm,
    Mortar,
    Howitzer,
    Rocket,
    SamShort,
    SamMedium,
    SamLong,
    /// Close-in weapon systems and AA guns.
    AaGun,
    Aam,
    /// Air-to-surface missiles and guided bombs.
    Asm,
    Bomb,
    Cruise,
    Ballistic,
    AntiShip,
    Torpedo,
    /// FPV and bomber drones as a weapon: the drone is the munition.
    Drone,
    /// Interceptor drones against drones.
    Interceptor,
    /// Lasers and high-power microwave (counter-drone).
    DirectedEnergy,
    Mine,
}

impl WeaponKind {
    pub fn is_indirect(self) -> bool {
        matches!(self, WeaponKind::Mortar | WeaponKind::Howitzer | WeaponKind::Rocket)
    }
    pub fn is_air_defence(self) -> bool {
        matches!(self, WeaponKind::SamShort | WeaponKind::SamMedium | WeaponKind::SamLong | WeaponKind::AaGun | WeaponKind::Interceptor | WeaponKind::DirectedEnergy)
    }
    pub fn flies_as_entity(self) -> bool {
        matches!(
            self,
            WeaponKind::Atgm
                | WeaponKind::SamShort
                | WeaponKind::SamMedium
                | WeaponKind::SamLong
                | WeaponKind::Aam
                | WeaponKind::Asm
                | WeaponKind::Cruise
                | WeaponKind::Ballistic
                | WeaponKind::AntiShip
                | WeaponKind::Torpedo
                | WeaponKind::Drone
                | WeaponKind::Interceptor
                | WeaponKind::Rocket
                | WeaponKind::Howitzer
                | WeaponKind::Mortar
                | WeaponKind::Bomb
        )
    }
}

/// Fire control: the error budget of an aimed shot (§8.8).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FireControl {
    /// Range estimation error, fraction of range (1σ): laser rangefinders ≈ 0.002, eye ≈ 0.1.
    pub range_err_frac: f64,
    /// Fixed aim bias per engagement (boresight, cant, crosswind), mrad 1σ.
    pub bias_mrad: f64,
    /// Round-to-round dispersion, mrad 1σ.
    pub dispersion_mrad: f64,
    /// Stabilised: fires on the move without extra dispersion.
    pub stabilised: bool,
    /// Lead error against movers, fraction of the target's travel during time of flight (1σ).
    pub lead_err_frac: f64,
}

impl Default for FireControl {
    fn default() -> Self {
        FireControl { range_err_frac: 0.05, bias_mrad: 0.5, dispersion_mrad: 0.4, stabilised: false, lead_err_frac: 0.3 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WeaponSpec {
    pub id: String,
    pub name: String,
    pub kind: WeaponKind,
    /// Munitions it can fire, best first.
    pub munitions: Vec<String>,
    /// Rounds per minute, sustained.
    pub rpm: f64,
    /// Rounds in a burst or volley.
    pub burst: u32,
    pub min_range_m: f64,
    pub max_range_m: f64,
    /// Muzzle velocity, m/s (guns, shells).
    pub muzzle_ms: f64,
    pub fire_control: FireControl,
    /// Seconds to reload a launcher or a magazine from carried stock.
    pub reload_s: f64,
    /// Time to bring into action / out of action (artillery, SAMs), s.
    pub emplace_s: f64,
    /// Needs its own or a linked fire-control radar track (SAMs).
    pub needs_track: bool,
    /// Can fire on the move.
    pub on_move: bool,
    /// Engages air targets.
    pub anti_air: bool,
}

impl Default for WeaponSpec {
    fn default() -> Self {
        WeaponSpec {
            id: String::new(),
            name: String::new(),
            kind: WeaponKind::Gun,
            munitions: vec![],
            rpm: 6.0,
            burst: 1,
            min_range_m: 0.0,
            max_range_m: 3000.0,
            muzzle_ms: 1600.0,
            fire_control: FireControl::default(),
            reload_s: 0.0,
            emplace_s: 0.0,
            needs_track: false,
            on_move: true,
            anti_air: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Warhead {
    /// Kinetic energy penetrator.
    Ke,
    /// Shaped charge.
    Heat,
    /// Tandem shaped charge (defeats ERA).
    Tandem,
    /// High explosive, fragmentation.
    He,
    /// Ball ammunition.
    Ball,
    /// Proximity-fuzed blast-fragmentation (SAMs, AAMs).
    BlastFrag,
    /// Hit-to-kill (some interceptors).
    HitToKill,
    /// Explosively formed penetrator / top attack.
    Efp,
    /// Cluster / DPICM.
    Submunition,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Guidance {
    None,
    /// Saclos / wire / radio command from the launcher.
    Command,
    /// Fibre-optic: immune to RF jamming.
    Fibre,
    /// Radio-controlled by an operator (FPV): needs the link.
    RfOperator,
    LaserSemiActive,
    /// Imaging IR or EO seeker, fire-and-forget.
    Imaging,
    ActiveRadar,
    SemiActiveRadar,
    /// GNSS-aided inertial: its CEP depends on the navigation state (§8.7).
    GnssIns,
    /// Inertial only.
    Ins,
    /// Anti-radiation: homes on emitters.
    AntiRadiation,
    /// Acoustic homing (torpedoes).
    Acoustic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MunitionSpec {
    pub id: String,
    pub name: String,
    pub warhead: Warhead,
    pub guidance: Guidance,
    /// Penetration at the muzzle or on impact, mm RHAe.
    pub pen_mm: f64,
    /// KE penetration lost per km of range, mm.
    pub pen_loss_per_km: f64,
    /// Attacks the top (EFP, drones, top-attack ATGMs).
    pub top_attack: bool,
    /// HE: lethal area against standing people in the open, m² (Carleton scale).
    pub lethal_area_m2: f64,
    /// Explosive mass, kg (blast against structures and ships).
    pub he_kg: f64,
    /// Proximity-fuze lethal radius against aircraft and missiles, m.
    pub prox_radius_m: f64,
    /// Seeker or guidance accuracy with good navigation, m (CEP for GNSS weapons, miss distance for homing).
    pub cep_m: f64,
    /// CEP growth per second of flight on inertial only, m/s.
    pub ins_drift_mps: f64,
    /// For munitions flown as entities: the record that flies (a missile or drone record id).
    pub flies_as: Option<String>,
    /// Chance the fuze and warhead work.
    pub reliability: f64,
    pub cost_usd: f64,
    /// Damage points against ships and structures per hit.
    pub damage_pts: f64,
}

impl Default for MunitionSpec {
    fn default() -> Self {
        MunitionSpec {
            id: String::new(),
            name: String::new(),
            warhead: Warhead::He,
            guidance: Guidance::None,
            pen_mm: 0.0,
            pen_loss_per_km: 0.0,
            top_attack: false,
            lethal_area_m2: 0.0,
            he_kg: 0.0,
            prox_radius_m: 0.0,
            cep_m: 0.0,
            ins_drift_mps: 0.0,
            flies_as: None,
            reliability: 0.95,
            cost_usd: 0.0,
            damage_pts: 0.0,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MountSpec {
    pub weapon: String,
    /// Launchers or barrels.
    pub launchers: u32,
    /// Rounds ready to fire per launcher (tubes, ready racks, VLS cells).
    pub ready: u32,
    /// Rounds carried in reserve, by munition id.
    pub reserve: BTreeMap<String, u32>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum Bearer {
    /// Combat-net radio, VHF (30–88 MHz).
    Vhf,
    Uhf,
    Hf,
    /// Geostationary satcom.
    SatcomGeo,
    /// Low-orbit satcom (Starlink-like).
    SatcomLeo,
    /// TDMA tactical data link (Link 16-like).
    Datalink,
    /// Line-of-sight mesh radio.
    Mesh,
    /// Field wire / fibre between fixed posts.
    Wire,
    /// Cellular network (civil).
    Cellular,
    Courier,
    /// The drone's own control and video link.
    DroneLink,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RadioSpec {
    pub bearer: Bearer,
    pub freq_mhz: f64,
    pub power_w: f64,
    pub gain_db: f64,
    /// Antenna height above ground, m (a mast raises it at the cost of signature and time).
    pub antenna_m: f64,
    pub bandwidth_khz: f64,
    pub sensitivity_dbm: f64,
    /// Data rate at good SNR, kbit/s.
    pub rate_kbps: f64,
    /// Frequency hopping: processing gain against jamming, dB.
    pub hop_gain_db: f64,
    /// Encryption and authentication delay per message, s.
    pub crypto_s: f64,
}

impl Default for RadioSpec {
    fn default() -> Self {
        RadioSpec {
            bearer: Bearer::Vhf,
            freq_mhz: 60.0,
            power_w: 5.0,
            gain_db: 0.0,
            antenna_m: 2.5,
            bandwidth_khz: 25.0,
            sensitivity_dbm: -110.0,
            rate_kbps: 16.0,
            hop_gain_db: 0.0,
            crypto_s: 0.5,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EmitterKind {
    /// Barrage or spot noise jammer against comms and drone links.
    CommsJammer,
    /// GNSS jammer.
    GnssJammer,
    /// GNSS spoofer.
    GnssSpoofer,
    /// Radar jammer (self-protection or stand-off).
    RadarJammer,
    /// A decoy emitter imitating a radar or a radio.
    Decoy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EmitterSpec {
    pub kind: EmitterKind,
    pub fmin_mhz: f64,
    pub fmax_mhz: f64,
    /// Effective radiated power over the whole band when on, W.
    pub erp_w: f64,
    pub sector_deg: f64,
    pub antenna_m: f64,
    /// Power is spread over bands: the more bands at once, the less per band (look-through and power limits).
    pub max_bands: u32,
}

impl Default for EmitterSpec {
    fn default() -> Self {
        EmitterSpec { kind: EmitterKind::CommsJammer, fmin_mhz: 20.0, fmax_mhz: 6000.0, erp_w: 100.0, sector_deg: 360.0, antenna_m: 5.0, max_bands: 3 }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoresSpec {
    pub fuel_l: f64,
    /// Batteries for drones and radios, count.
    pub batteries: f64,
    pub food_days: f64,
    pub water_l: f64,
    pub spares: f64,
    pub medical: f64,
    /// Cargo it can carry for others (trucks, ships, aircraft), tonnes.
    pub cargo_t: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CarrySpec {
    pub record: String,
    pub count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CrewSpec {
    /// 0–1: how well trained (sets error budgets, search, reaction times).
    pub skill: f64,
    /// 0–1: how cohesive and well led.
    pub cohesion: f64,
}

impl Default for CrewSpec {
    fn default() -> Self {
        CrewSpec { skill: 0.6, cohesion: 0.6 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub name: String,
    /// The archetype: its default behaviour template and its kind for assignment.
    pub archetype: String,
    pub domain: PlatformDomain,
    #[serde(default)]
    pub sidc: String,
    #[serde(default)]
    pub mover: MoverSpec,
    #[serde(default)]
    pub body: BodySpec,
    #[serde(default)]
    pub signature: SignatureSpec,
    #[serde(default)]
    pub sensors: Vec<SensorSpec>,
    #[serde(default)]
    pub mounts: Vec<MountSpec>,
    #[serde(default)]
    pub radios: Vec<RadioSpec>,
    #[serde(default)]
    pub emitters: Vec<EmitterSpec>,
    #[serde(default)]
    pub stores: StoresSpec,
    #[serde(default)]
    pub carries: Vec<CarrySpec>,
    #[serde(default)]
    pub crew: CrewSpec,
    /// Unit cost, US$ (for cost exchange).
    #[serde(default)]
    pub cost_usd: f64,
    /// Is it a command post?
    #[serde(default)]
    pub hq: bool,
    /// Its echelon, if it is a unit (platoon, company, …).
    #[serde(default)]
    pub echelon: Option<String>,
    #[serde(default)]
    pub note: String,
}

impl Record {
    pub fn mover_kind(&self) -> MoverKind {
        self.mover.kind.unwrap_or(match self.domain {
            PlatformDomain::Land => MoverKind::Wheeled,
            PlatformDomain::Surface => MoverKind::Surface,
            PlatformDomain::Subsurface => MoverKind::Sub,
            PlatformDomain::Air => MoverKind::FixedWing,
            PlatformDomain::Space => MoverKind::Orbit,
            PlatformDomain::Node => MoverKind::Static,
            PlatformDomain::Munition => MoverKind::Missile,
        })
    }
}
