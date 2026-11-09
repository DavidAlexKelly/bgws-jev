//! Units occupy space. A unit keeps one position and one decision maker,
//! but its surviving elements (vehicles, teams) are laid out in a formation
//! by what it is doing:
//!
//! - **column** on a road or on a march: nose to tail along the track the
//!   unit has driven (its trail), so the column bends with the road and
//!   has a real length;
//! - **wedge** moving across country: the lead in front, the rest echeloned
//!   back to either side;
//! - **line** halted: abreast across its facing (the nearest threat it
//!   knows of, else its heading).
//!
//! Spacing is per kind of element (`sim.formation.*`). An element that would
//! stand in water or ground its vehicles cannot enter is drawn in toward the
//! unit's centre. Sensing looks from and at several elements, and indirect
//! fire strikes each element where it stands, so dispersion and exposure
//! count.

use crate::components::*;
use crate::sim::Sim;
use bevy_ecs::prelude::{Component, Entity};
use bgws_catalogue::{Domain, MoverKind, Vulnerability};
use bgws_core::{figure, Enu, Figure, Figures};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Column,
    Wedge,
    Line,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormationParams {
    pub vehicle_m: f64,
    pub person_m: f64,
    pub column_vehicle_m: f64,
    pub column_person_m: f64,
    /// How far apart the trail's points are kept, m.
    pub trail_step_m: f64,
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.formation.vehicle_m", 50.0, "m", 10.0, 300.0, "default", Default, "spacing between vehicles abreast or in a wedge (as sim.fires.spacing_vehicle_m)"),
        figure!("sim.formation.person_m", 8.0, "m", 2.0, 50.0, "default", Default, "spacing between teams or soldiers abreast (as sim.fires.spacing_person_m)"),
        figure!("sim.formation.column_vehicle_m", 50.0, "m", 10.0, 200.0, "default", Default, "gap between vehicles in a column on the move"),
        figure!("sim.formation.column_person_m", 5.0, "m", 1.0, 30.0, "default", Default, "gap between soldiers in file on the move"),
        figure!("sim.formation.trail_step_m", 10.0, "m", 2.0, 50.0, "default", Default, "spacing of the points a unit keeps of its track, for its column to follow"),
    ]
}

impl FormationParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            vehicle_m: f.get("sim.formation.vehicle_m"),
            person_m: f.get("sim.formation.person_m"),
            column_vehicle_m: f.get("sim.formation.column_vehicle_m"),
            column_person_m: f.get("sim.formation.column_person_m"),
            trail_step_m: f.get("sim.formation.trail_step_m"),
        }
    }
}

/// The track a unit has driven, newest first, for its column to follow.
#[derive(Component, Clone, Debug, Default)]
pub struct Trail {
    pub points: VecDeque<Enu>,
}

impl Trail {
    /// Note where the unit is now; keep about `len_m` of track.
    pub fn note(&mut self, at: Enu, step: f64, len_m: f64) {
        if self.points.front().is_none_or(|p| p.sub(at).horizontal() >= step) {
            self.points.push_front(at);
            let keep = ((len_m / step).ceil() as usize + 2).max(4);
            self.points.truncate(keep);
        }
    }
}

/// What shape a unit takes now.
pub fn shape(s: &Sim, e: Entity) -> Shape {
    let Some(a) = s.world.get::<Act>(e) else { return Shape::Line };
    let Some(k) = s.world.get::<Kin>(e) else { return Shape::Line };
    match &a.activity {
        Activity::Move { mode, .. } => {
            if *mode == MoveModeS::March || s.ctx.ground.road_at(k.pos.e, k.pos.n).is_some() {
                Shape::Column
            } else {
                Shape::Wedge
            }
        }
        Activity::Withdraw { .. } => Shape::Column,
        _ => Shape::Line,
    }
}

/// Where a unit's surviving elements stand: one position each, the first at
/// the unit's own. Air, sea and space units, and units of one element, are
/// their position.
pub fn elements(s: &Sim, e: Entity) -> Vec<Enu> {
    let (Some(i), Some(k), Some(h)) = (s.world.get::<Ident>(e), s.world.get::<Kin>(e), s.world.get::<Health>(e)) else {
        return Vec::new();
    };
    let n = h.fit.max(1) as usize;
    if n == 1 || !matches!(i.domain, Domain::Land) || k.agl > 5.0 {
        return vec![k.pos];
    }
    let p = &s.ctx.p.formation;
    let person = i.platform.body.vulnerability == Vulnerability::Personnel;
    let shape = shape(s, e);
    let facing = if shape == Shape::Line { crate::systems::threat_bearing(s, e).unwrap_or(k.heading) } else { k.heading };
    let (fwd, right) = ((facing.sin(), facing.cos()), (facing.cos(), -facing.sin()));
    let at = |along: f64, across: f64| Enu::new(k.pos.e + fwd.0 * along + right.0 * across, k.pos.n + fwd.1 * along + right.1 * across, k.pos.u);
    let mut out: Vec<Enu> = match shape {
        Shape::Line => {
            let gap = if person { p.person_m } else { p.vehicle_m };
            (0..n).map(|j| at(0.0, (j as f64 - (n - 1) as f64 / 2.0) * gap)).collect()
        }
        Shape::Wedge => {
            let gap = if person { p.person_m } else { p.vehicle_m };
            (0..n)
                .map(|j| {
                    let rank = j.div_ceil(2) as f64;
                    let side = if j % 2 == 1 { -1.0 } else { 1.0 };
                    at(-rank * gap * 0.7, side * rank * gap * 0.7)
                })
                .collect()
        }
        Shape::Column => {
            let gap = if person { p.column_person_m } else { p.column_vehicle_m };
            let trail = s.world.get::<Trail>(e);
            (0..n).map(|j| behind(k.pos, k.heading, trail, j as f64 * gap)).collect()
        }
    };
    // Vehicles stand on ground they can be on: an element in water or a
    // block of buildings is drawn in toward the unit's position.
    if i.platform.mover.kind != MoverKind::Foot {
        let (class, _) = crate::systems::mover_class(i.platform.mover.kind);
        let ok = |q: &Enu| s.ctx.ground.land_cover(q.e, q.n) != bgws_env::LandCover::Water && s.ctx.ground.going(q.e, q.n, class).speed_kmh > 0.0;
        for q in out.iter_mut().skip(1) {
            for f in [1.0, 0.6, 0.3, 0.0] {
                let c = Enu::new(k.pos.e + (q.e - k.pos.e) * f, k.pos.n + (q.n - k.pos.n) * f, q.u);
                if f == 0.0 || ok(&c) {
                    *q = c;
                    break;
                }
            }
        }
    }
    out
}

/// The point `d` metres back along a unit's track (its trail, newest first),
/// or straight back from its heading where the trail runs out.
fn behind(head: Enu, heading: f64, trail: Option<&Trail>, d: f64) -> Enu {
    if d <= 0.0 {
        return head;
    }
    let mut left = d;
    let mut a = head;
    if let Some(t) = trail {
        for &b in &t.points {
            let seg = a.sub(b).horizontal();
            if seg <= 1e-6 {
                continue;
            }
            if seg >= left {
                let f = left / seg;
                return Enu::new(a.e + (b.e - a.e) * f, a.n + (b.n - a.n) * f, a.u);
            }
            left -= seg;
            a = b;
        }
    }
    // Beyond the trail: straight back from the last point, along the heading.
    Enu::new(a.e - heading.sin() * left, a.n - heading.cos() * left, a.u)
}

/// The elements of `els` nearest a point, up to `k`, nearest first.
pub fn nearest(els: &[Enu], to: Enu, k: usize) -> Vec<Enu> {
    let mut v: Vec<Enu> = els.to_vec();
    v.sort_by(|a, b| a.sub(to).horizontal().total_cmp(&b.sub(to).horizontal()));
    v.truncate(k);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_column_follows_the_track_it_drove() {
        // Drove north 300 m, then turned east 200 m: the head is at (200, 300).
        let mut t = Trail::default();
        for k in 0..=30 {
            t.note(Enu::new(0.0, k as f64 * 10.0, 0.0), 10.0, 1_000.0);
        }
        for k in 1..=20 {
            t.note(Enu::new(k as f64 * 10.0, 300.0, 0.0), 10.0, 1_000.0);
        }
        let head = Enu::new(200.0, 300.0, 0.0);
        let east = std::f64::consts::FRAC_PI_2;
        let b = behind(head, east, Some(&t), 100.0);
        assert!((b.e - 100.0).abs() < 1.0 && (b.n - 300.0).abs() < 1.0, "{b:?}");
        // Round the corner: 300 m back is 100 m down the northbound leg.
        let c = behind(head, east, Some(&t), 300.0);
        assert!(c.e.abs() < 1.0 && (c.n - 200.0).abs() < 1.0, "{c:?}");
        // Past the trail's end, straight back along the heading.
        let far = behind(head, east, None, 50.0);
        assert!((far.e - 150.0).abs() < 1e-9);
    }
}
