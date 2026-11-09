//! Synthetic performance records for tests and vignettes. These are
//! catalogue data, not model figures: the real records come from
//! `bgws-catalogue` (§14.1). Values are open-source-plausible **[default]**.

use super::aircraft::AircraftPerf;
use super::drone::MultirotorPerf;

/// A generic fourth-generation single-engine fighter.
pub fn fighter() -> AircraftPerf {
    AircraftPerf {
        name: "generic/fighter".into(),
        empty_mass: 8_900.0,
        fuel_capacity: 3_200.0,
        wing_area: 27.87,
        cd0: 0.020,
        k_induced: 0.124,
        cl_max: 1.6,
        mach_crit: 0.9,
        wave_drag_rise: 0.5,
        max_thrust_sl: 129_000.0,
        tsfc_sl: 2.5e-5,
        n_max: 9.0,
        mach_max: 2.0,
        ceiling: 15_000.0,
    }
}

/// A generic twin-engine transport / tanker.
pub fn transport() -> AircraftPerf {
    AircraftPerf {
        name: "generic/transport".into(),
        empty_mass: 80_000.0,
        fuel_capacity: 60_000.0,
        wing_area: 280.0,
        cd0: 0.022,
        k_induced: 0.045,
        cl_max: 1.5,
        mach_crit: 0.78,
        wave_drag_rise: 0.6,
        max_thrust_sl: 400_000.0,
        tsfc_sl: 1.6e-5,
        n_max: 2.5,
        mach_max: 0.86,
        ceiling: 12_500.0,
    }
}

/// A generic small reconnaissance quadcopter (Mavic class).
pub fn quadcopter() -> MultirotorPerf {
    MultirotorPerf {
        name: "generic/quadIsr".into(),
        empty_mass: 0.9,
        max_payload: 0.5,
        battery_wh: 60.0,
        disk_area: 0.181,
        figure_of_merit: 0.45,
        drag_area: 0.02,
        avionics_w: 8.0,
        max_airspeed: 21.0,
    }
}

/// A generic heavy bomber multirotor (Baba Yaga class).
pub fn heavy_multirotor() -> MultirotorPerf {
    MultirotorPerf {
        name: "generic/bomberMultirotor".into(),
        empty_mass: 25.0,
        max_payload: 15.0,
        battery_wh: 1_800.0,
        disk_area: 2.0,
        figure_of_merit: 0.5,
        drag_area: 0.25,
        avionics_w: 40.0,
        max_airspeed: 20.0,
    }
}
