//! Sortie generation (§8.1 Air, §8.11; acceptance check §15.3 **[P30]**:
//! fighters fly 2–3 sorties/day in a surge and 1–1.5 sustained).
//!
//! Sorties per aircraft per day are the minimum of three limits, each
//! computed from process figures, and the binding one is reported:
//!
//! - **aircraft cycle**: the operating window divided by the cycle time
//!   (sortie + turnaround + expected unscheduled fix time per sortie), times
//!   the mission-capable rate;
//! - **maintenance labour**: maintainer-hours available per day divided by the
//!   maintenance man-hours each sortie generates (less what a surge defers);
//! - **aircrew**: crews per aircraft × sorties a crew can fly per duty period
//!   (duty length over sortie-plus-brief, capped by the fatigue rule) × the
//!   fraction of days a crew can fly while keeping its rest.
//!
//! A small stochastic fleet simulation ([`simulate_fleet`]) checks the
//! analytic figure with random breaks and repair times.

use bgws_core::{figure, Figure, Figures};
use rand::Rng;
use rand_distr::{Distribution, Exp};
use serde::{Deserialize, Serialize};

/// Operating tempo.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tempo {
    Surge,
    Sustained,
}

impl Tempo {
    fn key(self) -> &'static str {
        match self {
            Tempo::Surge => "surge",
            Tempo::Sustained => "sustained",
        }
    }
}

/// Process figures for one tempo.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortieParams {
    pub sortie_h: f64,
    pub turnaround_h: f64,
    pub break_rate: f64,
    pub fix_h: f64,
    pub mission_capable: f64,
    pub ops_window_h: f64,
    pub maintainers_per_aircraft: f64,
    pub maintenance_shift_cover_h: f64,
    pub maintenance_productive: f64,
    pub mmh_per_fh: f64,
    pub maintenance_deferred: f64,
    pub crew_ratio: f64,
    pub crew_duty_h: f64,
    pub brief_h: f64,
    pub crew_max_sorties_per_duty: f64,
    pub crew_duty_days_fraction: f64,
}

impl SortieParams {
    pub fn from_figures(f: &Figures, tempo: Tempo) -> Self {
        let t = |k: &str| f.get(&format!("air.sortie.{}.{k}", tempo.key()));
        Self {
            sortie_h: f.get("air.sortie.duration_h"),
            turnaround_h: f.get("air.sortie.turnaround_h"),
            break_rate: f.get("air.sortie.break_rate"),
            fix_h: f.get("air.sortie.fix_h"),
            mission_capable: f.get("air.sortie.mission_capable"),
            ops_window_h: t("ops_window_h"),
            maintainers_per_aircraft: f.get("air.sortie.maintainers_per_aircraft"),
            maintenance_shift_cover_h: t("maintenance_cover_h"),
            maintenance_productive: f.get("air.sortie.maintenance_productive"),
            mmh_per_fh: f.get("air.sortie.mmh_per_fh"),
            maintenance_deferred: t("maintenance_deferred"),
            crew_ratio: f.get("air.sortie.crew_ratio"),
            crew_duty_h: t("crew_duty_h"),
            brief_h: f.get("air.sortie.brief_h"),
            crew_max_sorties_per_duty: t("crew_max_sorties"),
            crew_duty_days_fraction: t("crew_duty_days_fraction"),
        }
    }
}

pub fn figures() -> Vec<Figure> {
    let mut v = vec![
        figure!("air.sortie.duration_h", 2.0, "h", 0.75, 10.0, "default", Default, "fighter sortie block time"),
        figure!("air.sortie.turnaround_h", 1.25, "h", 0.5, 4.0, "default", Default, "refuel, rearm, through-flight checks"),
        figure!("air.sortie.break_rate", 0.12, "1/sortie", 0.0, 0.5, "default", Default, "aircraft returning broken"),
        figure!("air.sortie.fix_h", 5.0, "h", 1.0, 48.0, "default", Default, "mean unscheduled fix time"),
        figure!("air.sortie.mission_capable", 0.72, "fraction", 0.4, 0.95, "default", Default, "mission-capable rate"),
        figure!("air.sortie.maintainers_per_aircraft", 8.0, "count", 2.0, 25.0, "default", Default, "maintainers on shift per aircraft"),
        figure!("air.sortie.maintenance_productive", 0.55, "fraction", 0.3, 0.9, "default", Default, "wrench time per shift hour"),
        figure!("air.sortie.mmh_per_fh", 20.0, "h/h", 5.0, 60.0, "default", Default, "maintenance man-hours per flight hour"),
        figure!("air.sortie.crew_ratio", 1.25, "1", 0.8, 2.5, "default", Default, "aircrews per aircraft"),
        figure!("air.sortie.brief_h", 2.0, "h", 0.5, 4.0, "default", Default, "brief, step and debrief per sortie"),
    ];
    let per = [
        // (key, surge, sustained, unit, min, max, note)
        ("ops_window_h", 24.0, 14.0, "h", 6.0, 24.0, "flying window per day"),
        ("maintenance_cover_h", 24.0, 16.0, "h", 8.0, 24.0, "hours per day maintenance shifts cover"),
        ("maintenance_deferred", 0.4, 0.0, "fraction", 0.0, 0.7, "scheduled maintenance deferred"),
        ("crew_duty_h", 14.0, 12.0, "h", 8.0, 16.0, "crew duty period"),
        ("crew_max_sorties", 2.0, 1.0, "count", 1.0, 4.0, "fatigue cap on sorties per duty period"),
        ("crew_duty_days_fraction", 1.0, 0.857, "fraction", 0.5, 1.0, "share of days a crew flies (rest days)"),
    ];
    for (k, surge, sus, unit, lo, hi, note) in per {
        v.push(figure!(format!("air.sortie.surge.{k}"), surge, unit, lo, hi, "P30", Default, note));
        v.push(figure!(format!("air.sortie.sustained.{k}"), sus, unit, lo, hi, "P30", Default, note));
    }
    v
}

/// Which limit set the rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortieLimit {
    AircraftCycle,
    Maintenance,
    Aircrew,
}

/// The sortie rate and its explanation.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SortieGeneration {
    pub per_aircraft_per_day: f64,
    pub binding: SortieLimit,
    pub aircraft_limit: f64,
    pub maintenance_limit: f64,
    pub aircrew_limit: f64,
    pub cycle_h: f64,
}

/// Sorties per aircraft per day.
pub fn sortie_rate(p: &SortieParams) -> SortieGeneration {
    let cycle = p.sortie_h + p.turnaround_h + p.break_rate * p.fix_h;
    let aircraft = p.mission_capable * p.ops_window_h / cycle;
    let mmh_day = p.maintainers_per_aircraft * p.maintenance_shift_cover_h * p.maintenance_productive;
    let mmh_per_sortie = p.mmh_per_fh * p.sortie_h * (1.0 - p.maintenance_deferred);
    let maintenance = mmh_day / mmh_per_sortie;
    let per_duty = (p.crew_duty_h / (p.sortie_h + p.brief_h)).floor().min(p.crew_max_sorties_per_duty);
    let aircrew = p.crew_ratio * per_duty * p.crew_duty_days_fraction;
    let (rate, binding) = [(aircraft, SortieLimit::AircraftCycle), (maintenance, SortieLimit::Maintenance), (aircrew, SortieLimit::Aircrew)]
        .into_iter()
        .fold((f64::INFINITY, SortieLimit::AircraftCycle), |acc, x| if x.0 < acc.0 { x } else { acc });
    SortieGeneration { per_aircraft_per_day: rate, binding, aircraft_limit: aircraft, maintenance_limit: maintenance, aircrew_limit: aircrew, cycle_h: cycle }
}

/// Fleet simulation result.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetResult {
    pub sorties: u32,
    pub per_aircraft_per_day: f64,
}

/// A day-by-day Monte Carlo of `aircraft` airframes: each launches when it is
/// serviceable, inside the window, and a crew sortie and maintenance labour
/// remain in the day's budgets; breaks draw exponential repair times.
pub fn simulate_fleet(p: &SortieParams, aircraft: usize, days: usize, rng: &mut impl Rng) -> FleetResult {
    let mut ready_at = vec![0.0f64; aircraft];
    let rate = sortie_rate(p);
    let fix = Exp::new(1.0 / p.fix_h.max(1e-6)).ok();
    let mut sorties = 0u32;
    for day in 0..days {
        let day0 = day as f64 * 24.0;
        let window_end = day0 + p.ops_window_h;
        let crew_budget = (rate.aircrew_limit * aircraft as f64).floor() as u32;
        let mmh_budget = p.maintainers_per_aircraft * p.maintenance_shift_cover_h * p.maintenance_productive * aircraft as f64;
        let mmh_per_sortie = p.mmh_per_fh * p.sortie_h * (1.0 - p.maintenance_deferred);
        let (mut crew_used, mut mmh_used) = (0u32, 0.0);
        // Not mission capable today (scheduled maintenance, awaiting parts).
        let up: Vec<bool> = (0..aircraft).map(|_| rng.gen::<f64>() < p.mission_capable).collect();
        loop {
            // Earliest ready, serviceable aircraft (ties by index: deterministic).
            let next = (0..aircraft).filter(|&i| up[i]).map(|i| (ready_at[i].max(day0), i)).min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            let Some((t, i)) = next else { break };
            if t + p.sortie_h > window_end || crew_used >= crew_budget || mmh_used + mmh_per_sortie > mmh_budget {
                break;
            }
            sorties += 1;
            crew_used += 1;
            mmh_used += mmh_per_sortie;
            let mut ready = t + p.sortie_h + p.turnaround_h;
            if rng.gen::<f64>() < p.break_rate {
                ready += fix.map_or(p.fix_h, |d| d.sample(rng));
            }
            ready_at[i] = ready;
        }
        for r in ready_at.iter_mut() {
            *r = r.max(day0 + 24.0);
        }
    }
    FleetResult { sorties, per_aircraft_per_day: sorties as f64 / (aircraft * days).max(1) as f64 }
}
