//! The environment (RUST_ENGINE.md §7, §14.2): terrain grids and their
//! pyramid, land cover and micro-terrain, soil and trafficability, road and
//! river networks with routing, the change layer, the runtime tile format
//! and theatre packs, a deterministic synthetic theatre, the sea
//! (bathymetry, sea state, sound-speed profiles), weather fields with
//! their sources, atmospheric transmission, sun and moon, illuminance and
//! the diurnal thermal model.
//!
//! Model code is pure: only `tiles` reads and writes files, and only when
//! asked to. Every coefficient is a declared figure (`env.*`).

pub mod change;
pub mod landcover;
pub mod light;
pub mod network;
pub mod optics;
pub mod raster;
pub mod sea;
pub mod soil;
pub mod synth;
pub mod terrain;
pub mod thermal;
pub mod tiles;
pub mod weather;

pub use change::{Change, ChangeKind, ChangeLayer, ChangeParams, ObstacleKind};
pub use landcover::{CoverProps, LandCover, LandCoverTable, MicroTerrain};
pub use light::{illuminance, illuminance_at, moon_position, sun_position, Illuminance, LightParams, MoonPosition, SunPosition, TwilightPhase, UtcDateTime};
pub use network::{cost_astar, Bridge, GridPath, PowerLine, River, RoadClass, RoadNetwork, RoadSpeeds, RoadUser, Route};
pub use optics::{band_extinction, extinction_from_visibility, rain_attenuation_db_per_km, transmission, AtmosParams, OpticalBand, Polarisation};
pub use raster::{GridSpec, Raster};
pub use sea::{Bathymetry, OceanRegion, SeaStateField, Season, SoundSpeedProfile, WaterColumn};
pub use soil::{trafficability, update_wetness, CellGoing, SoilType, TrafficParams, Trafficability, VehicleClass};
pub use synth::{generate, SynthParams, SynthWorld, Town};
pub use terrain::{Pyramid, Terrain};
pub use thermal::{thermal_day, DiurnalWeather, Surface, ThermalDay, ThermalParams, VehicleThermalState};
pub use tiles::{read_tile, write_tile, Aoi, Codec, TheatrePack, TileError};
pub use weather::{WeatherCell, WeatherField, WeatherPreset};

use bgws_core::{Figure, Figures};

/// Every figure this crate declares.
pub fn figures() -> Vec<Figure> {
    let mut v = Vec::new();
    v.extend(landcover::figures());
    v.extend(soil::figures());
    v.extend(network::figures());
    v.extend(change::figures());
    v.extend(synth::figures());
    v.extend(sea::figures());
    v.extend(weather::figures());
    v.extend(optics::figures());
    v.extend(light::figures());
    v.extend(thermal::figures());
    v
}

/// A registry of this crate's figures at their declared values.
pub fn default_figures() -> Figures {
    Figures::from_list(figures()).expect("bgws-env figures are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_build() {
        let f = Figures::from_list(figures()).unwrap();
        assert!(f.len() > 150);
        // Every params struct reads cleanly.
        let _ = LandCoverTable::from_figures(&f);
        let _ = TrafficParams::from_figures(&f);
        let _ = RoadSpeeds::from_figures(&f);
        let _ = network::FordLimits::from_figures(&f);
        let _ = ChangeParams::from_figures(&f);
        let _ = sea::WaveParams::from_figures(&f);
        let _ = AtmosParams::from_figures(&f);
        let _ = LightParams::from_figures(&f);
        let _ = ThermalParams::from_figures(&f);
        let _ = SynthParams::new(1, 10, 10, 30.0, &f);
        let _ = weather::Climatology::for_place(50.0, 100, &f);
        let _ = f.get("env.sea.cz_vertex_margin_m_s");
        let _ = f.get("env.weather.front_speed_m_s");
        let _ = generate(&SynthParams::new(1, 20, 20, 30.0, &f), &f);
        let p = weather::SynthWeatherParams {
            seed: 1,
            spec: GridSpec::new(2, 2, 1000.0, 0.0, 0.0),
            start: UtcDateTime::new(2024, 1, 1, 0, 0, 0.0),
            lat: 50.0,
            lon: 0.0,
            duration_h: 1.0,
            step_h: 1.0,
            fronts: 1,
        };
        let _ = weather::synthetic(&p, &f);
        assert!(f.unused().is_empty(), "declared but unused: {:?}", f.unused());
    }
}
