//! A small 3-vector for the point-mass models (§8.1). Tactical flyouts work
//! in a local flat frame (x east, y north, z up, metres) that is valid within
//! a few tens of kilometres (§5.2); ballistic and orbital work uses the same
//! type in ECI or ECEF metres.

use serde::{Deserialize, Serialize};
use std::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

/// A 3-vector of f64.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl V3 {
    pub const ZERO: V3 = V3 { x: 0.0, y: 0.0, z: 0.0 };
    pub const Z: V3 = V3 { x: 0.0, y: 0.0, z: 1.0 };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub fn from_array(a: [f64; 3]) -> Self {
        Self::new(a[0], a[1], a[2])
    }
    pub fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        V3::new(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)
    }
    pub fn norm(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn norm2(self) -> f64 {
        self.dot(self)
    }
    pub fn horizontal(self) -> f64 {
        self.x.hypot(self.y)
    }
    /// The unit vector, or zero for a zero vector.
    pub fn unit(self) -> V3 {
        let n = self.norm();
        if n > 0.0 {
            self * (1.0 / n)
        } else {
            V3::ZERO
        }
    }
    /// The part of `self` perpendicular to unit vector `u`.
    pub fn reject(self, u: V3) -> V3 {
        self - u * self.dot(u)
    }
    /// Clamp the magnitude to `max`.
    pub fn clamp_norm(self, max: f64) -> V3 {
        let n = self.norm();
        if n > max && n > 0.0 {
            self * (max / n)
        } else {
            self
        }
    }
    /// Angle between two vectors, radians.
    pub fn angle_to(self, o: V3) -> f64 {
        let d = self.norm() * o.norm();
        if d <= 0.0 {
            return 0.0;
        }
        (self.dot(o) / d).clamp(-1.0, 1.0).acos()
    }
    /// Heading in the local frame: radians clockwise from north (+y).
    pub fn heading(self) -> f64 {
        self.x.atan2(self.y).rem_euclid(std::f64::consts::TAU)
    }
    /// Horizontal unit vector for a heading (clockwise from north).
    pub fn from_heading(h: f64) -> V3 {
        V3::new(h.sin(), h.cos(), 0.0)
    }
    /// Rotate about the z axis by `a` radians (counter-clockwise seen from above).
    pub fn rotate_z(self, a: f64) -> V3 {
        let (s, c) = a.sin_cos();
        V3::new(c * self.x - s * self.y, s * self.x + c * self.y, self.z)
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        V3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        V3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, k: f64) -> V3 {
        V3::new(self.x * k, self.y * k, self.z * k)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        V3::new(-self.x, -self.y, -self.z)
    }
}
impl AddAssign for V3 {
    fn add_assign(&mut self, o: V3) {
        *self = *self + o;
    }
}
impl SubAssign for V3 {
    fn sub_assign(&mut self, o: V3) {
        *self = *self - o;
    }
}

/// Wrap an angle to (−π, π].
pub fn wrap_pi(a: f64) -> f64 {
    let t = std::f64::consts::TAU;
    let mut x = a.rem_euclid(t);
    if x > std::f64::consts::PI {
        x -= t;
    }
    x
}

/// Closest point of approach of two points moving linearly over `[0, dt]`:
/// relative position `r0` at the start and `r1` at the end. Returns
/// `(time_of_cpa, miss_distance)`. Used inside every flyout sub-step so a fast
/// missile never passes through its target between samples (§8.1).
pub fn cpa_linear(r0: V3, r1: V3, dt: f64) -> (f64, f64) {
    if dt <= 0.0 {
        return (0.0, r0.norm());
    }
    let dv = (r1 - r0) * (1.0 / dt);
    let vv = dv.norm2();
    let t = if vv > 0.0 { (-r0.dot(dv) / vv).clamp(0.0, dt) } else { 0.0 };
    (t, (r0 + dv * t).norm())
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    #[test]
    fn cpa_catches_a_pass_through() {
        // Missile and target swap sides within one step: endpoints are 100 m
        // apart but the path passes within 1 m.
        let (t, miss) = cpa_linear(V3::new(50.0, 1.0, 0.0), V3::new(-50.0, 1.0, 0.0), 0.1);
        assert_relative_eq!(t, 0.05, epsilon = 1e-12);
        assert_relative_eq!(miss, 1.0, epsilon = 1e-12);
    }

    #[test]
    fn heading_convention() {
        assert_relative_eq!(V3::new(1.0, 0.0, 0.0).heading(), std::f64::consts::FRAC_PI_2);
        assert_relative_eq!(V3::from_heading(0.3).heading(), 0.3, epsilon = 1e-12);
        assert_relative_eq!(wrap_pi(3.0 * std::f64::consts::PI), std::f64::consts::PI, epsilon = 1e-12);
    }
}
