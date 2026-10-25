//! Air and missile defence (RUST_ENGINE.md §8.8 Air defence, §8.6): the kill
//! chain per threat as stages with time and probability, layered systems with
//! magazines, weapon-target assignment as an optimisation, shoot-look-shoot,
//! and cost-exchange accounting [R3, R4.4, R6.2, P10, P11, P20, P28, P29].
//!
//! The abstraction is the engagement, not the endgame: each shot has a
//! single-shot kill probability by system and threat class (figures), and the
//! engagement windows, magazine depletion, saturation and decoy draw emerge
//! from geometry and timing. Threat flight paths come from [`crate::missile`].

use crate::missile::Flight;
use crate::vec3::V3;
use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreatClass {
    OneWayDrone,
    Cruise,
    SeaSkimmer,
    Ballistic,
    Rocket,
    Aircraft,
    /// A decoy: looks like a one-way drone until classified.
    Decoy,
}

impl ThreatClass {
    pub const ALL: [ThreatClass; 7] = [
        ThreatClass::OneWayDrone,
        ThreatClass::Cruise,
        ThreatClass::SeaSkimmer,
        ThreatClass::Ballistic,
        ThreatClass::Rocket,
        ThreatClass::Aircraft,
        ThreatClass::Decoy,
    ];
    pub fn key(self) -> &'static str {
        match self {
            ThreatClass::OneWayDrone => "owa",
            ThreatClass::Cruise => "cruise",
            ThreatClass::SeaSkimmer => "seaSkimmer",
            ThreatClass::Ballistic => "ballistic",
            ThreatClass::Rocket => "rocket",
            ThreatClass::Aircraft => "aircraft",
            ThreatClass::Decoy => "decoy",
        }
    }
    /// What the defender believes it is before classification.
    pub fn apparent(self) -> ThreatClass {
        if self == ThreatClass::Decoy {
            ThreatClass::OneWayDrone
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layer {
    Long,
    Medium,
    Short,
    PointDefence,
    SoftKill,
}

/// How many interceptors go at one threat per engagement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Doctrine {
    /// One shot, assess, then shoot again if time allows.
    ShootLookShoot,
    /// Two shots at once, assess, repeat (doubles interceptor use [R6.2]).
    ShootShootLookShoot,
}

/// One defending fire unit: a battery, a ship's layer, a gun, a jammer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdSystem {
    pub name: String,
    pub layer: Layer,
    pub position: V3,
    /// Sensor mast height for the radar horizon, m.
    pub sensor_height: f64,
    pub min_range: f64,
    pub max_range: f64,
    /// Single-shot kill probability by (apparent) threat class.
    pub pk: BTreeMap<ThreatClass, f64>,
    /// Cost per shot, USD.
    pub cost: f64,
    pub rounds: u32,
    /// Seconds between shots from this unit.
    pub cycle: f64,
    /// Interceptor mean speed, m/s (time to the intercept point).
    pub interceptor_speed: f64,
    pub doctrine: Doctrine,
}

impl AdSystem {
    /// A system of a declared type, from figures `ad.sys.<kind>.*`.
    pub fn from_figures(kind: &str, name: &str, position: V3, f: &Figures) -> Self {
        let g = |k: &str| f.get(&format!("ad.sys.{kind}.{k}"));
        let layer = match g("layer") as i64 {
            0 => Layer::Long,
            1 => Layer::Medium,
            2 => Layer::Short,
            3 => Layer::PointDefence,
            _ => Layer::SoftKill,
        };
        let pk = ThreatClass::ALL
            .iter()
            .filter(|c| **c != ThreatClass::Decoy)
            .map(|c| (*c, f.try_get(&format!("ad.sys.{kind}.pk.{}", c.key())).unwrap_or(0.0)))
            .collect();
        AdSystem {
            name: name.to_string(),
            layer,
            position,
            sensor_height: g("sensor_height"),
            min_range: g("min_range"),
            max_range: g("max_range"),
            pk,
            cost: g("cost"),
            rounds: g("rounds") as u32,
            cycle: g("cycle"),
            interceptor_speed: g("speed"),
            doctrine: if g("salvo") >= 2.0 { Doctrine::ShootShootLookShoot } else { Doctrine::ShootLookShoot },
        }
    }

    pub fn pk_against(&self, apparent: ThreatClass) -> f64 {
        self.pk.get(&apparent).copied().unwrap_or(0.0)
    }
}

/// One incoming threat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Threat {
    pub id: u32,
    pub class: ThreatClass,
    pub flight: Flight,
    /// Cost to the attacker, USD.
    pub cost: f64,
    /// Value of what it would destroy if it leaks (defender's scale).
    pub value: f64,
}

/// The kill chain's stages (§8.8): time and probability each [R3].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KillChain {
    pub detect_to_track: f64,
    pub track_to_classify: f64,
    pub classify_to_assign: f64,
    pub assign_to_launch: f64,
    pub assess: f64,
    pub p_track: f64,
    pub p_classify: f64,
    /// Chance a decoy is recognised as one at classification.
    pub p_discriminate: f64,
    /// Weight on interceptor cost in assignment (value units per USD).
    pub cost_weight: f64,
    /// Hold a shot for a layer at most this fraction of its cost…
    pub defer_cost_ratio: f64,
    /// …whose Pk is at least this fraction of the shot's.
    pub defer_pk_ratio: f64,
}

impl KillChain {
    pub fn from_figures(f: &Figures) -> Self {
        KillChain {
            detect_to_track: f.get("ad.chain.track_s"),
            track_to_classify: f.get("ad.chain.classify_s"),
            classify_to_assign: f.get("ad.chain.assign_s"),
            assign_to_launch: f.get("ad.chain.launch_s"),
            assess: f.get("ad.chain.assess_s"),
            p_track: f.get("ad.chain.p_track"),
            p_classify: f.get("ad.chain.p_classify"),
            p_discriminate: f.get("ad.chain.p_discriminate"),
            cost_weight: f.get("ad.wta.cost_weight"),
            defer_cost_ratio: f.get("ad.wta.defer_cost_ratio"),
            defer_pk_ratio: f.get("ad.wta.defer_pk_ratio"),
        }
    }
    /// Time from first detection to the first possible launch.
    pub fn reaction(&self) -> f64 {
        self.detect_to_track + self.track_to_classify + self.classify_to_assign + self.assign_to_launch
    }
}

/// One feasible shot for assignment.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShotOption {
    pub system: usize,
    pub threat: usize,
    pub pk: f64,
    pub cost: f64,
}

/// Weapon-target assignment: each system takes at most one threat this
/// cycle, each threat at most one system, maximising Σ value·pk − w·cost.
/// Solved exactly with Kuhn–Munkres on integer-scaled scores; shots with a
/// negative score are dropped [R4.4, P11, P29].
pub fn assign_optimal(values: &[f64], options: &[ShotOption], cost_weight: f64) -> Vec<ShotOption> {
    use pathfinding::prelude::{kuhn_munkres, Matrix};
    let systems: Vec<usize> = {
        let mut s: Vec<usize> = options.iter().map(|o| o.system).collect();
        s.sort_unstable();
        s.dedup();
        s
    };
    let threats: Vec<usize> = {
        let mut t: Vec<usize> = options.iter().map(|o| o.threat).collect();
        t.sort_unstable();
        t.dedup();
        t
    };
    if systems.is_empty() {
        return Vec::new();
    }
    // Rows: systems. Columns: threats plus one "hold fire" per system.
    let cols = threats.len() + systems.len();
    let scale = 1e6;
    let mut m = Matrix::new(systems.len(), cols, 0i64);
    let mut best: BTreeMap<(usize, usize), ShotOption> = BTreeMap::new();
    for o in options {
        let score = values[o.threat] * o.pk - cost_weight * o.cost;
        let r = systems.binary_search(&o.system).expect("listed");
        let c = threats.binary_search(&o.threat).expect("listed");
        let v = (score * scale).round() as i64;
        if v > m[(r, c)] {
            m[(r, c)] = v;
            best.insert((r, c), *o);
        }
    }
    let (_, assignment) = kuhn_munkres(&m);
    let mut out: Vec<ShotOption> = assignment
        .iter()
        .enumerate()
        .filter_map(|(r, &c)| if c < threats.len() && m[(r, c)] > 0 { best.get(&(r, c)).copied() } else { None })
        .collect();
    out.sort_by_key(|o| (o.system, o.threat));
    out
}

/// Greedy marginal-return auction: repeatedly take the shot with the best
/// marginal gain in expected value per unit cost, allowing several shots at
/// one threat with diminishing returns. Cheaper and closer to how a stressed
/// defence allocates under cost pressure.
pub fn assign_greedy(values: &[f64], options: &[ShotOption], cost_weight: f64) -> Vec<ShotOption> {
    let mut survive: BTreeMap<usize, f64> = BTreeMap::new();
    let mut used_systems = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    loop {
        let mut pick: Option<(f64, ShotOption)> = None;
        for o in options.iter().filter(|o| !used_systems.contains(&o.system)) {
            let s = *survive.get(&o.threat).unwrap_or(&1.0);
            let gain = values[o.threat] * s * o.pk - cost_weight * o.cost;
            let better = match &pick {
                None => gain > 0.0,
                Some((g, p)) => gain > *g || (gain == *g && (o.system, o.threat) < (p.system, p.threat)),
            };
            if better {
                pick = Some((gain, *o));
            }
        }
        let Some((_, o)) = pick else { break };
        used_systems.insert(o.system);
        *survive.entry(o.threat).or_insert(1.0) *= 1.0 - o.pk;
        out.push(o);
    }
    out.sort_by_key(|o| (o.system, o.threat));
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Assignment {
    Optimal,
    Greedy,
}

/// What happened to one threat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreatOutcome {
    pub id: u32,
    pub class: ThreatClass,
    pub killed_by: Option<String>,
    pub killed_at: Option<f64>,
    pub shots: u32,
    pub leaked: bool,
    pub first_seen: Option<f64>,
    pub classified_as_decoy: bool,
}

/// The whole engagement, explainable for the AAR.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RaidOutcome {
    pub threats: Vec<ThreatOutcome>,
    /// Shots and kills per system.
    pub shots_by_system: BTreeMap<String, u32>,
    pub kills_by_system: BTreeMap<String, u32>,
    pub rounds_left: BTreeMap<String, u32>,
    pub interceptor_cost: f64,
    pub attacker_cost_destroyed: f64,
    pub value_leaked: f64,
}

impl RaidOutcome {
    /// Fraction of real (non-decoy) threats destroyed.
    pub fn intercept_rate(&self) -> f64 {
        let real: Vec<_> = self.threats.iter().filter(|t| t.class != ThreatClass::Decoy).collect();
        if real.is_empty() {
            return 1.0;
        }
        real.iter().filter(|t| t.killed_by.is_some()).count() as f64 / real.len() as f64
    }
    pub fn leakers(&self) -> usize {
        self.threats.iter().filter(|t| t.leaked && t.class != ThreatClass::Decoy).count()
    }
    /// Defender's interceptor spend per attacker dollar destroyed.
    pub fn cost_exchange(&self) -> f64 {
        if self.attacker_cost_destroyed <= 0.0 {
            return f64::INFINITY;
        }
        self.interceptor_cost / self.attacker_cost_destroyed
    }
    pub fn shots(&self) -> u32 {
        self.shots_by_system.values().sum()
    }
}

struct Pending {
    system: usize,
    threat: usize,
    resolve_at: f64,
    pk: f64,
}

/// Run a raid against a layered defence, stepping at `dt` seconds.
pub fn engage_raid(systems: &mut [AdSystem], threats: &[Threat], chain: &KillChain, rule: Assignment, dt: f64, rng: &mut impl Rng) -> RaidOutcome {
    let n = threats.len();
    let mut alive = vec![true; n];
    let mut out: Vec<ThreatOutcome> = threats
        .iter()
        .map(|t| ThreatOutcome { id: t.id, class: t.class, killed_by: None, killed_at: None, shots: 0, leaked: false, first_seen: None, classified_as_decoy: false })
        .collect();
    // Each threat's kill chain: when it becomes engageable, and whether the chain completes.
    let mut engageable_at = vec![f64::INFINITY; n];
    for (i, t) in threats.iter().enumerate() {
        let seen = systems
            .iter()
            .filter_map(|s| t.flight.first_visible(s.position, s.sensor_height))
            .fold(f64::INFINITY, f64::min);
        if !seen.is_finite() {
            continue;
        }
        out[i].first_seen = Some(seen);
        if rng.gen::<f64>() >= chain.p_track * chain.p_classify {
            continue;
        }
        if t.class == ThreatClass::Decoy && rng.gen::<f64>() < chain.p_discriminate {
            out[i].classified_as_decoy = true;
            continue;
        }
        engageable_at[i] = seen + chain.reaction();
    }
    let mut ready_at = vec![0.0f64; systems.len()];
    let mut pending: Vec<Pending> = Vec::new();
    let mut shots: BTreeMap<String, u32> = systems.iter().map(|s| (s.name.clone(), 0)).collect();
    let mut kills: BTreeMap<String, u32> = systems.iter().map(|s| (s.name.clone(), 0)).collect();
    let mut cost = 0.0;
    let t_start = threats.iter().map(|t| t.flight.launched).fold(f64::INFINITY, f64::min);
    let t_end = threats.iter().map(|t| t.flight.arrival()).fold(f64::NEG_INFINITY, f64::max);
    if !t_start.is_finite() {
        return RaidOutcome { threats: out, shots_by_system: shots, kills_by_system: kills, rounds_left: BTreeMap::new(), interceptor_cost: 0.0, attacker_cost_destroyed: 0.0, value_leaked: 0.0 };
    }
    let mut t = t_start;
    while t <= t_end + dt {
        // Resolve shots whose interceptors have arrived.
        pending.sort_by(|a, b| a.resolve_at.total_cmp(&b.resolve_at).then(a.threat.cmp(&b.threat)).then(a.system.cmp(&b.system)));
        let (due, later): (Vec<Pending>, Vec<Pending>) = pending.into_iter().partition(|p| p.resolve_at <= t);
        pending = later;
        for p in due {
            let hit = rng.gen::<f64>() < p.pk;
            if hit && alive[p.threat] {
                alive[p.threat] = false;
                out[p.threat].killed_by = Some(systems[p.system].name.clone());
                out[p.threat].killed_at = Some(p.resolve_at);
                *kills.get_mut(&systems[p.system].name).expect("listed") += 1;
            }
        }
        // Leakers.
        for i in 0..n {
            if alive[i] && threats[i].flight.arrival() <= t {
                alive[i] = false;
                out[i].leaked = true;
            }
        }
        // Options this step: ready systems × engageable threats in their window
        // with no shot still in the air at them (look before shooting again).
        let mut options = Vec::new();
        for (si, s) in systems.iter().enumerate() {
            if ready_at[si] > t || s.rounds == 0 {
                continue;
            }
            for (ti, th) in threats.iter().enumerate() {
                if !alive[ti] || engageable_at[ti] > t || pending.iter().any(|p| p.threat == ti) {
                    continue;
                }
                let pos = th.flight.position(t);
                let range = (pos - s.position).norm();
                if range < s.min_range || range > s.max_range {
                    continue;
                }
                let pk = s.pk_against(th.class.apparent());
                if pk <= 0.0 {
                    continue;
                }
                // The interceptor must meet the threat before it arrives.
                let fly = time_to_intercept(s, &th.flight, t);
                if t + fly >= th.flight.arrival() {
                    continue;
                }
                // Layered doctrine: hold an expensive shot when a cheaper layer
                // that is nearly as good will still get its chance [R6.2].
                if defer_to_cheaper(si, ti, t, systems, threats, chain) {
                    continue;
                }
                options.push(ShotOption { system: si, threat: ti, pk, cost: s.cost });
            }
        }
        let values: Vec<f64> = threats.iter().map(|th| if th.class == ThreatClass::Decoy { threats.iter().filter(|x| x.class == ThreatClass::OneWayDrone).map(|x| x.value).next().unwrap_or(th.value) } else { th.value }).collect();
        let picks = match rule {
            Assignment::Optimal => assign_optimal(&values, &options, chain.cost_weight),
            Assignment::Greedy => assign_greedy(&values, &options, chain.cost_weight),
        };
        for o in picks {
            let s = &mut systems[o.system];
            let salvo = match s.doctrine {
                Doctrine::ShootLookShoot => 1,
                Doctrine::ShootShootLookShoot => 2.min(s.rounds),
            };
            let fly = time_to_intercept(s, &threats[o.threat].flight, t);
            for _ in 0..salvo {
                s.rounds -= 1;
                cost += s.cost;
                *shots.get_mut(&s.name).expect("listed") += 1;
                out[o.threat].shots += 1;
                pending.push(Pending { system: o.system, threat: o.threat, resolve_at: t + fly.max(dt) + chain.assess, pk: o.pk });
            }
            ready_at[o.system] = t + s.cycle;
        }
        t += dt;
    }
    let attacker_cost_destroyed = out.iter().zip(threats).filter(|(o, _)| o.killed_by.is_some()).map(|(_, t)| t.cost).sum();
    let value_leaked = out.iter().zip(threats).filter(|(o, t)| o.leaked && t.class != ThreatClass::Decoy).map(|(_, t)| t.value).sum();
    RaidOutcome {
        threats: out,
        shots_by_system: shots,
        kills_by_system: kills,
        rounds_left: systems.iter().map(|s| (s.name.clone(), s.rounds)).collect(),
        interceptor_cost: cost,
        attacker_cost_destroyed,
        value_leaked,
    }
}

/// Time for an interceptor launched now to meet the threat, from the
/// interceptor's speed plus the threat's closing speed.
fn time_to_intercept(s: &AdSystem, flight: &Flight, t: f64) -> f64 {
    if s.interceptor_speed <= 0.0 {
        return 0.0;
    }
    let rel = flight.position(t) - s.position;
    let range = rel.norm();
    if range <= 0.0 {
        return 0.0;
    }
    let closing = (-(flight.velocity(t).dot(rel)) / range).max(0.0);
    range / (s.interceptor_speed + closing)
}

/// Will a cheaper system with at least `defer_pk_ratio` of this one's Pk
/// have the threat inside its envelope, with time to shoot, before it arrives?
fn defer_to_cheaper(si: usize, ti: usize, t: f64, systems: &[AdSystem], threats: &[Threat], chain: &KillChain) -> bool {
    let me = &systems[si];
    let th = &threats[ti];
    let class = th.class.apparent();
    let my_pk = me.pk_against(class);
    let arrival = th.flight.arrival();
    systems.iter().enumerate().any(|(oi, o)| {
        if oi == si || o.rounds == 0 || o.cost >= me.cost * chain.defer_cost_ratio || o.pk_against(class) < chain.defer_pk_ratio * my_pk {
            return false;
        }
        // Sample the rest of the flight for a moment inside the other envelope.
        let steps = 40;
        (1..=steps).any(|k| {
            let tt = t + (arrival - t) * k as f64 / (steps + 1) as f64;
            let r = (th.flight.position(tt) - o.position).norm();
            r >= o.min_range && r <= o.max_range && tt + time_to_intercept(o, &th.flight, tt) + o.cycle < arrival
        })
    })
}

/// Probability a ballistic threat is killed by `shots` independent
/// interceptors of per-shot Pk `pk` [P29].
pub fn salvo_pk(pk: f64, shots: u32) -> f64 {
    1.0 - (1.0 - pk).powi(shots as i32)
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("ad.chain.track_s", 8.0, "s", 2.0, 30.0, "R3", Low, "detection to firm track"),
        figure!("ad.chain.classify_s", 6.0, "s", 1.0, 30.0, "R3", Low, "track to classification"),
        figure!("ad.chain.assign_s", 4.0, "s", 1.0, 20.0, "R3", Low, "classification to weapon assignment"),
        figure!("ad.chain.launch_s", 3.0, "s", 1.0, 10.0, "R3", Low, "assignment to launch"),
        figure!("ad.chain.assess_s", 4.0, "s", 1.0, 15.0, "default", Default, "kill assessment after intercept"),
        figure!("ad.chain.p_track", 0.98, "fraction", 0.8, 1.0, "default", Default, "chance a detection becomes a firm track"),
        figure!("ad.chain.p_classify", 0.97, "fraction", 0.8, 1.0, "default", Default, "chance a track is classified in time"),
        figure!("ad.chain.p_discriminate", 0.4, "fraction", 0.0, 0.9, "P10", Low, "chance a decoy is recognised before engagement"),
        figure!("ad.wta.cost_weight", 0.05, "value/USD", 0.0, 1.0, "R4.4", Low, "weight on interceptor cost in assignment (value in USD)"),
        figure!("ad.wta.defer_cost_ratio", 0.25, "fraction", 0.0, 1.0, "R6.2", Default, "defer to a layer costing at most this fraction"),
        figure!("ad.wta.defer_pk_ratio", 0.5, "fraction", 0.0, 1.0, "R6.2", Default, "defer only to a layer with at least this share of the Pk"),
    ];
    // Systems: (kind, layer, sensor h, min, max, cost, rounds, cycle, speed, salvo, [(class, pk)], note).
    type Row<'a> = (&'a str, f64, f64, f64, f64, f64, f64, f64, f64, f64, &'a [(&'a str, f64)], &'a str);
    let rows: [Row; 10] = [
        ("lrsam", 0.0, 25.0, 3_000.0, 160_000.0, 4.0e6, 16.0, 3.0, 1_200.0, 2.0, &[("ballistic", 0.65), ("cruise", 0.85), ("aircraft", 0.85), ("seaSkimmer", 0.7), ("owa", 0.85)], "PAC-3/SM-6 class [P11, P29]"),
        ("mrsam", 1.0, 20.0, 1_500.0, 50_000.0, 1.0e6, 24.0, 2.0, 900.0, 1.0, &[("cruise", 0.8), ("aircraft", 0.8), ("seaSkimmer", 0.75), ("owa", 0.85), ("rocket", 0.5)], "ESSM/NASAMS class [P28]"),
        ("shorad", 2.0, 12.0, 500.0, 9_000.0, 1.5e5, 16.0, 2.0, 600.0, 1.0, &[("cruise", 0.7), ("aircraft", 0.6), ("seaSkimmer", 0.75), ("owa", 0.8), ("rocket", 0.4)], "RAM/Stinger-class launchers [P28]"),
        ("gun", 3.0, 10.0, 100.0, 2_000.0, 2.0e3, 60.0, 2.0, 1_000.0, 1.0, &[("owa", 0.35), ("cruise", 0.3), ("seaSkimmer", 0.35), ("rocket", 0.2)], "CIWS/AA gun burst [P28]"),
        ("gun5in", 2.0, 20.0, 1_000.0, 15_000.0, 1.0e4, 600.0, 6.0, 800.0, 1.0, &[("owa", 0.3), ("cruise", 0.15), ("seaSkimmer", 0.1)], "5-inch naval gun against drones [R6.2: 160 rounds in the Red Sea]"),
        ("mobileGun", 3.0, 3.0, 50.0, 1_500.0, 5.0e2, 40.0, 4.0, 1_000.0, 1.0, &[("owa", 0.3)], "truck-mounted gun team vs Shahed [P10]"),
        ("interceptorDrone", 2.0, 10.0, 300.0, 15_000.0, 5.0e3, 30.0, 8.0, 80.0, 1.0, &[("owa", 0.6)], "interceptor drone [P11: $1.4-16k]"),
        ("ewSoftKill", 4.0, 10.0, 0.0, 25_000.0, 0.0, 1000.0, 30.0, 0.0, 1.0, &[("owa", 0.2), ("cruise", 0.1)], "GNSS/datalink jamming of drones and cruise missiles"),
        ("fighter", 0.0, 8_000.0, 1_000.0, 80_000.0, 4.0e5, 6.0, 30.0, 1_000.0, 1.0, &[("owa", 0.85), ("cruise", 0.8), ("aircraft", 0.6)], "fighter with AAMs and gun on CAP [R6.2]"),
        ("bmd", 0.0, 20.0, 5_000.0, 100_000.0, 4.2e6, 16.0, 3.0, 1_500.0, 2.0, &[("ballistic", 0.65)], "BMD battery, shoot-shoot-look-shoot [P20, P29]"),
    ];
    for (k, layer, h, min, max, cost, rounds, cycle, speed, salvo, pks, note) in rows {
        let p = |n: &str| format!("ad.sys.{k}.{n}");
        v.push(figure!(p("layer"), layer, "enum", 0.0, 4.0, "default", Default, "0 long, 1 medium, 2 short, 3 point, 4 soft kill"));
        v.push(figure!(p("sensor_height"), h, "m", 1.0, 15_000.0, "default", Default, note));
        v.push(figure!(p("min_range"), min, "m", 0.0, 10_000.0, "default", Default, "minimum engagement range"));
        v.push(figure!(p("max_range"), max, "m", 500.0, 250_000.0, "P28", Default, "maximum engagement range"));
        v.push(figure!(p("cost"), cost, "USD", 0.0, 1.5e7, "P11", Low, "cost per shot"));
        v.push(figure!(p("rounds"), rounds, "count", 1.0, 2000.0, "default", Default, "magazine"));
        v.push(figure!(p("cycle"), cycle, "s", 0.5, 120.0, "default", Default, "time between engagements"));
        v.push(figure!(p("speed"), speed, "m/s", 0.0, 3_000.0, "default", Default, "interceptor mean speed (0 = instant effect)"));
        v.push(figure!(p("salvo"), salvo, "count", 1.0, 2.0, "R6.2", Low, "1 shoot-look-shoot, 2 shoot-shoot-look-shoot"));
        for (c, pk) in pks {
            v.push(figure!(p(&format!("pk.{c}")), *pk, "fraction", 0.0, 1.0, "P29", Default, "single-shot kill probability"));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::missile::{FlightClass, Profile};
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn figs() -> Figures {
        let mut l = figures();
        l.extend(crate::missile::figures());
        Figures::from_list(l).unwrap()
    }

    #[test]
    fn optimal_assignment_beats_or_matches_greedy_single_shot() {
        let values = [10.0, 5.0, 1.0];
        let opts = [
            ShotOption { system: 0, threat: 0, pk: 0.5, cost: 0.0 },
            ShotOption { system: 0, threat: 1, pk: 0.9, cost: 0.0 },
            ShotOption { system: 1, threat: 0, pk: 0.9, cost: 0.0 },
            ShotOption { system: 1, threat: 2, pk: 0.9, cost: 0.0 },
        ];
        let a = assign_optimal(&values, &opts, 0.0);
        let score: f64 = a.iter().map(|o| values[o.threat] * o.pk).sum();
        assert_eq!(a.len(), 2);
        assert!((score - (4.5 + 9.0)).abs() < 1e-9, "{a:?}");
        // Cost pressure: an expensive shot at a cheap target is not taken.
        let pricey = [ShotOption { system: 0, threat: 2, pk: 0.9, cost: 1e7 }];
        assert!(assign_optimal(&values, &pricey, 1e-6).is_empty());
        assert!(assign_greedy(&values, &pricey, 1e-6).is_empty());
    }

    fn shahed_raid(f: &Figures, n_real: u32, n_decoy: u32) -> Vec<Threat> {
        let prof = Profile::from_figures(FlightClass::OneWayDrone, f);
        (0..n_real + n_decoy)
            .map(|i| {
                let a = i as f64 * 0.013;
                let from = V3::new(120_000.0 * a.cos(), 120_000.0 * a.sin(), 0.0);
                let class = if i < n_real { ThreatClass::OneWayDrone } else { ThreatClass::Decoy };
                Threat { id: i, class, flight: Flight::new(prof.clone(), from, V3::ZERO, i as f64 * 3.0), cost: if i < n_real { 5.0e4 } else { 1.0e4 }, value: 1.0e6 }
            })
            .collect()
    }

    fn layered(f: &Figures) -> Vec<AdSystem> {
        let mut v = vec![
            AdSystem::from_figures("mrsam", "mrsam", V3::new(-2_000.0, 0.0, 0.0), f),
            AdSystem::from_figures("ewSoftKill", "ew", V3::ZERO, f),
        ];
        for i in 0..3 {
            let a = i as f64 * std::f64::consts::TAU / 3.0;
            v.push(AdSystem::from_figures("mobileGun", &format!("gun{i}"), V3::new(8_000.0 * a.cos(), 8_000.0 * a.sin(), 0.0), f));
            v.push(AdSystem::from_figures("interceptorDrone", &format!("idrone{i}"), V3::new(4_000.0 * a.cos(), 4_000.0 * a.sin(), 0.0), f));
        }
        v
    }

    #[test]
    fn shahed_raid_intercept_rate_in_band() {
        // P10: ~60% strike / 40% decoy, ~85% intercepted.
        let f = figs();
        let chain = KillChain::from_figures(&f);
        let mut rates = Vec::new();
        let mut decoy_shots = 0;
        for seed in 0..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut sys = layered(&f);
            let out = engage_raid(&mut sys, &shahed_raid(&f, 60, 40), &chain, Assignment::Optimal, 1.0, &mut rng);
            rates.push(out.intercept_rate());
            decoy_shots += out.threats.iter().filter(|t| t.class == ThreatClass::Decoy).map(|t| t.shots).sum::<u32>();
        }
        let mean = rates.iter().sum::<f64>() / rates.len() as f64;
        assert!((0.75..=0.95).contains(&mean), "intercept rate {mean}");
        assert!(decoy_shots > 0, "decoys should draw fire");
    }

    #[test]
    fn ballistic_defence_with_two_shots_in_band() {
        // P20: ~85% (273/322) with 1.5-2 interceptors per threat at Pk 0.6-0.7 [P29].
        let f = figs();
        let chain = KillChain::from_figures(&f);
        assert!((salvo_pk(0.65, 2) - 0.8775).abs() < 1e-9);
        let prof = Profile::from_figures(FlightClass::Ballistic, &f);
        let mut killed = 0;
        let mut total = 0;
        let mut shots = 0;
        for seed in 0..20 {
            let mut rng = ChaCha8Rng::seed_from_u64(100 + seed);
            let mut sys = vec![AdSystem::from_figures("bmd", "bmd1", V3::ZERO, &f), AdSystem::from_figures("bmd", "bmd2", V3::new(3_000.0, 0.0, 0.0), &f)];
            let raid: Vec<Threat> = (0..6)
                .map(|i| Threat { id: i, class: ThreatClass::Ballistic, flight: Flight::new(prof.clone(), V3::new(300_000.0, 20_000.0 * i as f64, 0.0), V3::new(0.0, 500.0, 0.0), 30.0 * i as f64), cost: 3.0e6, value: 1.0e8 })
                .collect();
            let out = engage_raid(&mut sys, &raid, &chain, Assignment::Optimal, 1.0, &mut rng);
            killed += out.threats.iter().filter(|t| t.killed_by.is_some()).count();
            total += raid.len();
            shots += out.shots();
        }
        let rate = killed as f64 / total as f64;
        let per = shots as f64 / total as f64;
        assert!((0.75..=0.95).contains(&rate), "BMD rate {rate}");
        assert!((1.5..=3.5).contains(&per), "interceptors per threat {per}");
    }

    #[test]
    fn magazines_deplete_and_cost_exchange_is_reported() {
        let f = figs();
        let chain = KillChain::from_figures(&f);
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        let mut sys = vec![AdSystem::from_figures("lrsam", "lrsam", V3::ZERO, &f)];
        let out = engage_raid(&mut sys, &shahed_raid(&f, 40, 0), &chain, Assignment::Greedy, 1.0, &mut rng);
        assert!(out.rounds_left["lrsam"] < 16);
        assert!(out.cost_exchange() > 10.0, "expensive interceptors vs cheap drones: {}", out.cost_exchange());
    }
}
