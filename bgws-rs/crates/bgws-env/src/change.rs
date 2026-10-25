//! The change layer (§7.1): craters, rubble, burnt forest, blown bridges
//! and emplaced obstacles alter the cells they touch. Each change records
//! which sides know about it; the terrain is ground truth, and the sim's
//! side views decide who sees the change [R4.5].
//!
//! Changes apply in id order, so replaying a list rebuilds the same terrain.

use crate::landcover::LandCover;
use crate::network::{polyline_distance, RoadNetwork};
use crate::terrain::Terrain;
use bgws_core::{figure, Figure, Figures, Side, SimTime};
use petgraph::graph::EdgeIndex;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObstacleKind {
    Minefield,
    AntiTankDitch,
    DragonsTeeth,
    Wire,
    Abatis,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChangeKind {
    /// A shell or bomb crater at the change position.
    Crater { radius_m: f64, depth_m: f64 },
    /// Collapsed buildings over a radius.
    Rubble { radius_m: f64 },
    /// Forest burnt over a radius: canopy gone, fallen timber.
    BurntForest { radius_m: f64 },
    /// A bridge dropped (road edge index).
    BlownBridge { edge: usize },
    /// A road edge cratered or blocked.
    RoadCut { edge: usize },
    /// A linear obstacle along a polyline with a depth (belt width).
    Obstacle { kind: ObstacleKind, line: Vec<(f64, f64)>, depth_m: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub id: u64,
    pub at: SimTime,
    pub e: f64,
    pub n: f64,
    pub kind: ChangeKind,
    /// Sides that have observed or been told of it.
    #[serde(default)]
    pub known_to: BTreeSet<Side>,
}

/// What a change did.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct ChangeEffect {
    pub cells: Vec<usize>,
    pub edges: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeParams {
    pub crater_gap_m: f64,
    pub rubble_gap_m: f64,
    pub burnt_gap_m: f64,
    /// Gap that stops every class (an emplaced obstacle).
    pub obstacle_gap_m: f64,
}

impl ChangeParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            crater_gap_m: f.get("env.change.crater_gap_m"),
            rubble_gap_m: f.get("env.change.rubble_gap_m"),
            burnt_gap_m: f.get("env.change.burnt_gap_m"),
            obstacle_gap_m: f.get("env.change.obstacle_gap_m"),
        }
    }
}

/// The ordered list of changes in a theatre.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChangeLayer {
    pub changes: Vec<Change>,
}

fn set_gap(t: &mut Terrain, i: usize, j: usize, gap: f64) {
    let cur = t.obstacle_gap.get(i, j);
    let g = gap as f32;
    if cur <= 0.0 || g < cur {
        t.obstacle_gap.set(i, j, g);
    }
}

fn cells_within(t: &Terrain, e: f64, n: f64, r: f64) -> Vec<(usize, usize, f64)> {
    let s = t.spec;
    let mut out = Vec::new();
    let reach = (r / s.cell).ceil() as i64 + 1;
    let (ci, cj) = s.nearest_clamped(e, n);
    for j in (cj as i64 - reach).max(0)..=(cj as i64 + reach).min(s.height as i64 - 1) {
        for i in (ci as i64 - reach).max(0)..=(ci as i64 + reach).min(s.width as i64 - 1) {
            let (x, y) = s.pos(i as usize, j as usize);
            let d = (x - e).hypot(y - n);
            if d <= r {
                out.push((i as usize, j as usize, d));
            }
        }
    }
    out
}

impl ChangeLayer {
    /// Record a change (kept sorted by id).
    pub fn push(&mut self, c: Change) {
        let pos = self.changes.partition_point(|x| x.id < c.id);
        self.changes.insert(pos, c);
    }

    /// Apply one change to terrain and roads.
    pub fn apply_one(c: &Change, t: &mut Terrain, roads: Option<&mut RoadNetwork>, p: &ChangeParams) -> ChangeEffect {
        let mut fx = ChangeEffect::default();
        match &c.kind {
            ChangeKind::Crater { radius_m, depth_m } => {
                for (i, j, d) in cells_within(t, c.e, c.n, *radius_m) {
                    let k = t.spec.idx(i, j);
                    let h = t.elevation.get(i, j) as f64 - depth_m * (1.0 - (d / radius_m).powi(2));
                    t.elevation.set(i, j, h as f32);
                    if matches!(t.land_cover.get(i, j), LandCover::Open | LandCover::Crops) {
                        t.land_cover.set(i, j, LandCover::Bare);
                    }
                    set_gap(t, i, j, p.crater_gap_m);
                    fx.cells.push(k);
                }
            }
            ChangeKind::Rubble { radius_m } => {
                for (i, j, _) in cells_within(t, c.e, c.n, *radius_m) {
                    if t.land_cover.get(i, j).is_urban() {
                        // Rubble keeps its cover and concealment but chokes the going.
                        t.land_cover.set(i, j, LandCover::UrbanDense);
                        set_gap(t, i, j, p.rubble_gap_m);
                        fx.cells.push(t.spec.idx(i, j));
                    }
                }
            }
            ChangeKind::BurntForest { radius_m } => {
                for (i, j, _) in cells_within(t, c.e, c.n, *radius_m) {
                    if t.land_cover.get(i, j).is_forest() {
                        t.land_cover.set(i, j, LandCover::Scrub);
                        set_gap(t, i, j, p.burnt_gap_m);
                        fx.cells.push(t.spec.idx(i, j));
                    }
                }
            }
            ChangeKind::BlownBridge { edge } | ChangeKind::RoadCut { edge } => {
                if let Some(net) = roads {
                    if let Some(ed) = net.graph.edge_weight_mut(EdgeIndex::new(*edge)) {
                        match (&c.kind, ed.bridge.as_mut()) {
                            (ChangeKind::BlownBridge { .. }, Some(b)) => b.blown = true,
                            _ => ed.blocked = true,
                        }
                        fx.edges.push(*edge);
                    }
                }
            }
            ChangeKind::Obstacle { line, depth_m, .. } => {
                let s = t.spec;
                let half = depth_m * 0.5 + s.cell * 0.5;
                for k in 0..s.len() {
                    let (i, j) = s.ij(k);
                    let (x, y) = s.pos(i, j);
                    if polyline_distance(line, x, y) <= half {
                        set_gap(t, i, j, p.obstacle_gap_m);
                        fx.cells.push(k);
                    }
                }
            }
        }
        fx
    }

    /// Apply every change in id order.
    pub fn apply_all(&self, t: &mut Terrain, mut roads: Option<&mut RoadNetwork>, p: &ChangeParams) -> Vec<ChangeEffect> {
        self.changes.iter().map(|c| Self::apply_one(c, t, roads.as_deref_mut(), p)).collect()
    }

    /// Changes a side knows about.
    pub fn known_to(&self, side: Side) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |c| c.known_to.contains(&side))
    }
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("env.change.crater_gap_m", 4.0, "m", 0.5, 30.0, "default", Default, "clear gap between craters in a cratered cell"),
        figure!("env.change.rubble_gap_m", 2.0, "m", 0.3, 10.0, "default", Default, "clear gap through rubble"),
        figure!("env.change.burnt_gap_m", 5.0, "m", 0.5, 30.0, "default", Default, "clear gap through fallen burnt timber"),
        figure!("env.change.obstacle_gap_m", 0.1, "m", 0.01, 0.5, "R4.5", Default, "an emplaced obstacle belt stops all classes until breached"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::GridSpec;
    use crate::soil::{trafficability, TrafficParams, VehicleClass};
    use bgws_core::Geodetic;

    #[test]
    fn crater_obstacle_and_burn() {
        let mut all = figures();
        all.extend(crate::soil::figures());
        all.extend(crate::landcover::figures());
        let f = Figures::from_list(all).unwrap();
        let p = ChangeParams::from_figures(&f);
        let tp = TrafficParams::from_figures(&f);
        let spec = GridSpec::new(50, 50, 10.0, 0.0, 0.0);
        let mut t = Terrain::flat(Geodetic::default(), spec, 100.0);
        t.land_cover = crate::raster::Raster::from_fn(spec, |i, _| if i > 40 { LandCover::ForestDense } else { LandCover::Open });
        let mut layer = ChangeLayer::default();
        layer.push(Change { id: 2, at: SimTime::ZERO, e: 0.0, n: 0.0, kind: ChangeKind::Obstacle { kind: ObstacleKind::Minefield, line: vec![(200.0, 0.0), (200.0, 490.0)], depth_m: 20.0 }, known_to: BTreeSet::new() });
        layer.push(Change { id: 1, at: SimTime::ZERO, e: 100.0, n: 100.0, kind: ChangeKind::Crater { radius_m: 6.0, depth_m: 2.0 }, known_to: [Side::Blue].into() });
        layer.push(Change { id: 3, at: SimTime::ZERO, e: 450.0, n: 250.0, kind: ChangeKind::BurntForest { radius_m: 30.0 }, known_to: BTreeSet::new() });
        assert_eq!(layer.changes.iter().map(|c| c.id).collect::<Vec<_>>(), vec![1, 2, 3]);
        let fx = layer.apply_all(&mut t, None, &p);
        assert!((t.height(100.0, 100.0) - 98.0).abs() < 1e-4);
        assert_eq!(t.land_cover_at(100.0, 100.0), LandCover::Bare);
        assert!(fx[1].cells.len() >= 50);
        let going = t.going(200.0, 250.0);
        assert!(trafficability(&going, VehicleClass::Tracked, &tp).immobilised);
        assert!(trafficability(&going, VehicleClass::Foot, &tp).immobilised);
        assert_eq!(t.land_cover_at(450.0, 250.0), LandCover::Scrub);
        assert_eq!(layer.known_to(Side::Blue).count(), 1);
    }
}
