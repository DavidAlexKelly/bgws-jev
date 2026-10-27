//! Propagation (RUST_ENGINE.md §8.4): optical line of sight with curvature,
//! refraction and probabilistic canopy masking; radar and radio horizons;
//! VHF/UHF path loss (free space, Deygout multiple knife-edge diffraction,
//! clutter, a simplified ITM); HF ground wave and NVIS skywave; satellite and
//! microwave links with rain; acoustic transmission loss in water and air;
//! and the receiver budget that turns any of them into SINR, BER, message
//! success and throughput.
//!
//! Terrain LOS goes through `bgws_gpu::Kernels`, so callers choose the CPU
//! twin or the GPU at run time; everything else is CPU. Every coefficient
//! is a declared figure (`prop.*`).

pub mod acoustic;
pub mod budget;
pub mod hf;
pub mod horizon;
pub mod links;
pub mod los;
pub mod radio;

pub use acoustic::{air_tl, thorp_db_per_km, water_tl, AcousticParams, AcousticPath, AirTl, WaterGeometry, WaterTl};
pub use budget::{ber, link_budget, noise_dbw, BudgetInput, BudgetResult, Jammer, Waveform};
pub use hf::{ground_wave, nvis, GroundType, GroundWave, NvisParams, NvisResult};
pub use horizon::{radar_horizon, shadow_ceiling, terrain_horizon_elevation, HorizonResult};
pub use links::{microwave_link, sat_link, LinkParams, MicrowaveLink, SatBand, SatLink};
pub use los::{height_grid, optical_los, optical_los_batch, OpticalLos, OpticalParams, TerrainGrid};
pub use radio::{deygout, fresnel_v, fspl_db, itm_loss, knife_edge_j, terrain_path_loss, ItmInput, ItmParams, ItmResult, PathLoss, Profile, RadioParams, Terminal};

use bgws_core::{figure, Figure, Figures};
use bgws_env::LandCover;

/// Figures this crate declares (its own `prop.*` keys only).
pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("prop.optical.k_factor", 1.15, "1", 1.0, 1.4, "default", Default, "effective Earth radius factor for optical refraction"),
        figure!("prop.optical.endpoint_clear_m", 15.0, "m", 0.0, 100.0, "R4.1", Default, "canopy this close to an observer or target is ignored (edge of a wood)"),
        figure!("prop.radio.k_factor", 4.0 / 3.0, "1", 1.0, 2.0, "default", Medium, "effective Earth radius factor for radio (4/3 standard atmosphere)"),
        figure!("prop.radio.profile_step_m", 30.0, "m", 5.0, 500.0, "default", Default, "terrain profile sampling for path loss"),
        figure!("prop.radio.deygout_depth", 3.0, "1", 1.0, 5.0, "default", Default, "Deygout recursion depth (1 = single edge)"),
        figure!("prop.clutter.freq_exp", 0.3, "1", 0.0, 1.0, "default", Default, "frequency scaling of clutter loss (f/1 GHz)^e"),
        figure!("prop.itm.knife_weight_dh_m", 50.0, "m", 5.0, 500.0, "default", Default, "Δh at which knife-edge and smooth-sphere diffraction weigh equally"),
        figure!("prop.itm.blend_start", 0.5, "fraction", 0.1, 0.95, "default", Default, "fraction of the horizon distance where the LOS-to-diffraction blend starts"),
        figure!("prop.itm.sigma_per_dh", 0.39, "1", 0.1, 1.0, "default", Low, "terrain rms height per unit Δh (interdecile of a Gaussian ≈ 2.56σ)"),
        figure!("prop.itm.horizon_dh_coeff", 0.07, "1", 0.0, 0.3, "default", Low, "ITM horizon reduction exp(−c·√(Δh/h))"),
        figure!("prop.nvis.day_ceiling_mhz", 10.0, "MHz", 6.0, 14.0, "P5", Medium, "NVIS usable ceiling by day, solar maximum"),
        figure!("prop.nvis.night_ceiling_mhz", 6.5, "MHz", 3.0, 9.0, "P5", Medium, "NVIS usable ceiling at night (R2: 6–7 MHz)"),
        figure!("prop.nvis.day_luf_mhz", 3.0, "MHz", 1.5, 5.0, "default", Default, "lowest usable NVIS frequency by day (D-layer absorption)"),
        figure!("prop.nvis.night_luf_mhz", 2.0, "MHz", 1.0, 3.0, "P5", Medium, "lowest usable NVIS frequency at night"),
        figure!("prop.nvis.min_range_km", 35.0, "km", 0.0, 100.0, "P5", Medium, "NVIS minimum planning range"),
        figure!("prop.nvis.max_range_km", 450.0, "km", 200.0, 800.0, "P5", Medium, "NVIS maximum planning range"),
        figure!("prop.nvis.layer_height_km", 300.0, "km", 150.0, 450.0, "default", Low, "virtual reflection height of the F layer"),
        figure!("prop.nvis.absorption_db_mhz2", 100.0, "dB*MHz2", 10.0, 500.0, "default", Default, "D-layer absorption at overhead sun, × 1/f²"),
        figure!("prop.nvis.solar_min_factor", 0.75, "fraction", 0.4, 1.0, "R2", Low, "NVIS ceiling at solar minimum relative to maximum"),
        figure!("prop.sat.rain_height_km", 4.0, "km", 1.0, 6.0, "default", Low, "rain height for slant-path rain fade (ITU-R P.839 mid-latitude)"),
        figure!("prop.sat.min_elevation_deg", 5.0, "deg", 0.0, 20.0, "default", Default, "minimum operating elevation"),
        figure!("prop.sat.gas_zenith_db", 0.05, "dB", 0.0, 1.0, "default", Default, "clear-air gaseous loss at zenith at 10 GHz"),
        figure!("prop.sat.p618_l0_km", 35.0, "km", 10.0, 60.0, "default", Medium, "ITU-R P.618 horizontal reduction L0 = c·e^(−0.015R)"),
        figure!("prop.mw.oxygen_peak_db_km", 15.0, "dB/km", 5.0, 20.0, "default", Medium, "oxygen absorption at the 60 GHz line"),
        figure!("prop.mw.background_gas_db_km", 0.01, "dB/km", 0.0, 0.2, "default", Default, "background gaseous absorption at 10 GHz, ∝ f"),
        figure!("prop.acoustic.cz_gain_db", 10.0, "dB", 0.0, 25.0, "default", Default, "focusing gain in a convergence zone over spherical spreading"),
        figure!("prop.acoustic.cz_width_fraction", 0.08, "fraction", 0.02, 0.2, "default", Default, "CZ annulus width as a fraction of its range"),
        figure!("prop.acoustic.direct_path_fraction", 0.2, "fraction", 0.05, 0.5, "default", Default, "direct-path zone as a fraction of the CZ range"),
        figure!("prop.acoustic.shadow_db", 25.0, "dB", 5.0, 60.0, "default", Default, "excess loss in the deep shadow (bottom-bounce floor)"),
        figure!("prop.acoustic.layer_loss_db", 20.0, "dB", 5.0, 40.0, "default", Default, "cross-layer loss below/above the sonic layer"),
        figure!("prop.acoustic.duct_leakage_db_km", 0.5, "dB/km", 0.0, 5.0, "default", Default, "surface duct leakage"),
        figure!("prop.acoustic.duct_cutoff_coeff", 0.0085, "1", 0.004, 0.02, "default", Low, "Urick: longest trapped wavelength 0.0085·D^1.5 (ft)"),
        figure!("prop.acoustic.shallow_depth_m", 200.0, "m", 50.0, 500.0, "default", Default, "water shallower than this uses shallow-water spreading"),
        figure!("prop.acoustic.bottom_loss_db_km", 0.5, "dB/km", 0.0, 5.0, "default", Default, "shallow-water bottom interaction loss"),
        figure!("prop.acoustic.air_excess_db", 3.0, "dB", 0.0, 15.0, "default", Default, "ground/excess attenuation in air beyond 50 m"),
        figure!("prop.acoustic.air_upwind_db_per_m_s_km", 2.0, "dB/(m/s)/km", 0.0, 10.0, "default", Default, "upwind refraction loss"),
        figure!("prop.acoustic.air_upwind_max_db", 20.0, "dB", 0.0, 40.0, "default", Default, "cap on the upwind shadow loss"),
        figure!("prop.acoustic.air_downwind_max_db", 5.0, "dB", 0.0, 15.0, "default", Default, "cap on the downwind gain"),
    ];
    // Clutter loss at 1 GHz for a terminal at ground level in each class.
    let clutter = [
        (LandCover::Open, 0.0),
        (LandCover::Crops, 2.0),
        (LandCover::Scrub, 4.0),
        (LandCover::ForestLight, 8.0),
        (LandCover::ForestDense, 15.0),
        (LandCover::UrbanLight, 12.0),
        (LandCover::UrbanDense, 20.0),
        (LandCover::Water, 0.0),
        (LandCover::Wetland, 1.0),
        (LandCover::Bare, 0.0),
    ];
    for (c, db) in clutter {
        v.push(figure!(format!("prop.clutter.{}_db", c.key()), db, "dB", 0.0, 40.0, "R2", Default, "terminal clutter loss at 1 GHz (DOT&E: dismounted terrain masking)"));
    }
    v
}

/// This crate's figures plus those it reads from `bgws-env` and `bgws-gpu`.
pub fn all_figures() -> Vec<Figure> {
    let mut v = figures();
    v.extend(bgws_env::figures());
    v.extend(bgws_gpu::figures());
    v
}

/// A registry of [`all_figures`] at their declared values.
pub fn default_figures() -> Figures {
    Figures::from_list(all_figures()).expect("figures are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_build_and_are_used() {
        let own = Figures::from_list(figures()).unwrap();
        let f = default_figures();
        let _ = OpticalParams::from_figures(&f);
        let _ = RadioParams::from_figures(&f);
        let _ = NvisParams::from_figures(&f);
        let _ = LinkParams::from_figures(&f);
        let _ = AcousticParams::from_figures(&f);
        let unused: Vec<String> = f.unused().into_iter().filter(|k| own.figure(k).is_some()).collect();
        assert!(unused.is_empty(), "declared but unused: {unused:?}");
    }
}
