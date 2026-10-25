//! Naval and undersea (RUST_ENGINE.md §8.1 Sea, §8.6): ship and submarine
//! movement, small craft limited by sea state [R6.3], Hughes salvo exchanges
//! [R6.1], layered ship defence against a salvo through [`crate::air_defence`]
//! with magazine depletion [R6.2, P28], compartment damage with fire, flooding
//! and damage control, mines, and reloading and replenishment.

use bgws_core::{figure, Figure, Figures};
use petgraph::graph::{NodeIndex, UnGraph};
use rand::Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HullClass {
    Destroyer,
    Frigate,
    Corvette,
    Merchant,
    SmallCraft,
    Usv,
    Submarine,
}

impl HullClass {
    pub fn key(self) -> &'static str {
        match self {
            HullClass::Destroyer => "destroyer",
            HullClass::Frigate => "frigate",
            HullClass::Corvette => "corvette",
            HullClass::Merchant => "merchant",
            HullClass::SmallCraft => "smallCraft",
            HullClass::Usv => "usv",
            HullClass::Submarine => "submarine",
        }
    }
    pub const ALL: [HullClass; 7] = [HullClass::Destroyer, HullClass::Frigate, HullClass::Corvette, HullClass::Merchant, HullClass::SmallCraft, HullClass::Usv, HullClass::Submarine];
}

/// Movement figures for one hull class.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HullParams {
    pub class: HullClass,
    pub max_speed: f64,
    /// Sea state (Douglas) at which speed starts to fall, and at which the hull can no longer operate.
    pub sea_state_limit: f64,
    pub sea_state_max: f64,
    pub turning_diameter: f64,
    /// Fuel per hour at max speed, tonnes; fuel ∝ speed³.
    pub fuel_at_max: f64,
    pub fuel_capacity: f64,
}

impl HullParams {
    pub fn from_figures(class: HullClass, f: &Figures) -> Self {
        let g = |k: &str| f.get(&format!("naval.hull.{}.{k}", class.key()));
        HullParams {
            class,
            max_speed: g("max_speed"),
            sea_state_limit: g("ss_limit"),
            sea_state_max: g("ss_max"),
            turning_diameter: g("turn_d"),
            fuel_at_max: g("fuel_max"),
            fuel_capacity: g("fuel_cap"),
        }
    }

    /// Speed attainable in sea state `ss`, m/s: full below the limit, falling
    /// linearly to zero at the maximum.
    pub fn speed_in(&self, ss: f64) -> f64 {
        if ss <= self.sea_state_limit {
            self.max_speed
        } else if ss >= self.sea_state_max {
            0.0
        } else {
            self.max_speed * (self.sea_state_max - ss) / (self.sea_state_max - self.sea_state_limit)
        }
    }

    /// Fuel burn per hour at speed `v` (cubic law), tonnes.
    pub fn fuel_per_hour(&self, v: f64) -> f64 {
        self.fuel_at_max * (v / self.max_speed).powi(3)
    }

    /// Range in metres at speed `v` on the fuel aboard.
    pub fn range_at(&self, v: f64, fuel: f64) -> f64 {
        let burn = self.fuel_per_hour(v);
        if burn <= 0.0 {
            return f64::INFINITY;
        }
        fuel / burn * 3600.0 * v
    }

    /// Maximum turn rate at speed `v`, rad/s.
    pub fn turn_rate(&self, v: f64) -> f64 {
        2.0 * v / self.turning_diameter
    }
}

/// A submarine's acoustic and energy state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Submarine {
    pub nuclear: bool,
    /// Battery state of charge, 0–1 (diesel-electric boats).
    pub battery: f64,
    pub snorting: bool,
    pub depth: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubParams {
    pub quiet_noise_db: f64,
    pub noise_per_knot_db: f64,
    pub snort_noise_db: f64,
    pub battery_hours_at_4kn: f64,
    pub recharge_hours: f64,
    pub snort_depth: f64,
}

impl SubParams {
    pub fn from_figures(f: &Figures) -> Self {
        SubParams {
            quiet_noise_db: f.get("naval.sub.quiet_sl"),
            noise_per_knot_db: f.get("naval.sub.sl_per_knot"),
            snort_noise_db: f.get("naval.sub.snort_sl"),
            battery_hours_at_4kn: f.get("naval.sub.battery_h_4kn"),
            recharge_hours: f.get("naval.sub.recharge_h"),
            snort_depth: f.get("naval.sub.snort_depth"),
        }
    }
}

impl Submarine {
    /// Radiated source level (dB re 1 µPa @ 1 m) at `speed_kn`.
    pub fn source_level(&self, p: &SubParams, speed_kn: f64) -> f64 {
        let base = p.quiet_noise_db + p.noise_per_knot_db * (speed_kn - 4.0).max(0.0);
        if self.snorting {
            base.max(p.snort_noise_db)
        } else {
            base
        }
    }

    /// Advance the battery by `hours` at `speed_kn` (battery drain ∝ speed³ about 4 kn).
    pub fn step(&mut self, p: &SubParams, speed_kn: f64, hours: f64) {
        if self.nuclear {
            return;
        }
        if self.snorting {
            self.battery = (self.battery + hours / p.recharge_hours).min(1.0);
        } else {
            let rate = (speed_kn / 4.0).max(0.25).powi(3) / p.battery_hours_at_4kn;
            self.battery = (self.battery - rate * hours).max(0.0);
        }
    }

    /// Must it come up to snort soon?
    pub fn needs_snort(&self, threshold: f64) -> bool {
        !self.nuclear && self.battery < threshold
    }
}

/// Hughes' salvo equation: ships of side B put out of action by side A's
/// salvo, ΔB = max(0, (α·A·σ − b₃·B·τ)/b₁) [R6.1].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Salvo {
    /// Attacking ships, and the well-aimed missiles each fires.
    pub attackers: f64,
    pub alpha: f64,
    /// Scouting effectiveness (0–1) and defender alertness (0–1).
    pub scouting: f64,
    pub alertness: f64,
    pub defenders: f64,
    /// Missiles each defender can stop, and hits each can take before out of action.
    pub b3: f64,
    pub b1: f64,
}

impl Salvo {
    pub fn ships_out_of_action(&self) -> f64 {
        let offence = self.alpha * self.attackers * self.scouting;
        let defence = self.b3 * self.defenders * self.alertness;
        ((offence - defence) / self.b1).max(0.0).min(self.defenders)
    }
}

/// When a ship should leave station to reload: below this fraction of VLS
/// cells [R6.2].
pub fn should_withdraw_to_rearm(cells_loaded: u32, cells_total: u32, threshold: f64) -> bool {
    cells_total > 0 && (cells_loaded as f64) < threshold * cells_total as f64
}

/// Time to reload VLS cells: needs a port or a capable tender.
pub fn reload_hours(cells: u32, at_port: bool, tender: bool, f: &Figures) -> Option<f64> {
    if at_port {
        Some(cells as f64 * f.get("naval.reload.port_min_per_cell") / 60.0)
    } else if tender {
        Some(cells as f64 * f.get("naval.reload.tender_min_per_cell") / 60.0)
    } else {
        None
    }
}

/// Underway replenishment of fuel and stores: hours alongside.
pub fn unrep_hours(fuel_t: f64, stores_t: f64, f: &Figures) -> f64 {
    f.get("naval.unrep.setup_h") + fuel_t / f.get("naval.unrep.fuel_t_per_h") + stores_t / f.get("naval.unrep.stores_t_per_h")
}

/// A ship system a compartment carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShipSystem {
    Radar,
    Launchers,
    Gun,
    Propulsion,
    Power,
    Bridge,
    Hangar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Compartment {
    pub name: String,
    pub systems: Vec<ShipSystem>,
    /// 0–1: fraction burning.
    pub fire: f64,
    /// 0–1: fraction flooded.
    pub flood: f64,
    pub destroyed: bool,
}

/// A ship's internal layout and damage state: compartments joined by
/// adjacency (fire spreads along edges), flooding sums into lost buoyancy.
#[derive(Clone, Debug)]
pub struct ShipDamage {
    pub graph: UnGraph<Compartment, ()>,
    /// Damage-control parties available.
    pub dc_parties: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageState {
    pub systems_lost: Vec<ShipSystem>,
    pub flooding: f64,
    pub fires: u32,
    pub mission_kill: bool,
    pub sunk: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DamageParams {
    pub fire_growth: f64,
    pub fire_spread: f64,
    pub flood_rate: f64,
    pub dc_fire: f64,
    pub dc_flood: f64,
    pub sink_flooding: f64,
    pub hit_flood: f64,
    pub hit_fire_p: f64,
    pub hit_destroy_p: f64,
}

impl DamageParams {
    pub fn from_figures(f: &Figures) -> Self {
        DamageParams {
            fire_growth: f.get("naval.dmg.fire_growth"),
            fire_spread: f.get("naval.dmg.fire_spread"),
            flood_rate: f.get("naval.dmg.flood_rate"),
            dc_fire: f.get("naval.dmg.dc_fire"),
            dc_flood: f.get("naval.dmg.dc_flood"),
            sink_flooding: f.get("naval.dmg.sink_flooding"),
            hit_flood: f.get("naval.dmg.hit_flood"),
            hit_fire_p: f.get("naval.dmg.hit_fire_p"),
            hit_destroy_p: f.get("naval.dmg.hit_destroy_p"),
        }
    }
}

impl ShipDamage {
    /// A generic surface combatant's layout, bow to stern.
    pub fn combatant(dc_parties: u32) -> Self {
        let layout: [(&str, &[ShipSystem]); 6] = [
            ("bow", &[ShipSystem::Gun]),
            ("forward VLS", &[ShipSystem::Launchers]),
            ("bridge and CIC", &[ShipSystem::Bridge, ShipSystem::Radar]),
            ("machinery", &[ShipSystem::Propulsion, ShipSystem::Power]),
            ("aft VLS", &[ShipSystem::Launchers]),
            ("hangar", &[ShipSystem::Hangar]),
        ];
        let mut g = UnGraph::new_undirected();
        let ids: Vec<NodeIndex> = layout
            .iter()
            .map(|(n, s)| g.add_node(Compartment { name: n.to_string(), systems: s.to_vec(), fire: 0.0, flood: 0.0, destroyed: false }))
            .collect();
        for w in ids.windows(2) {
            g.add_edge(w[0], w[1], ());
        }
        ShipDamage { graph: g, dc_parties }
    }

    /// A hit in compartment `i` (by index), with `warhead_scale` (1 = a typical anti-ship missile).
    pub fn hit(&mut self, i: usize, warhead_scale: f64, p: &DamageParams, rng: &mut impl Rng) {
        let n = NodeIndex::new(i.min(self.graph.node_count() - 1));
        let c = &mut self.graph[n];
        c.flood = (c.flood + p.hit_flood * warhead_scale).min(1.0);
        if rng.gen::<f64>() < p.hit_fire_p {
            c.fire = c.fire.max(0.2 * warhead_scale).min(1.0);
        }
        if rng.gen::<f64>() < p.hit_destroy_p * warhead_scale {
            c.destroyed = true;
        }
    }

    /// Advance fire and flooding by `minutes`, with damage control sent to the
    /// worst compartments first.
    pub fn step(&mut self, minutes: f64, p: &DamageParams, rng: &mut impl Rng) {
        let mut order: Vec<NodeIndex> = self.graph.node_indices().collect();
        order.sort_by(|a, b| {
            let (ca, cb) = (&self.graph[*a], &self.graph[*b]);
            (cb.fire + cb.flood).total_cmp(&(ca.fire + ca.flood)).then(a.index().cmp(&b.index()))
        });
        let fighting: Vec<NodeIndex> = order.iter().take(self.dc_parties as usize).copied().collect();
        let mut spread = Vec::new();
        for n in self.graph.node_indices() {
            let dc = fighting.contains(&n);
            let c = &mut self.graph[n];
            if c.fire > 0.0 {
                c.fire = (c.fire + minutes * (p.fire_growth - if dc { p.dc_fire } else { 0.0 })).clamp(0.0, 1.0);
                if c.fire >= 0.8 {
                    c.destroyed = true;
                }
                if c.fire > 0.3 {
                    spread.push(n);
                }
            }
            if c.flood > 0.0 && c.flood < 1.0 {
                c.flood = (c.flood + minutes * (p.flood_rate * c.flood - if dc { p.dc_flood } else { 0.0 })).clamp(0.0, 1.0);
            }
        }
        for n in spread {
            let neighbours: Vec<NodeIndex> = self.graph.neighbors(n).collect();
            for m in neighbours {
                if rng.gen::<f64>() < 1.0 - (-p.fire_spread * minutes).exp() {
                    let c = &mut self.graph[m];
                    c.fire = c.fire.max(0.1);
                }
            }
        }
    }

    pub fn state(&self, p: &DamageParams) -> DamageState {
        let mut lost: Vec<ShipSystem> = self
            .graph
            .node_weights()
            .filter(|c| c.destroyed || c.fire > 0.5 || c.flood > 0.6)
            .flat_map(|c| c.systems.iter().copied())
            .collect();
        lost.sort();
        lost.dedup();
        let flooding = self.graph.node_weights().map(|c| c.flood).sum::<f64>() / self.graph.node_count() as f64;
        let fires = self.graph.node_weights().filter(|c| c.fire > 0.0).count() as u32;
        let mission_kill = lost.contains(&ShipSystem::Radar) || lost.contains(&ShipSystem::Propulsion) || lost.iter().filter(|s| **s == ShipSystem::Launchers).count() > 0 && lost.contains(&ShipSystem::Power);
        DamageState { mission_kill: mission_kill || flooding >= p.sink_flooding * 0.5, sunk: flooding >= p.sink_flooding, systems_lost: lost, flooding, fires }
    }
}

/// Sea mines (§8.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MineKind {
    Contact,
    Magnetic,
    Acoustic,
    Pressure,
}

/// A ship's influence signatures, 0–1 relative to an untreated merchant.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InfluenceSignature {
    pub magnetic: f64,
    pub acoustic: f64,
    pub pressure: f64,
    pub beam: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Minefield {
    pub kind: MineKind,
    /// Mines per square kilometre.
    pub density: f64,
    /// Fraction already cleared.
    pub cleared: f64,
}

impl Minefield {
    /// Probability a transit of `length_m` through the field actuates a mine.
    pub fn actuation_probability(&self, sig: &InfluenceSignature, length_m: f64, f: &Figures) -> f64 {
        let (width, sens) = match self.kind {
            MineKind::Contact => (sig.beam, 1.0),
            MineKind::Magnetic => (f.get("naval.mine.influence_width") * sig.magnetic, 1.0),
            MineKind::Acoustic => (f.get("naval.mine.influence_width") * sig.acoustic, 1.0),
            MineKind::Pressure => (f.get("naval.mine.influence_width") * 0.5 * sig.pressure, 1.0),
        };
        let swept_km2 = width * length_m / 1e6;
        1.0 - (-self.density * (1.0 - self.cleared) * swept_km2 * sens).exp()
    }

    /// Clear by sweeping or hunting for `hours` with `assets` vessels.
    pub fn clear(&mut self, hunting: bool, assets: f64, hours: f64, area_km2: f64, f: &Figures) {
        let rate = if hunting { f.get("naval.mine.hunt_km2_h") } else { f.get("naval.mine.sweep_km2_h") };
        let sweep = if hunting { 1.0 } else if self.kind == MineKind::Contact { 0.9 } else { 0.6 };
        let frac = (rate * assets * hours / area_km2.max(1e-6)).min(1.0) * sweep;
        self.cleared = 1.0 - (1.0 - self.cleared) * (1.0 - frac);
    }
}

pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    // (class, max speed m/s, ss limit, ss max, turning diameter m, fuel t/h at max, fuel capacity t, note)
    type Row = (HullClass, f64, f64, f64, f64, f64, f64, &'static str);
    let rows: [Row; 7] = [
        (HullClass::Destroyer, 15.4, 6.0, 9.0, 900.0, 18.0, 1_500.0, "Burke-class-like destroyer"),
        (HullClass::Frigate, 14.4, 6.0, 8.5, 800.0, 12.0, 800.0, "frigate"),
        (HullClass::Corvette, 13.4, 5.0, 7.5, 600.0, 6.0, 250.0, "corvette"),
        (HullClass::Merchant, 7.7, 6.0, 9.0, 1_500.0, 4.0, 3_000.0, "bulk carrier/container ship"),
        (HullClass::SmallCraft, 18.0, 3.0, 5.0, 150.0, 0.6, 10.0, "fast attack craft [R6.3]"),
        (HullClass::Usv, 20.0, 3.0, 4.5, 80.0, 0.1, 1.0, "Magura-type USV [R6.3]"),
        (HullClass::Submarine, 10.0, 7.0, 9.0, 700.0, 2.0, 300.0, "diesel-electric submarine (snorting)"),
    ];
    for (c, vmax, ssl, ssm, td, fm, fc, note) in rows {
        let p = |n: &str| format!("naval.hull.{}.{n}", c.key());
        v.push(figure!(p("max_speed"), vmax, "m/s", vmax * 0.7, vmax * 1.3, "default", Default, note));
        v.push(figure!(p("ss_limit"), ssl, "douglas", 1.0, 9.0, "R6.3", Default, "sea state where speed starts to fall"));
        v.push(figure!(p("ss_max"), ssm, "douglas", 2.0, 9.0, "R6.3", Default, "sea state beyond which it cannot operate"));
        v.push(figure!(p("turn_d"), td, "m", td * 0.5, td * 2.0, "default", Default, "tactical diameter"));
        v.push(figure!(p("fuel_max"), fm, "t/h", fm * 0.5, fm * 2.0, "default", Default, "fuel burn at maximum speed"));
        v.push(figure!(p("fuel_cap"), fc, "t", fc * 0.5, fc * 2.0, "default", Default, "fuel capacity"));
    }
    v.extend([
        figure!("naval.sub.quiet_sl", 115.0, "dB", 100.0, 135.0, "R6.3", Default, "radiated noise at 4 kn on battery"),
        figure!("naval.sub.sl_per_knot", 1.5, "dB/kn", 0.5, 3.0, "default", Default, "noise growth above 4 kn"),
        figure!("naval.sub.snort_sl", 140.0, "dB", 125.0, 155.0, "default", Default, "radiated noise while snorting"),
        figure!("naval.sub.battery_h_4kn", 72.0, "h", 24.0, 120.0, "default", Default, "battery endurance at 4 kn"),
        figure!("naval.sub.recharge_h", 6.0, "h", 2.0, 12.0, "default", Default, "snorting hours for a full charge"),
        figure!("naval.sub.snort_depth", 15.0, "m", 8.0, 25.0, "default", Default, "periscope/snort depth"),
        figure!("naval.reload.port_min_per_cell", 30.0, "min", 10.0, 90.0, "R6.2", Low, "VLS reload alongside"),
        figure!("naval.reload.tender_min_per_cell", 60.0, "min", 20.0, 180.0, "R6.2", Default, "VLS reload from a tender"),
        figure!("naval.reload.withdraw_fraction", 0.3, "fraction", 0.1, 0.5, "R6.2", Low, "withdraw to rearm below this VLS fraction"),
        figure!("naval.unrep.setup_h", 0.75, "h", 0.25, 2.0, "default", Default, "approach and rig for UNREP"),
        figure!("naval.unrep.fuel_t_per_h", 400.0, "t/h", 100.0, 900.0, "default", Default, "UNREP fuel transfer rate"),
        figure!("naval.unrep.stores_t_per_h", 40.0, "t/h", 10.0, 100.0, "default", Default, "UNREP stores transfer rate"),
        figure!("naval.dmg.fire_growth", 0.02, "1/min", 0.0, 0.1, "default", Default, "fire growth without damage control"),
        figure!("naval.dmg.fire_spread", 0.02, "1/min", 0.0, 0.1, "default", Default, "fire spread hazard to adjacent compartments"),
        figure!("naval.dmg.flood_rate", 0.03, "1/min", 0.0, 0.2, "default", Default, "relative flooding growth"),
        figure!("naval.dmg.dc_fire", 0.05, "1/min", 0.0, 0.2, "default", Default, "fire reduction by one DC party"),
        figure!("naval.dmg.dc_flood", 0.02, "1/min", 0.0, 0.1, "default", Default, "flooding reduction by one DC party"),
        figure!("naval.dmg.sink_flooding", 0.45, "fraction", 0.2, 0.8, "default", Default, "mean flooding at which the ship sinks"),
        figure!("naval.dmg.hit_flood", 0.25, "fraction", 0.0, 1.0, "default", Default, "flooding a hit starts in its compartment"),
        figure!("naval.dmg.hit_fire_p", 0.7, "fraction", 0.0, 1.0, "default", Default, "chance a hit starts a fire"),
        figure!("naval.dmg.hit_destroy_p", 0.35, "fraction", 0.0, 1.0, "default", Default, "chance a hit destroys its compartment's systems"),
        figure!("naval.mine.influence_width", 120.0, "m", 30.0, 400.0, "default", Default, "actuation width against an untreated hull"),
        figure!("naval.mine.sweep_km2_h", 1.5, "km2/h", 0.2, 5.0, "default", Default, "sweep rate per vessel"),
        figure!("naval.mine.hunt_km2_h", 0.3, "km2/h", 0.05, 1.5, "default", Default, "hunt rate per vessel"),
    ]);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_defence::{engage_raid, AdSystem, Assignment, KillChain, Threat, ThreatClass};
    use crate::missile::{Flight, FlightClass, Profile};
    use crate::vec3::V3;
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;

    fn figs() -> Figures {
        let mut l = figures();
        l.extend(crate::air_defence::figures());
        l.extend(crate::missile::figures());
        Figures::from_list(l).unwrap()
    }

    #[test]
    fn small_craft_stop_long_before_destroyers() {
        let f = figs();
        let usv = HullParams::from_figures(HullClass::Usv, &f);
        let ddg = HullParams::from_figures(HullClass::Destroyer, &f);
        assert_eq!(usv.speed_in(5.0), 0.0);
        assert_eq!(ddg.speed_in(5.0), ddg.max_speed);
        // Cubic fuel law: half speed goes about four times as far.
        let r_full = ddg.range_at(ddg.max_speed, 1000.0);
        let r_half = ddg.range_at(ddg.max_speed / 2.0, 1000.0);
        assert!((r_half / r_full - 4.0).abs() < 1e-9);
    }

    #[test]
    fn diesel_boat_must_snort_and_is_loud_when_it_does() {
        let f = figs();
        let p = SubParams::from_figures(&f);
        let mut s = Submarine { nuclear: false, battery: 1.0, snorting: false, depth: 100.0 };
        let quiet = s.source_level(&p, 4.0);
        s.step(&p, 8.0, 8.0);
        assert!(s.needs_snort(0.2));
        s.snorting = true;
        assert!(s.source_level(&p, 4.0) > quiet + 20.0);
        s.step(&p, 4.0, 6.0);
        assert!((s.battery - 1.0).abs() < 1e-9);
    }

    #[test]
    fn hughes_salvo() {
        let s = Salvo { attackers: 4.0, alpha: 8.0, scouting: 1.0, alertness: 1.0, defenders: 3.0, b3: 6.0, b1: 2.0 };
        assert!((s.ships_out_of_action() - 7.0f64.min(3.0)).abs() < 1e-9);
        let poor = Salvo { scouting: 0.5, ..s.clone() };
        assert!((poor.ships_out_of_action() - 0.0f64.max((16.0 - 18.0) / 2.0)).abs() < 1e-9);
    }

    #[test]
    fn red_sea_style_defence_spends_fewer_than_one_missile_per_engagement_with_guns() {
        // R6.2: ~0.58 missiles per engagement when guns, fighters and EW carry part of the load.
        let f = figs();
        let chain = KillChain::from_figures(&f);
        let owa = Profile::from_figures(FlightClass::OneWayDrone, &f);
        let mut missiles = 0u32;
        let mut engaged = 0usize;
        for seed in 0..10 {
            let mut rng = ChaCha8Rng::seed_from_u64(seed);
            let mut sys = vec![
                AdSystem::from_figures("lrsam", "SM-2/6", V3::ZERO, &f),
                AdSystem::from_figures("mrsam", "ESSM", V3::ZERO, &f),
                AdSystem::from_figures("gun5in", "5-inch", V3::ZERO, &f),
                AdSystem::from_figures("gun", "CIWS", V3::ZERO, &f),
                AdSystem::from_figures("fighter", "CAP1", V3::new(-15_000.0, 30_000.0, 6_000.0), &f),
                AdSystem::from_figures("ewSoftKill", "EW", V3::ZERO, &f),
            ];
            let cruise = Profile::from_figures(FlightClass::Cruise, &f);
            let raid: Vec<Threat> = (0..20)
        .map(|i| {
            // Drones and anti-ship cruise missiles, as in the Red Sea [R6.2].
            let (class, prof, cost) = if i % 2 == 0 { (ThreatClass::OneWayDrone, owa.clone(), 2.0e4) } else { (ThreatClass::Cruise, cruise.clone(), 5.0e5) };
            Threat { id: i, class, flight: Flight::new(prof, V3::new(-20_000.0 + 2_000.0 * i as f64, 150_000.0, 0.0), V3::ZERO, 45.0 * i as f64), cost, value: 5.0e5 }
        })
        .collect();
            let out = engage_raid(&mut sys, &raid, &chain, Assignment::Optimal, 1.0, &mut rng);
            missiles += out.shots_by_system["SM-2/6"] + out.shots_by_system["ESSM"];
            engaged += out.threats.iter().filter(|t| t.shots > 0).count();
        }
        let per = missiles as f64 / engaged as f64;
        assert!((0.3..=1.0).contains(&per), "missiles per engagement {per}");
        assert!(should_withdraw_to_rearm(20, 96, f.get("naval.reload.withdraw_fraction")));
        assert_eq!(reload_hours(10, false, false, &f), None);
    }

    #[test]
    fn damage_control_matters() {
        let f = figs();
        let p = DamageParams::from_figures(&f);
        let run = |dc: u32| {
            let mut rng = ChaCha8Rng::seed_from_u64(3);
            let mut s = ShipDamage::combatant(dc);
            s.hit(3, 1.0, &p, &mut rng);
            s.hit(2, 1.0, &p, &mut rng);
            for _ in 0..60 {
                s.step(1.0, &p, &mut rng);
            }
            s.state(&p)
        };
        let none = run(0);
        let good = run(3);
        assert!(good.flooding < none.flooding);
        assert!(good.fires <= none.fires);
    }

    #[test]
    fn mines() {
        let f = figs();
        let merchant = InfluenceSignature { magnetic: 1.0, acoustic: 1.0, pressure: 1.0, beam: 30.0 };
        let degaussed = InfluenceSignature { magnetic: 0.2, ..merchant };
        let mut field = Minefield { kind: MineKind::Magnetic, density: 2.0, cleared: 0.0 };
        let p0 = field.actuation_probability(&merchant, 5_000.0, &f);
        assert!(field.actuation_probability(&degaussed, 5_000.0, &f) < p0);
        field.clear(true, 2.0, 10.0, 20.0, &f);
        assert!(field.actuation_probability(&merchant, 5_000.0, &f) < p0);
    }
}
