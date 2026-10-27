//! The air domain's own figures (rule 3): the thresholds, intervals and
//! factors of the sim's air systems. The physics lives in `bgws-domains`
//! (`air.*`, `missile.*`, `ad.*`) and `bgws-models` (`sensing.radar.*`);
//! these are the sim's glue: tasking, decision cadence, the engagement
//! modifiers of the TS world layer (`world/domains/air.ts`) and the
//! airspace-control figures.

use bgws_core::{figure, Figure, Figures};
use bgws_domains::air::{SortieParams, Tempo};
use bgws_domains::air_defence::KillChain;

pub fn figures() -> Vec<Figure> {
    vec![
        // Tasking and sorties.
        figure!(
            "sim.air.tasking_s",
            600.0,
            "s",
            60.0,
            3_600.0,
            "default",
            Default,
            "air tasking cycle: idle flights get missions this often"
        ),
        figure!(
            "sim.air.surge",
            1.0,
            "bool",
            0.0,
            1.0,
            "P30",
            Default,
            "1 = surge tempo (24 h window), 0 = sustained"
        ),
        figure!(
            "sim.air.helo_turnaround_factor",
            0.5,
            "fraction",
            0.2,
            1.0,
            "default",
            Default,
            "helicopter turnaround as a share of a fighter's"
        ),
        figure!(
            "sim.air.bingo_fraction",
            0.25,
            "fraction",
            0.1,
            0.5,
            "default",
            Default,
            "fuel share at which a flight is at bingo"
        ),
        figure!(
            "sim.air.cap_depth",
            0.4,
            "fraction",
            0.1,
            0.9,
            "default",
            Default,
            "CAP station: this share of the way from home to the enemy"
        ),
        figure!(
            "sim.air.cap_radius_m",
            15_000.0,
            "m",
            3_000.0,
            60_000.0,
            "default",
            Default,
            "CAP orbit radius"
        ),
        figure!(
            "sim.air.cap_alt_m",
            8_000.0,
            "m",
            1_000.0,
            14_000.0,
            "default",
            Default,
            "CAP altitude"
        ),
        figure!(
            "sim.air.strike_alt_m",
            7_000.0,
            "m",
            100.0,
            14_000.0,
            "default",
            Default,
            "strike transit altitude"
        ),
        figure!(
            "sim.air.release_share",
            0.8,
            "fraction",
            0.3,
            1.0,
            "default",
            Default,
            "release at this share of the weapon's maximum range"
        ),
        figure!(
            "sim.air.station_share",
            0.15,
            "fraction",
            0.0,
            0.6,
            "default",
            Default,
            "AEW and tanker tracks: this share of the way to the enemy"
        ),
        figure!(
            "sim.air.helo_alt_m",
            150.0,
            "m",
            30.0,
            1_000.0,
            "default",
            Default,
            "helicopter transit height"
        ),
        figure!(
            "sim.air.land_m",
            3_000.0,
            "m",
            500.0,
            10_000.0,
            "default",
            Default,
            "a flight this close to its base lands"
        ),
        // Air-to-air and missile warning (world/domains/air.ts).
        figure!(
            "sim.air.wez_ask_s",
            60.0,
            "s",
            5.0,
            600.0,
            "default",
            Default,
            "least time between two shoot questions about one target"
        ),
        figure!(
            "sim.air.wez_share",
            0.8,
            "fraction",
            0.3,
            1.0,
            "default",
            Default,
            "a target inside this share of the missile's range is in the envelope"
        ),
        figure!(
            "sim.air.energy_falloff",
            0.5,
            "fraction",
            0.0,
            0.9,
            "default",
            Default,
            "Pk lost at maximum range (energy), quadratic in range share"
        ),
        figure!(
            "sim.air.defend_factor",
            0.4,
            "fraction",
            0.1,
            1.0,
            "default",
            Default,
            "Pk multiplier against a defending flight (notch, chaff, flares)"
        ),
        figure!(
            "sim.air.defend_s",
            20.0,
            "s",
            5.0,
            60.0,
            "default",
            Default,
            "how long a defensive manoeuvre lasts"
        ),
        figure!(
            "sim.air.low_factor",
            0.7,
            "fraction",
            0.2,
            1.0,
            "default",
            Default,
            "radar-guided Pk multiplier against a target under 300 m"
        ),
        figure!(
            "sim.air.low_agl_m",
            300.0,
            "m",
            50.0,
            1_000.0,
            "default",
            Default,
            "below this height a target is low for radar seekers"
        ),
        figure!(
            "sim.air.low_level_m",
            150.0,
            "m",
            30.0,
            500.0,
            "default",
            Default,
            "height a flight flies at when it goes low"
        ),
        figure!(
            "sim.air.see_ir_missile",
            0.3,
            "fraction",
            0.0,
            1.0,
            "default",
            Default,
            "chance a crew sees an IR missile without a warner"
        ),
        figure!(
            "sim.air.nctr_m",
            40_000.0,
            "m",
            5_000.0,
            120_000.0,
            "default",
            Default,
            "range inside which an air radar classifies a target (NCTR)"
        ),
        figure!(
            "sim.air.basket_m",
            1_500.0,
            "m",
            100.0,
            10_000.0,
            "default",
            Default,
            "seeker basket: the target must be this close to the intercept point"
        ),
        figure!(
            "sim.air.basket_share",
            0.2,
            "fraction",
            0.0,
            0.6,
            "default",
            Default,
            "a homing interceptor's basket grows by this share of its flight distance"
        ),
        figure!(
            "sim.air.detour_margin",
            1.2,
            "fraction",
            1.0,
            2.0,
            "default",
            Default,
            "a detour clears a threat ring by this factor"
        ),
        // Air defence.
        figure!(
            "sim.ad.ask_s",
            600.0,
            "s",
            30.0,
            3_600.0,
            "default",
            Default,
            "least time between two raid questions to one AD unit"
        ),
        figure!(
            "sim.ad.cheapest_cost_mult",
            2.0,
            "fraction",
            1.0,
            10.0,
            "R4.4",
            Low,
            "cost weight multiplier when a unit uses the cheapest interceptor that will do"
        ),
        figure!(
            "sim.ad.surest_cost_mult",
            0.0,
            "fraction",
            0.0,
            1.0,
            "default",
            Default,
            "cost weight multiplier when a unit uses the surest interceptor"
        ),
        figure!(
            "sim.ad.value_owa",
            1.0e6,
            "USD",
            1.0e4,
            1.0e8,
            "P11",
            Low,
            "value at risk from a leaking one-way drone"
        ),
        figure!(
            "sim.ad.value_cruise",
            5.0e6,
            "USD",
            1.0e5,
            1.0e9,
            "default",
            Default,
            "value at risk from a leaking cruise missile"
        ),
        figure!(
            "sim.ad.value_ballistic",
            2.0e7,
            "USD",
            1.0e5,
            1.0e9,
            "default",
            Default,
            "value at risk from a leaking ballistic missile"
        ),
        figure!(
            "sim.ad.value_aircraft",
            5.0e7,
            "USD",
            1.0e5,
            1.0e9,
            "default",
            Default,
            "value of denying an enemy aircraft"
        ),
        figure!(
            "sim.ad.value_rocket",
            2.0e5,
            "USD",
            1.0e3,
            1.0e8,
            "default",
            Default,
            "value at risk from a leaking rocket"
        ),
        figure!(
            "sim.ad.value_asm",
            1.0e7,
            "USD",
            1.0e5,
            1.0e9,
            "default",
            Default,
            "value at risk from a leaking anti-ship or air-to-ground missile"
        ),
        figure!(
            "sim.ad.silent_s",
            300.0,
            "s",
            30.0,
            3_600.0,
            "default",
            Default,
            "a radar shut down stays off this long"
        ),
        figure!(
            "sim.ad.relocate_m",
            1_500.0,
            "m",
            300.0,
            5_000.0,
            "default",
            Default,
            "how far an AD unit moves when it relocates its radar"
        ),
        figure!(
            "sim.ad.see_arm",
            0.6,
            "fraction",
            0.0,
            1.0,
            "default",
            Default,
            "chance a radar crew sees an anti-radiation missile coming"
        ),
        figure!(
            "sim.ad.arm_silent_cep_mult",
            20.0,
            "fraction",
            1.0,
            100.0,
            "default",
            Default,
            "an ARM whose radar went silent: CEP multiplier"
        ),
        figure!(
            "sim.ad.reload_s",
            900.0,
            "s",
            60.0,
            7_200.0,
            "default",
            Default,
            "reloading empty launchers from the unit's stores"
        ),
        // Radar sensing glue.
        figure!(
            "sim.radar.beam_deg",
            2.0,
            "deg",
            0.5,
            30.0,
            "default",
            Default,
            "azimuth beamwidth when the record gives none"
        ),
        figure!(
            "sim.radar.default_scan_s",
            5.0,
            "s",
            0.5,
            60.0,
            "default",
            Default,
            "scan period when the record gives none"
        ),
        figure!(
            "sim.radar.sigma_min_m",
            50.0,
            "m",
            5.0,
            1_000.0,
            "default",
            Default,
            "smallest 1-sigma error of a radar plot"
        ),
        figure!(
            "sim.radar.rcs_munition_m2",
            0.1,
            "m2",
            0.001,
            10.0,
            "default",
            Default,
            "RCS of a munition in flight with no record"
        ),
        figure!(
            "sim.radar.terrain_check_m",
            40_000.0,
            "m",
            0.0,
            200_000.0,
            "default",
            Default,
            "terrain masking is checked for low targets inside this range"
        ),
        figure!(
            "sim.irst.rate_per_s",
            0.3,
            "1/s",
            0.01,
            5.0,
            "default",
            Default,
            "IRST/EO detection rate against an air target in reach"
        ),
        // Impacts.
        figure!(
            "sim.impact.footprint_m",
            150.0,
            "m",
            20.0,
            1_000.0,
            "default",
            Default,
            "radius an element's platforms are spread over"
        ),
        // Airspace control.
        figure!(
            "sim.airspace.misid_per_min",
            0.05,
            "1/min",
            0.0,
            1.0,
            "default",
            Default,
            "chance per minute an AD unit takes an own aircraft outside its corridor for hostile"
        ),
        figure!(
            "sim.airspace.fratricide_pk",
            0.5,
            "fraction",
            0.0,
            1.0,
            "default",
            Default,
            "kill chance of a fratricidal engagement"
        ),
    ]
}

/// The figures as one bundle, built once.
#[derive(Clone, Debug)]
pub struct AirParams {
    pub tasking_s: f64,
    pub tempo: Tempo,
    pub sortie: SortieParams,
    pub helo_turnaround_factor: f64,
    pub bingo: f64,
    pub cap_depth: f64,
    pub cap_radius: f64,
    pub cap_alt: f64,
    pub strike_alt: f64,
    pub release_share: f64,
    pub station_share: f64,
    pub helo_alt: f64,
    pub land_m: f64,
    pub wez_ask_s: f64,
    pub wez_share: f64,
    pub energy_falloff: f64,
    pub defend_factor: f64,
    pub defend_s: f64,
    pub low_factor: f64,
    pub low_agl: f64,
    pub low_level: f64,
    pub see_ir_missile: f64,
    pub nctr_m: f64,
    pub basket_m: f64,
    pub basket_share: f64,
    pub detour_margin: f64,
    pub ad_ask_s: f64,
    pub cheapest_mult: f64,
    pub surest_mult: f64,
    pub value_owa: f64,
    pub value_cruise: f64,
    pub value_ballistic: f64,
    pub value_aircraft: f64,
    pub value_rocket: f64,
    pub value_asm: f64,
    pub silent_s: f64,
    pub relocate_m: f64,
    pub see_arm: f64,
    pub arm_silent_cep_mult: f64,
    pub reload_s: f64,
    pub beam_deg: f64,
    pub default_scan_s: f64,
    pub sigma_min: f64,
    pub rcs_munition: f64,
    pub terrain_check_m: f64,
    pub irst_rate: f64,
    pub footprint_m: f64,
    pub misid_per_min: f64,
    pub fratricide_pk: f64,
    pub chain: KillChain,
    pub air: bgws_domains::air::AirParams,
    pub paths: bgws_domains::air::PathParams,
    pub radar: bgws_models::sensing::radar::RadarParams,
}

impl AirParams {
    pub fn from_figures(f: &Figures) -> Self {
        let tempo = if f.get("sim.air.surge") >= 0.5 {
            Tempo::Surge
        } else {
            Tempo::Sustained
        };
        AirParams {
            tasking_s: f.get("sim.air.tasking_s"),
            tempo,
            sortie: SortieParams::from_figures(f, tempo),
            helo_turnaround_factor: f.get("sim.air.helo_turnaround_factor"),
            bingo: f.get("sim.air.bingo_fraction"),
            cap_depth: f.get("sim.air.cap_depth"),
            cap_radius: f.get("sim.air.cap_radius_m"),
            cap_alt: f.get("sim.air.cap_alt_m"),
            strike_alt: f.get("sim.air.strike_alt_m"),
            release_share: f.get("sim.air.release_share"),
            station_share: f.get("sim.air.station_share"),
            helo_alt: f.get("sim.air.helo_alt_m"),
            land_m: f.get("sim.air.land_m"),
            wez_ask_s: f.get("sim.air.wez_ask_s"),
            wez_share: f.get("sim.air.wez_share"),
            energy_falloff: f.get("sim.air.energy_falloff"),
            defend_factor: f.get("sim.air.defend_factor"),
            defend_s: f.get("sim.air.defend_s"),
            low_factor: f.get("sim.air.low_factor"),
            low_agl: f.get("sim.air.low_agl_m"),
            low_level: f.get("sim.air.low_level_m"),
            see_ir_missile: f.get("sim.air.see_ir_missile"),
            nctr_m: f.get("sim.air.nctr_m"),
            basket_m: f.get("sim.air.basket_m"),
            basket_share: f.get("sim.air.basket_share"),
            detour_margin: f.get("sim.air.detour_margin"),
            ad_ask_s: f.get("sim.ad.ask_s"),
            cheapest_mult: f.get("sim.ad.cheapest_cost_mult"),
            surest_mult: f.get("sim.ad.surest_cost_mult"),
            value_owa: f.get("sim.ad.value_owa"),
            value_cruise: f.get("sim.ad.value_cruise"),
            value_ballistic: f.get("sim.ad.value_ballistic"),
            value_aircraft: f.get("sim.ad.value_aircraft"),
            value_rocket: f.get("sim.ad.value_rocket"),
            value_asm: f.get("sim.ad.value_asm"),
            silent_s: f.get("sim.ad.silent_s"),
            relocate_m: f.get("sim.ad.relocate_m"),
            see_arm: f.get("sim.ad.see_arm"),
            arm_silent_cep_mult: f.get("sim.ad.arm_silent_cep_mult"),
            reload_s: f.get("sim.ad.reload_s"),
            beam_deg: f.get("sim.radar.beam_deg"),
            default_scan_s: f.get("sim.radar.default_scan_s"),
            sigma_min: f.get("sim.radar.sigma_min_m"),
            rcs_munition: f.get("sim.radar.rcs_munition_m2"),
            terrain_check_m: f.get("sim.radar.terrain_check_m"),
            irst_rate: f.get("sim.irst.rate_per_s"),
            footprint_m: f.get("sim.impact.footprint_m"),
            misid_per_min: f.get("sim.airspace.misid_per_min"),
            fratricide_pk: f.get("sim.airspace.fratricide_pk"),
            chain: KillChain::from_figures(f),
            air: bgws_domains::air::AirParams::from_figures(f),
            paths: bgws_domains::air::PathParams::from_figures(f),
            radar: bgws_models::sensing::radar::RadarParams::from_figures(f),
        }
    }
}
