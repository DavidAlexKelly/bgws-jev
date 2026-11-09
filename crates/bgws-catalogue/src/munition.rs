//! Munitions (RUST_ENGINE.md §8.8, §14.1): what a mount fires. Ported from
//! the TS `WeaponSpec` (the flight, guidance and kill half of it; the mount
//! half lives on `Mount`) and extended with a penetration-versus-range table,
//! a Carleton damage function for area effects, the seeker, warhead mass and
//! the structural-hit weight that weapons.ts kept in a side table.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MunitionKind {
    /// Kinetic-energy penetrator (APFSDS, APDS).
    Ke,
    /// Shaped charge, unguided (RPG, HEAT round).
    Heat,
    /// High-explosive, direct or area.
    He,
    /// Guided anti-tank missile.
    Atgm,
    Sam,
    Aam,
    /// Anti-ship missile.
    Asm,
    /// Air-to-ground missile.
    Agm,
    Cruise,
    Ballistic,
    Loitering,
    Fpv,
    SmallArms,
    /// A gun shell (artillery, naval, CIWS, air-defence cannon).
    Shell,
    Mortar,
    Rocket,
    Torpedo,
    GlideBomb,
    Bomb,
    /// Anti-radiation missile.
    Arm,
    /// One-way attack drone (Shahed-like).
    Owa,
    /// Interceptor drone.
    Interceptor,
    /// A grenade or bomblet dropped by a drone.
    DroneDrop,
}

impl MunitionKind {
    /// Defeats armour with a shaped charge (CE) rather than kinetic energy.
    pub fn is_chemical_energy(self) -> bool {
        matches!(self, MunitionKind::Heat | MunitionKind::Atgm | MunitionKind::Fpv | MunitionKind::Loitering | MunitionKind::Agm | MunitionKind::DroneDrop)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Guidance {
    None,
    Gnss,
    Laser,
    Ir,
    Radar,
    ActiveRadar,
    AntiRadiation,
    Operator,
    Acoustic,
    Inertial,
    Wire,
    Fibre,
}

impl Guidance {
    /// Degraded by GNSS jamming.
    pub fn uses_gnss(self) -> bool {
        matches!(self, Guidance::Gnss)
    }
    /// Its control link can be jammed (radio-controlled operator guidance).
    pub fn link_jammable(self) -> bool {
        matches!(self, Guidance::Operator)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlightProfile {
    None,
    Ballistic,
    Cruise,
    Pursuit,
    SeaSkim,
    Glide,
    Underwater,
}

/// What a munition can be aimed at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TargetDomain {
    Land,
    Air,
    Surface,
    Subsurface,
    Munition,
}

/// A terminal seeker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Seeker {
    /// "ir", "iir", "radar", "laser", "eo", "antiRadiation", "acoustic".
    pub kind: String,
    /// Lock-on range against its usual target, m.
    pub acquisition_m: f64,
    /// Seeker field of view, degrees.
    pub fov_deg: f64,
}

/// One row of a penetration table.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PenPoint {
    pub range_m: f64,
    /// Penetration, mm RHA.
    pub mm: f64,
}

/// The Carleton damage function: P(x, y) = exp(−(x²/bx² + y²/by²)), with
/// x along the line of fire and y across it; its integral is the lethal
/// area π·bx·by (the classic artillery effectiveness form, [default]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Carleton {
    /// Range-direction scale, m.
    pub bx_m: f64,
    /// Deflection-direction scale, m.
    pub by_m: f64,
}

impl Carleton {
    /// Kill probability for a single target at (dx along, dy across), m.
    pub fn pk(&self, dx: f64, dy: f64) -> f64 {
        if self.bx_m <= 0.0 || self.by_m <= 0.0 {
            return 0.0;
        }
        (-(dx * dx / (self.bx_m * self.bx_m) + dy * dy / (self.by_m * self.by_m))).exp()
    }
    /// The lethal area, m²: the integral of `pk` over the plane.
    pub fn lethal_area_m2(&self) -> f64 {
        std::f64::consts::PI * self.bx_m * self.by_m
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Munition {
    pub id: String,
    pub name: String,
    pub kind: MunitionKind,
    /// The TS ammunition family it draws on (`tank120`, `artillery155`).
    pub family: String,
    pub guidance: Guidance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seeker: Option<Seeker>,
    pub flight: FlightProfile,
    /// Average speed in flight, m/s (0 for rounds resolved in the tick).
    pub speed_ms: f64,
    pub min_range_m: f64,
    pub max_range_m: f64,
    /// Chance one round kills a target it reaches and hits, against its usual target.
    pub pk: f64,
    /// Circular error probable, m (guided: fixed; unguided: at max range).
    pub cep_m: f64,
    #[serde(default)]
    pub warhead_kg: f64,
    /// Lethal radius against people and soft targets, m (TS figure).
    #[serde(default)]
    pub lethal_radius_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carleton: Option<Carleton>,
    /// Penetration (mm RHA) against range (m), ascending in range. Empty: blast only.
    #[serde(default)]
    pub penetration: Vec<PenPoint>,
    #[serde(default)]
    pub top_attack: bool,
    /// Tandem shaped charge: defeats ERA.
    #[serde(default)]
    pub tandem: bool,
    pub targets: Vec<TargetDomain>,
    pub cost_usd: f64,
    /// Standard 200 kg-warhead hits one round is worth against ships and structures.
    #[serde(default)]
    pub structural_hits: f64,
    /// No warhead (a decoy drone).
    #[serde(default)]
    pub decoy: bool,
    #[serde(default)]
    pub source: String,
}

impl Munition {
    /// Penetration at a range, mm RHA: linear interpolation in the table,
    /// held flat beyond its ends. Zero for blast-only munitions.
    pub fn penetration_at(&self, range_m: f64) -> f64 {
        let t = &self.penetration;
        let Some(first) = t.first() else { return 0.0 };
        if range_m <= first.range_m {
            return first.mm;
        }
        for w in t.windows(2) {
            let (a, b) = (w[0], w[1]);
            if range_m <= b.range_m {
                let f = if b.range_m > a.range_m { (range_m - a.range_m) / (b.range_m - a.range_m) } else { 1.0 };
                return a.mm + f * (b.mm - a.mm);
            }
        }
        t.last().map(|p| p.mm).unwrap_or(0.0)
    }

    /// Does this round's warhead defeat the armour at a range? Returns the
    /// margin (penetration − armour, mm) for the AAR (rule 10).
    pub fn penetration_margin(&self, range_m: f64, armour_mm: f64) -> f64 {
        self.penetration_at(range_m) - armour_mm
    }

    /// Lethal area, m²: from the Carleton function, else a disc of the lethal radius.
    pub fn lethal_area_m2(&self) -> f64 {
        match self.carleton {
            Some(c) => c.lethal_area_m2(),
            None => std::f64::consts::PI * self.lethal_radius_m * self.lethal_radius_m,
        }
    }

    pub fn can_target(&self, d: TargetDomain) -> bool {
        self.targets.contains(&d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn apfsds() -> Munition {
        Munition {
            id: "t".into(),
            name: "t".into(),
            kind: MunitionKind::Ke,
            family: "tank120".into(),
            guidance: Guidance::None,
            seeker: None,
            flight: FlightProfile::None,
            speed_ms: 1650.0,
            min_range_m: 0.0,
            max_range_m: 4000.0,
            pk: 0.6,
            cep_m: 0.0,
            warhead_kg: 0.0,
            lethal_radius_m: 0.0,
            carleton: Some(Carleton { bx_m: 10.0, by_m: 5.0 }),
            penetration: vec![PenPoint { range_m: 0.0, mm: 700.0 }, PenPoint { range_m: 2000.0, mm: 600.0 }],
            top_attack: false,
            tandem: false,
            targets: vec![TargetDomain::Land],
            cost_usd: 8000.0,
            structural_hits: 0.03,
            decoy: false,
            source: String::new(),
        }
    }

    #[test]
    fn penetration_interpolates_and_holds_flat() {
        let m = apfsds();
        assert_relative_eq!(m.penetration_at(1000.0), 650.0);
        assert_relative_eq!(m.penetration_at(-5.0), 700.0);
        assert_relative_eq!(m.penetration_at(9000.0), 600.0);
        assert_relative_eq!(m.penetration_margin(1000.0, 600.0), 50.0);
    }

    #[test]
    fn carleton_integrates_to_its_lethal_area() {
        let c = Carleton { bx_m: 20.0, by_m: 10.0 };
        // Numerical integral over a wide grid equals π·bx·by.
        let h = 0.5;
        let mut sum = 0.0;
        let mut x = -100.0;
        while x <= 100.0 {
            let mut y = -60.0;
            while y <= 60.0 {
                sum += c.pk(x, y) * h * h;
                y += h;
            }
            x += h;
        }
        assert_relative_eq!(sum, c.lethal_area_m2(), max_relative = 1e-3);
        assert_relative_eq!(c.pk(0.0, 0.0), 1.0);
    }
}
