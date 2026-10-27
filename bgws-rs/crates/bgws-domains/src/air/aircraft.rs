//! The point-mass, three-degree-of-freedom energy model for aircraft
//! (RUST_ENGINE.md §8.1 Air): thrust lapse with density, a parabolic drag
//! polar with transonic wave-drag rise, the lift limit and structural g that
//! together give the manoeuvre envelope, sustained and instantaneous turn
//! rates, climb from specific excess power, and fuel flow from a thrust-
//! specific fuel consumption that varies with altitude and Mach.
//!
//! State: position (local ENU metres), true airspeed V, heading ψ, flight-path
//! angle γ and fuel. Controls: throttle, load factor n and bank μ. The
//! equations are the standard point-mass set
//!
//! ```text
//! dV/dt = (T − D)/m − g sinγ
//! dγ/dt = g/V (n cosμ − cosγ)
//! dψ/dt = g n sinμ / (V cosγ)
//! ```
//!
//! An autopilot turns a commanded heading, altitude and speed into n, μ and
//! throttle within the envelope, so every limit (stall, g, thrust, ceiling)
//! emerges from the record rather than from a rule.

use crate::atmosphere::{isa, Air, G0};
use crate::vec3::{wrap_pi, V3};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// An aircraft's performance record (a catalogue `Mover` record, §5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AircraftPerf {
    pub name: String,
    /// Operating empty mass, kg.
    pub empty_mass: f64,
    /// Internal fuel capacity, kg.
    pub fuel_capacity: f64,
    /// Reference wing area, m².
    pub wing_area: f64,
    /// Zero-lift drag coefficient (subsonic).
    pub cd0: f64,
    /// Induced drag factor k in CD = CD0 + k·CL².
    pub k_induced: f64,
    /// Maximum lift coefficient (clean).
    pub cl_max: f64,
    /// Drag-divergence Mach number.
    pub mach_crit: f64,
    /// Wave drag: CD0 rises by this many CD0s per 0.1 Mach above `mach_crit`, up to Mach 1.2.
    pub wave_drag_rise: f64,
    /// Maximum sea-level static thrust (with reheat if fitted), N.
    pub max_thrust_sl: f64,
    /// Thrust-specific fuel consumption at sea level, kg/(N·s).
    pub tsfc_sl: f64,
    /// Structural load-factor limit, g.
    pub n_max: f64,
    /// Maximum Mach number (structural or inlet).
    pub mach_max: f64,
    /// Service ceiling, m.
    pub ceiling: f64,
}

/// Model figures for the aircraft dynamics and autopilot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirParams {
    /// Thrust lapse exponent: T = T_sl·σ^n.
    pub thrust_lapse: f64,
    /// TSFC rise per unit Mach.
    pub tsfc_mach: f64,
    /// Idle thrust as a fraction of maximum.
    pub idle_fraction: f64,
    /// Heading-error gain, 1/s.
    pub heading_gain: f64,
    /// Altitude capture time constant, s.
    pub alt_tau: f64,
    /// Flight-path response time constant, s.
    pub gamma_tau: f64,
    /// Speed capture time constant, s.
    pub speed_tau: f64,
    /// Maximum commanded descent rate, m/s.
    pub max_descent: f64,
    /// Integration sub-step, s.
    pub substep: f64,
}

impl AirParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            thrust_lapse: f.get("air.thrust_lapse_exponent"),
            tsfc_mach: f.get("air.tsfc_mach_factor"),
            idle_fraction: f.get("air.idle_thrust_fraction"),
            heading_gain: f.get("air.autopilot.heading_gain"),
            alt_tau: f.get("air.autopilot.alt_tau"),
            gamma_tau: f.get("air.autopilot.gamma_tau"),
            speed_tau: f.get("air.autopilot.speed_tau"),
            max_descent: f.get("air.autopilot.max_descent"),
            substep: f.get("air.integration_substep"),
        }
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("air.thrust_lapse_exponent", 0.75, "1", 0.5, 1.0, "default", Default, "T = T_sl·σ^n for turbofans"),
        figure!("air.tsfc_mach_factor", 0.35, "1", 0.0, 1.0, "default", Default, "TSFC × (1 + kM)"),
        figure!("air.idle_thrust_fraction", 0.05, "fraction", 0.0, 0.15, "default", Default, "flight idle thrust"),
        figure!("air.autopilot.heading_gain", 0.6, "1/s", 0.1, 2.0, "default", Default, "turn-rate demand per rad of heading error"),
        figure!("air.autopilot.alt_tau", 8.0, "s", 2.0, 30.0, "default", Default, "altitude capture time constant"),
        figure!("air.autopilot.gamma_tau", 1.5, "s", 0.3, 5.0, "default", Default, "flight-path response"),
        figure!("air.autopilot.speed_tau", 10.0, "s", 2.0, 40.0, "default", Default, "speed capture time constant"),
        figure!("air.autopilot.max_descent", 60.0, "m/s", 10.0, 150.0, "default", Default, "commanded descent limit"),
        figure!("air.integration_substep", 0.05, "s", 0.005, 0.2, "default", Default, "point-mass integration step"),
    ]
}

/// The dynamic state of one aircraft.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirState {
    /// Local ENU position, m (z is altitude above MSL).
    pub pos: V3,
    /// True airspeed, m/s.
    pub speed: f64,
    /// Heading, radians clockwise from north.
    pub heading: f64,
    /// Flight-path angle, radians.
    pub gamma: f64,
    /// Fuel remaining, kg.
    pub fuel: f64,
    /// External stores and payload, kg.
    pub stores: f64,
}

impl AirState {
    /// Velocity through the air mass.
    pub fn air_velocity(&self) -> V3 {
        let (sg, cg) = self.gamma.sin_cos();
        V3::new(cg * self.heading.sin(), cg * self.heading.cos(), sg) * self.speed
    }
}

/// What the autopilot is asked to hold.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AirCommand {
    pub heading: f64,
    pub altitude: f64,
    pub speed: f64,
}

/// The envelope point at one flight condition.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct EnvelopePoint {
    pub mach: f64,
    pub dynamic_pressure: f64,
    /// Load factor available from CLmax and structure, g.
    pub n_available: f64,
    /// Load factor sustainable at full thrust, g.
    pub n_sustained: f64,
    /// Instantaneous and sustained turn rates, rad/s.
    pub turn_rate_instantaneous: f64,
    pub turn_rate_sustained: f64,
    /// Maximum rate of climb (Ps at n = 1, full thrust), m/s.
    pub climb_rate: f64,
}

/// What one step did and why, for the AAR.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct AirStepReport {
    pub thrust: f64,
    pub drag: f64,
    pub load_factor: f64,
    pub bank: f64,
    pub throttle: f64,
    pub fuel_flow: f64,
    pub mach: f64,
    /// The g demand exceeded what lift and structure allow.
    pub g_limited: bool,
    /// The speed demand exceeded thrust.
    pub thrust_limited: bool,
}

impl AircraftPerf {
    pub fn mass(&self, s: &AirState) -> f64 {
        self.empty_mass + s.fuel.max(0.0) + s.stores
    }

    /// Zero-lift drag with transonic rise.
    pub fn cd0_at(&self, mach: f64) -> f64 {
        let over = (mach.min(1.2) - self.mach_crit).max(0.0);
        self.cd0 * (1.0 + self.wave_drag_rise * over / 0.1)
    }

    /// Drag at load factor `n`, N.
    pub fn drag(&self, air: &Air, v: f64, mass: f64, n: f64) -> f64 {
        let q = air.dynamic_pressure(v).max(1e-6);
        let cl = n * mass * G0 / (q * self.wing_area);
        q * self.wing_area * (self.cd0_at(air.mach(v)) + self.k_induced * cl * cl)
    }

    /// Maximum thrust available, N.
    pub fn thrust_max(&self, p: &AirParams, air: &Air) -> f64 {
        self.max_thrust_sl * air.sigma().powf(p.thrust_lapse)
    }

    /// Thrust-specific fuel consumption at this condition, kg/(N·s).
    pub fn tsfc(&self, p: &AirParams, air: &Air, mach: f64) -> f64 {
        self.tsfc_sl * (air.temperature / crate::atmosphere::T0).sqrt() * (1.0 + p.tsfc_mach * mach)
    }

    /// Fuel flow at a throttle setting, kg/s.
    pub fn fuel_flow(&self, p: &AirParams, h: f64, mach: f64, throttle: f64) -> f64 {
        let air = isa(h);
        self.tsfc(p, &air, mach) * self.thrust_max(p, &air) * throttle.clamp(0.0, 1.0)
    }

    /// Load factor available from CLmax and structure, g.
    pub fn n_available(&self, air: &Air, v: f64, mass: f64) -> f64 {
        let n_lift = air.dynamic_pressure(v) * self.wing_area * self.cl_max / (mass * G0);
        n_lift.min(self.n_max)
    }

    /// 1-g stall speed, m/s.
    pub fn stall_speed(&self, h: f64, mass: f64) -> f64 {
        (2.0 * mass * G0 / (isa(h).density * self.wing_area * self.cl_max)).sqrt()
    }

    /// Corner speed (lift limit meets structural limit), m/s.
    pub fn corner_speed(&self, h: f64, mass: f64) -> f64 {
        self.stall_speed(h, mass) * self.n_max.sqrt()
    }

    /// The manoeuvre envelope at a flight condition.
    pub fn envelope(&self, p: &AirParams, h: f64, v: f64, mass: f64) -> EnvelopePoint {
        let air = isa(h);
        let q = air.dynamic_pressure(v);
        let mach = air.mach(v);
        let n_av = self.n_available(&air, v, mass);
        let t = self.thrust_max(p, &air);
        let cd0 = self.cd0_at(mach);
        let qs = q * self.wing_area;
        let cl_t = ((t / qs - cd0) / self.k_induced).max(0.0).sqrt();
        let n_sus = (cl_t * qs / (mass * G0)).min(n_av);
        let rate = |n: f64| if n > 1.0 { G0 * (n * n - 1.0).sqrt() / v } else { 0.0 };
        let climb = v * (t - self.drag(&air, v, mass, 1.0)) / (mass * G0);
        EnvelopePoint {
            mach,
            dynamic_pressure: q,
            n_available: n_av,
            n_sustained: n_sus,
            turn_rate_instantaneous: rate(n_av),
            turn_rate_sustained: rate(n_sus),
            climb_rate: climb,
        }
    }

    /// Maximum level speed at a height (thrust = drag), m/s, by bisection.
    pub fn max_level_speed(&self, p: &AirParams, h: f64, mass: f64) -> f64 {
        let air = isa(h);
        let t = self.thrust_max(p, &air);
        let vmax_struct = self.mach_max * air.speed_of_sound;
        let mut lo = self.stall_speed(h, mass);
        let mut hi = vmax_struct;
        if t >= self.drag(&air, hi, mass, 1.0) {
            return hi;
        }
        if t < self.drag(&air, lo * 1.3, mass, 1.0) {
            return 0.0;
        }
        lo *= 1.3;
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if t >= self.drag(&air, mid, mass, 1.0) {
                lo = mid
            } else {
                hi = mid
            }
        }
        lo
    }

    /// Best-glide (L/D max) condition: speed m/s, L/D, and sink rate m/s.
    pub fn best_glide(&self, h: f64, mass: f64) -> (f64, f64, f64) {
        let air = isa(h);
        let ld = 1.0 / (2.0 * (self.k_induced * self.cd0).sqrt());
        let cl = (self.cd0 / self.k_induced).sqrt();
        let v = (2.0 * mass * G0 / (air.density * self.wing_area * cl)).sqrt();
        (v, ld, v / ld)
    }

    /// Advance the aircraft by `dt` seconds towards a command, in still air
    /// plus `wind` (ENU m/s). Integrates in sub-steps of `p.substep`.
    pub fn step(&self, p: &AirParams, s: &mut AirState, cmd: &AirCommand, wind: V3, dt: f64) -> AirStepReport {
        let n_sub = (dt / p.substep).ceil().max(1.0) as usize;
        let h = dt / n_sub as f64;
        let mut rep = AirStepReport::default();
        for _ in 0..n_sub {
            rep = self.substep(p, s, cmd, wind, h);
        }
        rep
    }

    fn substep(&self, p: &AirParams, s: &mut AirState, cmd: &AirCommand, wind: V3, dt: f64) -> AirStepReport {
        let air = isa(s.pos.z);
        let m = self.mass(s);
        let v = s.speed.max(1.0);
        let mach = air.mach(v);
        let n_av = self.n_available(&air, v, m);
        let t_max = if s.fuel > 0.0 { self.thrust_max(p, &air) } else { 0.0 };

        // Vertical demand: altitude error → climb rate → flight-path angle.
        let climb_cap = (v * (t_max - self.drag(&air, v, m, 1.0)) / (m * G0)).max(0.0);
        let ceiling_ok = s.pos.z < self.ceiling;
        let alt_cmd = cmd.altitude.min(self.ceiling);
        let hdot = ((alt_cmd - s.pos.z) / p.alt_tau).clamp(-p.max_descent, if ceiling_ok { climb_cap.max(0.0) } else { 0.0 });
        let gamma_des = (hdot / v).clamp(-0.9, 0.9).asin();
        let gamma_dot = (gamma_des - s.gamma) / p.gamma_tau;

        // Horizontal demand: heading error → turn rate.
        let psi_dot = p.heading_gain * wrap_pi(cmd.heading - s.heading);
        let (sg, cg) = s.gamma.sin_cos();
        let mut nv = cg + v * gamma_dot / G0;
        let mut nh = v * cg * psi_dot / G0;
        let n_req = nv.hypot(nh);
        let g_limited = n_req > n_av;
        if g_limited {
            // Keep the vertical component first (don't fall out of the sky), then turn.
            nv = nv.clamp(-n_av, n_av);
            let rem = (n_av * n_av - nv * nv).max(0.0).sqrt();
            nh = nh.clamp(-rem, rem);
        }
        let n = nv.hypot(nh);
        let mu = nh.atan2(nv);
        let drag = self.drag(&air, v, m, n);

        // Throttle for the speed demand.
        let v_cmd = cmd.speed.min(self.mach_max * air.speed_of_sound);
        let vdot_des = (v_cmd - v) / p.speed_tau;
        let t_req = m * (vdot_des + G0 * sg) + drag;
        let throttle = if t_max > 0.0 { (t_req / t_max).clamp(p.idle_fraction, 1.0) } else { 0.0 };
        let thrust = throttle * t_max;
        let ff = self.tsfc(p, &air, mach) * thrust;

        let vdot = (thrust - drag) / m - G0 * sg;
        let gdot = G0 / v * (n * mu.cos() - cg);
        let pdot = G0 * n * mu.sin() / (v * cg.max(0.05));
        s.speed = (v + vdot * dt).max(1.0);
        s.gamma = (s.gamma + gdot * dt).clamp(-1.4, 1.4);
        s.heading = (s.heading + pdot * dt).rem_euclid(std::f64::consts::TAU);
        s.pos += (s.air_velocity() + wind) * dt;
        s.fuel = (s.fuel - ff * dt).max(0.0);
        AirStepReport {
            thrust,
            drag,
            load_factor: n,
            bank: mu,
            throttle,
            fuel_flow: ff,
            mach,
            g_limited,
            thrust_limited: t_req > t_max,
        }
    }
}
