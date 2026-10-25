//! Flight-path steering (§8.1 Air): waypoint following, orbits and
//! racetracks, intercept (collision-course) steering, and terrain following
//! over a terrain-height callback with minimum clearance. Each produces an
//! [`AirCommand`] for the autopilot in [`super::aircraft`]; the aircraft's own
//! envelope then limits how well it is followed.

use super::aircraft::{AirCommand, AirState};
use crate::vec3::{wrap_pi, V3};
use bgws_core::{figure, Figure, Figures};
use serde::{Deserialize, Serialize};

/// Model figures for steering.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathParams {
    /// Waypoint capture radius, m.
    pub arrival_radius: f64,
    /// Orbit cross-track correction gain (rad per orbit radius of error).
    pub orbit_gain: f64,
    /// Terrain-following look-ahead, s.
    pub tf_lookahead: f64,
    /// Terrain samples over the look-ahead.
    pub tf_samples: usize,
    /// Fraction of the maximum climb rate the TF planner may rely on.
    pub tf_climb_margin: f64,
}

impl PathParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            arrival_radius: f.get("air.path.arrival_radius"),
            orbit_gain: f.get("air.path.orbit_gain"),
            tf_lookahead: f.get("air.path.tf_lookahead"),
            tf_samples: f.get("air.path.tf_samples") as usize,
            tf_climb_margin: f.get("air.path.tf_climb_margin"),
        }
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("air.path.arrival_radius", 800.0, "m", 50.0, 5000.0, "default", Default, "waypoint capture radius"),
        figure!("air.path.orbit_gain", 2.0, "1", 0.2, 10.0, "default", Default, "orbit cross-track gain"),
        figure!("air.path.tf_lookahead", 20.0, "s", 3.0, 60.0, "default", Default, "terrain-following look-ahead"),
        figure!("air.path.tf_samples", 40.0, "count", 5.0, 200.0, "default", Default, "terrain samples in the look-ahead"),
        figure!("air.path.tf_climb_margin", 0.5, "fraction", 0.1, 1.0, "default", Default, "share of max climb TF relies on"),
    ]
}

/// One waypoint: position (z = altitude) and speed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub pos: V3,
    pub speed: f64,
}

/// A route being flown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub waypoints: Vec<Waypoint>,
    pub next: usize,
    /// Start again at the first waypoint after the last.
    pub cyclic: bool,
}

impl Route {
    pub fn new(waypoints: Vec<Waypoint>, cyclic: bool) -> Self {
        Self { waypoints, next: 0, cyclic }
    }

    /// The command towards the next waypoint, advancing when inside the
    /// capture radius or abeam (passed it). `None` when the route is done.
    pub fn command(&mut self, p: &PathParams, s: &AirState) -> Option<AirCommand> {
        loop {
            let wp = *self.waypoints.get(self.next)?;
            let d = wp.pos - s.pos;
            let ahead = d.x * s.heading.sin() + d.y * s.heading.cos();
            let captured = d.horizontal() < p.arrival_radius || (d.horizontal() < 3.0 * p.arrival_radius && ahead < 0.0);
            if captured {
                self.next += 1;
                if self.next >= self.waypoints.len() {
                    if self.cyclic && !self.waypoints.is_empty() {
                        self.next = 0;
                    } else {
                        return None;
                    }
                }
                continue;
            }
            return Some(AirCommand { heading: d.heading(), altitude: wp.pos.z, speed: wp.speed });
        }
    }
}

/// Command for a circular orbit about `centre` (z ignored).
pub fn orbit_command(p: &PathParams, s: &AirState, centre: V3, radius: f64, altitude: f64, speed: f64, clockwise: bool) -> AirCommand {
    let r = s.pos - centre;
    let d = r.horizontal().max(1.0);
    let radial = r.heading();
    let dir = if clockwise { 1.0 } else { -1.0 };
    let tangent = radial + dir * std::f64::consts::FRAC_PI_2;
    // Positive error = outside the circle → turn inwards.
    let err = ((d - radius) / radius * p.orbit_gain).clamp(-1.2, 1.2);
    AirCommand { heading: (tangent + dir * err).rem_euclid(std::f64::consts::TAU), altitude, speed }
}

/// A racetrack (stadium) orbit: two turn centres and a radius.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Racetrack {
    pub a: V3,
    pub b: V3,
    pub radius: f64,
    pub altitude: f64,
    pub speed: f64,
    pub clockwise: bool,
}

impl Racetrack {
    /// Command to fly the racetrack: on the straight legs the track is
    /// parallel to a→b offset by the radius; at the ends it orbits the centre.
    pub fn command(&self, p: &PathParams, s: &AirState) -> AirCommand {
        let ab = V3::new(self.b.x - self.a.x, self.b.y - self.a.y, 0.0);
        let len = ab.norm().max(1.0);
        let u = ab * (1.0 / len);
        let rel = V3::new(s.pos.x - self.a.x, s.pos.y - self.a.y, 0.0);
        let along = rel.dot(u);
        if along <= 0.0 {
            return orbit_command(p, s, self.a, self.radius, self.altitude, self.speed, self.clockwise);
        }
        if along >= len {
            return orbit_command(p, s, self.b, self.radius, self.altitude, self.speed, self.clockwise);
        }
        // Signed distance left of the a→b axis.
        let left_normal = V3::new(-u.y, u.x, 0.0);
        let side = rel.dot(left_normal);
        let cw = if self.clockwise { 1.0 } else { -1.0 };
        // Clockwise: the left (side > 0) leg flies a→b, the right leg b→a.
        let leg_dir = if side * cw >= 0.0 { 1.0 } else { -1.0 };
        let leg = u * leg_dir;
        let leg_heading = leg.heading();
        let line = if side >= 0.0 { self.radius } else { -self.radius };
        let right_of_leg = V3::new(leg.y, -leg.x, 0.0);
        // Cross-track error measured to the right of the leg; steer left to null it.
        let e_right = (side - line) * left_normal.dot(right_of_leg);
        let corr = (e_right / self.radius * p.orbit_gain).clamp(-1.0, 1.0);
        AirCommand { heading: (leg_heading - corr).rem_euclid(std::f64::consts::TAU), altitude: self.altitude, speed: self.speed }
    }
}

/// How an intercept was solved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterceptKind {
    /// A collision course exists (lead pursuit).
    CollisionCourse,
    /// The target is too fast: chase it directly.
    PurePursuit,
}

/// An intercept solution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InterceptSolution {
    pub kind: InterceptKind,
    /// Heading to fly, radians.
    pub heading: f64,
    /// Climb angle to fly, radians.
    pub climb_angle: f64,
    /// Predicted time to intercept (infinite for pure pursuit of a faster target).
    pub time_to_go: f64,
    /// Predicted intercept point.
    pub point: V3,
}

/// The collision-course intercept of a constant-velocity target by an
/// interceptor at constant speed: solve |r + v_t·t| = s·t for the smallest
/// positive t. Falls back to pure pursuit when no solution exists.
pub fn intercept(own: V3, own_speed: f64, target: V3, target_vel: V3) -> InterceptSolution {
    let r = target - own;
    let a = target_vel.norm2() - own_speed * own_speed;
    let b = 2.0 * r.dot(target_vel);
    let c = r.norm2();
    let t = if a.abs() < 1e-9 {
        if b < 0.0 {
            Some(-c / b)
        } else {
            None
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            None
        } else {
            let sq = disc.sqrt();
            let mut roots = [(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)];
            roots.sort_by(|x, y| x.total_cmp(y));
            roots.into_iter().find(|&t| t > 0.0)
        }
    };
    match t {
        Some(t) => {
            let p = target + target_vel * t;
            let d = p - own;
            InterceptSolution { kind: InterceptKind::CollisionCourse, heading: d.heading(), climb_angle: d.z.atan2(d.horizontal()), time_to_go: t, point: p }
        }
        None => InterceptSolution {
            kind: InterceptKind::PurePursuit,
            heading: r.heading(),
            climb_angle: r.z.atan2(r.horizontal()),
            time_to_go: f64::INFINITY,
            point: target,
        },
    }
}

/// The altitude to command for terrain following: the highest of
/// (terrain + clearance) at each look-ahead sample, less the height the
/// aircraft can gain by then at a margin of its climb rate. `terrain(x, y)`
/// returns the terrain height (m) at a local position.
pub fn terrain_following_altitude(
    p: &PathParams,
    s: &AirState,
    clearance: f64,
    climb_rate: f64,
    terrain: &dyn Fn(f64, f64) -> f64,
) -> f64 {
    let dir = V3::from_heading(s.heading);
    let v = s.speed.max(1.0);
    let n = p.tf_samples.max(1);
    let climb = climb_rate.max(0.0) * p.tf_climb_margin;
    let mut cmd = terrain(s.pos.x, s.pos.y) + clearance;
    for i in 1..=n {
        let t = p.tf_lookahead * i as f64 / n as f64;
        let q = s.pos + dir * (v * t);
        let need = terrain(q.x, q.y) + clearance - climb * t;
        cmd = cmd.max(need);
    }
    cmd
}

/// Heading error helper (for tests and behaviour templates).
pub fn heading_error(from: f64, to: f64) -> f64 {
    wrap_pi(to - from)
}
