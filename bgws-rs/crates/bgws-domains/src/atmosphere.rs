//! The International Standard Atmosphere (ISA, identical to the US Standard
//! Atmosphere 1976 below 86 km) for the air and missile models (§8.1, §7.3):
//! temperature, pressure, density and speed of sound by geopotential layer,
//! with an exponential tail above 86 km for re-entry drag.
//!
//! These are physical standards, not tunable figures (brief rule 3). A
//! temperature deviation (ISA+ΔT) lets the weather model warm or cool the
//! column; pressure is kept standard, so density follows the gas law.

use serde::{Deserialize, Serialize};

/// Sea-level standard pressure, Pa.
pub const P0: f64 = 101_325.0;
/// Sea-level standard temperature, K.
pub const T0: f64 = 288.15;
/// Sea-level standard density, kg/m³.
pub const RHO0: f64 = 1.225;
/// Specific gas constant for dry air, J/(kg·K).
pub const R_AIR: f64 = 287.052_87;
/// Ratio of specific heats for air.
pub const GAMMA_AIR: f64 = 1.4;
/// Standard gravity, m/s².
pub const G0: f64 = bgws_core::units::G0;
/// Earth radius used for the geopotential-height conversion, m.
const R_GEOPOT: f64 = 6_356_766.0;

/// US-1976 layer bases (geopotential metres) and lapse rates (K/m).
const LAYERS: [(f64, f64); 7] = [
    (0.0, -0.0065),
    (11_000.0, 0.0),
    (20_000.0, 0.001),
    (32_000.0, 0.0028),
    (47_000.0, 0.0),
    (51_000.0, -0.0028),
    (71_000.0, -0.002),
];
const TOP: f64 = 84_852.0;

/// The state of the air at a height.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Air {
    /// Temperature, K.
    pub temperature: f64,
    /// Static pressure, Pa.
    pub pressure: f64,
    /// Density, kg/m³.
    pub density: f64,
    /// Speed of sound, m/s.
    pub speed_of_sound: f64,
}

impl Air {
    /// Dynamic pressure q = ½ρV², Pa.
    pub fn dynamic_pressure(&self, v: f64) -> f64 {
        0.5 * self.density * v * v
    }
    /// Mach number for a true airspeed.
    pub fn mach(&self, v: f64) -> f64 {
        v / self.speed_of_sound
    }
    /// Density ratio σ = ρ/ρ₀.
    pub fn sigma(&self) -> f64 {
        self.density / RHO0
    }
}

/// Standard air at geometric height `h` metres above mean sea level.
pub fn isa(h: f64) -> Air {
    isa_dev(h, 0.0)
}

/// Air at height `h` with a temperature deviation `dt` kelvin from standard.
pub fn isa_dev(h: f64, dt: f64) -> Air {
    let h = h.max(-500.0);
    // Geopotential height.
    let hp = R_GEOPOT * h / (R_GEOPOT + h);
    let (t_std, p) = if hp <= TOP { layer_state(hp) } else { tail_state(hp) };
    let t = (t_std + dt).max(150.0);
    let rho = p / (R_AIR * t);
    Air { temperature: t, pressure: p, density: rho, speed_of_sound: (GAMMA_AIR * R_AIR * t).sqrt() }
}

fn layer_state(hp: f64) -> (f64, f64) {
    let mut t_b = T0;
    let mut p_b = P0;
    for (i, &(h_b, lapse)) in LAYERS.iter().enumerate() {
        let h_top = LAYERS.get(i + 1).map_or(TOP, |l| l.0);
        // Negative only below sea level in the first layer, which is fine.
        let dh = hp.min(h_top) - h_b;
        let (t, p) = layer_step(t_b, p_b, lapse, dh);
        if hp <= h_top {
            return (t, p);
        }
        t_b = t;
        p_b = p;
    }
    (t_b, p_b)
}

fn layer_step(t_b: f64, p_b: f64, lapse: f64, dh: f64) -> (f64, f64) {
    if lapse.abs() < 1e-12 {
        (t_b, p_b * (-G0 * dh / (R_AIR * t_b)).exp())
    } else {
        let t = t_b + lapse * dh;
        (t, p_b * (t / t_b).powf(-G0 / (lapse * R_AIR)))
    }
}

/// Above 84.852 km geopotential: isothermal-ish exponential decay with a
/// 6 km scale height, enough for re-entry drag onset.
fn tail_state(hp: f64) -> (f64, f64) {
    let (t, p) = layer_state(TOP);
    (t, p * (-(hp - TOP) / 6_000.0).exp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn standard_table_values() {
        let s = isa(0.0);
        assert_relative_eq!(s.density, RHO0, epsilon = 1e-3);
        assert_relative_eq!(s.speed_of_sound, 340.29, epsilon = 0.05);
        // 11 km: T = 216.65 K, p ≈ 22,632 Pa, ρ ≈ 0.3639 kg/m³ (geometric 11 km is slightly higher geopotential).
        let a = isa(11_019.0);
        assert_relative_eq!(a.temperature, 216.65, epsilon = 0.05);
        assert_relative_eq!(a.pressure, 22_632.0, epsilon = 15.0);
        // 20 km geometric: ρ ≈ 0.0889 kg/m³.
        assert_relative_eq!(isa(20_000.0).density, 0.0889, epsilon = 0.001);
        // 50 km: ρ ≈ 1.03e-3.
        assert_relative_eq!(isa(50_000.0).density, 1.027e-3, epsilon = 0.03e-3);
        assert!(isa(120_000.0).density < 1e-6);
    }

    #[test]
    fn density_decreases_monotonically() {
        let mut last = f64::INFINITY;
        for i in 0..200 {
            let r = isa(i as f64 * 600.0).density;
            assert!(r < last);
            last = r;
        }
        assert!(isa_dev(0.0, 20.0).density < isa(0.0).density);
    }
}
