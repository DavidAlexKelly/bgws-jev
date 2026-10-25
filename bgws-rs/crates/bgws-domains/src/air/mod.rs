//! Air movement (RUST_ENGINE.md §8.1 Air): the point-mass energy model for
//! aircraft ([`aircraft`]), flight-path steering ([`paths`]), multirotor and
//! fixed-wing drones with the fibre tether ([`drone`], **[P8]**), and sortie
//! generation ([`sortie`], **[P30]**). The standard atmosphere lives in
//! [`crate::atmosphere`].

pub mod aircraft;
pub mod drone;
pub mod paths;
pub mod samples;
pub mod sortie;

pub use aircraft::{AirCommand, AirParams, AirState, AirStepReport, AircraftPerf, EnvelopePoint};
pub use drone::{glide, Battery, DroneParams, FibreTether, Glide, MultirotorEndurance, MultirotorPerf, MultirotorPower, TetherStatus};
pub use paths::{intercept, orbit_command, terrain_following_altitude, InterceptKind, InterceptSolution, PathParams, Racetrack, Route, Waypoint};
pub use sortie::{simulate_fleet, sortie_rate, FleetResult, SortieGeneration, SortieLimit, SortieParams, Tempo};

use bgws_core::Figure;

/// Every figure the air models declare.
pub fn figures() -> Vec<Figure> {
    let mut v = aircraft::figures();
    v.extend(paths::figures());
    v.extend(drone::figures());
    v.extend(sortie::figures());
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::atmosphere::isa;
    use crate::vec3::V3;
    use approx::assert_relative_eq;
    use bgws_core::{Figures, Seed, StreamKey};

    fn figs() -> Figures {
        Figures::from_list(figures()).unwrap()
    }

    fn level(perf: &AircraftPerf, alt: f64, v: f64) -> AirState {
        AirState { pos: V3::new(0.0, 0.0, alt), speed: v, heading: 0.0, gamma: 0.0, fuel: perf.fuel_capacity * 0.8, stores: 0.0 }
    }

    #[test]
    fn envelope_is_physical() {
        let f = figs();
        let p = AirParams::from_figures(&f);
        let a = samples::fighter();
        let m = a.empty_mass + 0.5 * a.fuel_capacity;
        // Stall speed around 60–90 m/s at sea level; corner speed 150–220 m/s.
        let vs = a.stall_speed(0.0, m);
        assert!((55.0..95.0).contains(&vs), "{vs}");
        let vc = a.corner_speed(0.0, m);
        assert!((140.0..230.0).contains(&vc), "{vc}");
        // At corner speed the instantaneous turn is the best; sustained is less.
        let e = a.envelope(&p, 3000.0, 230.0, m);
        assert!(e.turn_rate_instantaneous > e.turn_rate_sustained);
        assert!(e.turn_rate_sustained.to_degrees() > 8.0 && e.turn_rate_instantaneous.to_degrees() < 30.0, "{e:?}");
        // Climb and max speed fall with height; max speed is supersonic at 11 km.
        assert!(a.envelope(&p, 0.0, 250.0, m).climb_rate > a.envelope(&p, 12_000.0, 250.0, m).climb_rate);
        let vmax = a.max_level_speed(&p, 11_000.0, m);
        assert!(isa(11_000.0).mach(vmax) > 1.2, "{vmax}");
        // Fuel flow rises with throttle and falls with height at the same throttle.
        assert!(a.fuel_flow(&p, 0.0, 0.8, 1.0) > a.fuel_flow(&p, 10_000.0, 0.8, 1.0));
    }

    #[test]
    fn autopilot_captures_heading_altitude_and_speed() {
        let f = figs();
        let p = AirParams::from_figures(&f);
        let a = samples::fighter();
        let mut s = level(&a, 3000.0, 200.0);
        let cmd = AirCommand { heading: 1.5, altitude: 6000.0, speed: 250.0 };
        let fuel0 = s.fuel;
        for _ in 0..240 {
            a.step(&p, &mut s, &cmd, V3::ZERO, 1.0);
        }
        assert!(paths::heading_error(s.heading, 1.5).abs() < 0.02, "{}", s.heading);
        assert!((s.pos.z - 6000.0).abs() < 50.0, "{}", s.pos.z);
        assert!((s.speed - 250.0).abs() < 5.0, "{}", s.speed);
        assert!(s.fuel < fuel0);
    }

    #[test]
    fn turn_rate_respects_the_envelope() {
        let f = figs();
        let p = AirParams::from_figures(&f);
        let a = samples::fighter();
        let mut s = level(&a, 5000.0, 220.0);
        let m = a.mass(&s);
        let env = a.envelope(&p, 5000.0, 220.0, m);
        let cmd = AirCommand { heading: std::f64::consts::PI, altitude: 5000.0, speed: 220.0 };
        let h0 = s.heading;
        let r = a.step(&p, &mut s, &cmd, V3::ZERO, 1.0);
        let rate = (s.heading - h0).abs();
        assert!(rate <= env.turn_rate_instantaneous * 1.05, "{rate} vs {}", env.turn_rate_instantaneous);
        assert!(r.g_limited);
        assert!(r.load_factor <= env.n_available + 1e-6);
    }

    #[test]
    fn route_orbit_racetrack_and_intercept() {
        let f = figs();
        let p = AirParams::from_figures(&f);
        let pp = PathParams::from_figures(&f);
        let a = samples::fighter();
        let mut s = level(&a, 4000.0, 220.0);
        let mut route = Route::new(
            vec![
                Waypoint { pos: V3::new(20_000.0, 20_000.0, 5000.0), speed: 220.0 },
                Waypoint { pos: V3::new(40_000.0, 0.0, 3000.0), speed: 200.0 },
            ],
            false,
        );
        let mut steps = 0;
        while let Some(c) = route.command(&pp, &s) {
            a.step(&p, &mut s, &c, V3::ZERO, 1.0);
            steps += 1;
            assert!(steps < 2000);
        }
        assert!((s.pos - V3::new(40_000.0, 0.0, s.pos.z)).horizontal() < 3.0 * pp.arrival_radius);

        // Orbit: after settling, the distance to the centre stays near the radius.
        let centre = V3::new(60_000.0, 0.0, 0.0);
        for i in 0..900 {
            let c = orbit_command(&pp, &s, centre, 10_000.0, 6000.0, 200.0, true);
            a.step(&p, &mut s, &c, V3::new(10.0, 0.0, 0.0), 1.0);
            if i > 600 {
                let d = (s.pos - centre).horizontal();
                assert!((d - 10_000.0).abs() < 1500.0, "{d}");
            }
        }

        // Racetrack: stays within the stadium band.
        let rt = Racetrack { a: V3::new(0.0, 0.0, 0.0), b: V3::new(40_000.0, 0.0, 0.0), radius: 8_000.0, altitude: 7000.0, speed: 200.0, clockwise: true };
        let mut s2 = level(&a, 7000.0, 200.0);
        s2.pos = V3::new(20_000.0, 8_000.0, 7000.0);
        s2.heading = std::f64::consts::FRAC_PI_2;
        for i in 0..1200 {
            let c = rt.command(&pp, &s2);
            a.step(&p, &mut s2, &c, V3::ZERO, 1.0);
            if i > 100 {
                let dx = s2.pos.x.clamp(0.0, 40_000.0);
                let d = (s2.pos - V3::new(dx, 0.0, s2.pos.z)).horizontal();
                assert!((d - 8_000.0).abs() < 2_000.0, "step {i}: {d}");
            }
        }

        // Intercept: collision course against a crossing target.
        let sol = intercept(V3::ZERO, 300.0, V3::new(0.0, 30_000.0, 0.0), V3::new(200.0, 0.0, 0.0));
        assert_eq!(sol.kind, InterceptKind::CollisionCourse);
        assert_relative_eq!((sol.point - V3::ZERO).norm(), 300.0 * sol.time_to_go, epsilon = 1e-6);
        let chase = intercept(V3::ZERO, 100.0, V3::new(0.0, 30_000.0, 0.0), V3::new(0.0, 200.0, 0.0));
        assert_eq!(chase.kind, InterceptKind::PurePursuit);
    }

    #[test]
    fn terrain_following_keeps_clearance_over_a_ridge() {
        let f = figs();
        let p = AirParams::from_figures(&f);
        let pp = PathParams::from_figures(&f);
        let a = samples::fighter();
        let ridge = |_x: f64, y: f64| 600.0 * (-((y - 15_000.0) / 2_000.0).powi(2)).exp();
        let mut s = level(&a, 100.0, 220.0);
        let clearance = 100.0;
        let mut min_clear = f64::INFINITY;
        for _ in 0..(30_000.0f64 / 220.0 / 0.5) as usize {
            let m = a.mass(&s);
            let climb = a.envelope(&p, s.pos.z, s.speed, m).climb_rate;
            let alt = terrain_following_altitude(&pp, &s, clearance, climb, &ridge);
            a.step(&p, &mut s, &AirCommand { heading: 0.0, altitude: alt, speed: 220.0 }, V3::ZERO, 0.5);
            min_clear = min_clear.min(s.pos.z - ridge(s.pos.x, s.pos.y));
        }
        assert!(s.pos.y > 25_000.0);
        assert!(min_clear > 0.4 * clearance, "{min_clear}");
    }

    #[test]
    fn multirotor_endurance_by_wind_payload_and_cold() {
        let f = figs();
        let dp = DroneParams::from_figures(&f);
        let q = samples::quadcopter();
        let air = isa(100.0);
        let hover = q.endurance(&dp, &air, 0.0, V3::ZERO, V3::ZERO);
        // A Mavic-class quad hovers for roughly 20–40 minutes.
        assert!((20.0 * 60.0..45.0 * 60.0).contains(&hover.endurance_s), "{}", hover.endurance_s / 60.0);
        let loaded = q.endurance(&dp, &air, 0.3, V3::ZERO, V3::ZERO);
        assert!(loaded.endurance_s < hover.endurance_s);
        // Holding station in a strong wind costs more than a light one; a modest
        // breeze can even help (translational lift), as momentum theory says.
        let windy = q.endurance(&dp, &air, 0.0, V3::ZERO, V3::new(20.0, 0.0, 0.0));
        assert!(windy.endurance_s < hover.endurance_s);
        let too_windy = q.endurance(&dp, &air, 0.0, V3::ZERO, V3::new(30.0, 0.0, 0.0));
        assert!(!too_windy.feasible);
        let cold = crate::atmosphere::isa_dev(100.0, -30.0);
        assert!(q.endurance(&dp, &cold, 0.0, V3::ZERO, V3::ZERO).endurance_s < hover.endurance_s);
        let mut b = Battery::full(q.battery_wh);
        assert!(b.drain(hover.power.total, 60.0));
        assert!(b.fraction() < 1.0);
    }

    #[test]
    fn glide_and_fibre_tether() {
        let f = figs();
        let dp = DroneParams::from_figures(&f);
        let a = samples::fighter();
        let (v, ld, sink) = a.best_glide(1000.0, a.empty_mass + 1000.0);
        assert!(ld > 8.0 && ld < 16.0, "{ld}");
        let g = glide(v, ld, 1000.0, 0.0);
        assert_relative_eq!(g.ground_range, 1000.0 * ld, epsilon = 1e-6);
        assert_relative_eq!(g.sink_rate, sink, epsilon = 1e-9);
        assert!(glide(v, ld, 1000.0, -20.0).ground_range < g.ground_range);

        let mut t = FibreTether::new(&dp);
        assert!(t.max_range(&dp) < dp.fibre_spool);
        let st = t.fly(&dp, 5_000.0, true);
        assert!(!st.at_limit && st.snag_probability > 0.0);
        let st = t.fly(&dp, 10_000.0, false);
        assert!(st.at_limit && st.remaining <= 1e-9);
    }

    #[test]
    fn sortie_generation_matches_p30() {
        let f = figs();
        let surge = sortie_rate(&SortieParams::from_figures(&f, Tempo::Surge));
        let sus = sortie_rate(&SortieParams::from_figures(&f, Tempo::Sustained));
        assert!((2.0..=3.0).contains(&surge.per_aircraft_per_day), "{surge:?}");
        assert!((1.0..=1.5).contains(&sus.per_aircraft_per_day), "{sus:?}");
        // The stochastic fleet agrees with the analytic rate.
        let mut rng = Seed(7).stream(StreamKey::new("test.sortie", 0, 0, 0));
        let sim = simulate_fleet(&SortieParams::from_figures(&f, Tempo::Surge), 24, 10, &mut rng);
        assert!((1.8..=3.0).contains(&sim.per_aircraft_per_day), "{sim:?}");
        let sim = simulate_fleet(&SortieParams::from_figures(&f, Tempo::Sustained), 24, 10, &mut rng);
        assert!((0.9..=1.5).contains(&sim.per_aircraft_per_day), "{sim:?}");
    }
}
