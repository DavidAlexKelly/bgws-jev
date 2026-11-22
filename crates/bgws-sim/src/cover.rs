//! Cover from the features on the ground (§7.1): a wall or building corner
//! to get behind, a tree line, hedgerow or garden trees, a railway's bank, a
//! fold or ditch in the ground. These are where small units actually take
//! cover, 5–20 m features that a 30 m land-cover class with a random draw per
//! cell only stands in for.
//!
//! A point's feature cover is read from the pack's 5 m buildings-and-trees
//! surface and railways, and from the terrain's shape, round the point; it
//! is combined with the class's draw (the unmapped walls, ditches and
//! shell scrapes) by taking the better of the two, so mapped features only
//! ever add. Generated ground has no surface: there only the folds of the
//! ground and railways count.

use crate::ground::Ground;
use bgws_core::{figure, Figure, Figures};
use bgws_env::landcover::MicroTerrain;

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.cover.wall", 0.6, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire right beside a building (its wall or corner to get behind)"),
        figure!("sim.cover.wall_conceal", 0.5, "fraction", 0.0, 1.0, "default", Low, "concealment right beside a building"),
        figure!("sim.cover.wall_reach_m", 6.0, "m", 2.0, 20.0, "default", Default, "how close to a building a unit must be for its walls to cover it fully (half that cover at twice the distance)"),
        figure!("sim.cover.trees", 0.3, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire in a tree line or among garden trees (trunks, roots, the bank a hedge grows on)"),
        figure!("sim.cover.trees_conceal", 0.7, "fraction", 0.0, 1.0, "default", Low, "concealment in a tree line or among garden trees"),
        figure!("sim.cover.trees_full_share", 0.25, "fraction", 0.05, 1.0, "default", Default, "share of the ground within 10 m under trees at which a point counts as fully in a tree line (a line through the point covers about this much)"),
        figure!("sim.cover.rail_bank", 0.45, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire behind a railway's bank or ballast"),
        figure!("sim.cover.rail_conceal", 0.3, "fraction", 0.0, 1.0, "default", Low, "concealment at a railway's bank"),
        figure!("sim.cover.fold_per_m", 0.3, "fraction/m", 0.0, 1.0, "default", Low, "cover from direct fire per metre a point lies below the ground round it (a fold, hollow or ditch)"),
        figure!("sim.cover.fold_max", 0.6, "fraction", 0.0, 1.0, "default", Low, "most cover a fold of the ground gives"),
        figure!("sim.cover.hull_mask", 0.4, "fraction", 0.1, 1.0, "default", Low, "a vehicle halted behind features that mask this much (a wall, a railway bank, a fold of the ground; not trees or hedges, which hide but do not stop rounds) counts as hull-down at once"),
        figure!("sim.cover.fold_radius_m", 25.0, "m", 5.0, 100.0, "default", Default, "how far round a point the ground is compared with it to find a fold (at least the terrain's cell)"),
    ]
}

#[derive(Clone, Copy, Debug)]
pub struct FeatureCoverParams {
    pub wall: f64,
    pub wall_conceal: f64,
    pub wall_reach_m: f64,
    pub trees: f64,
    pub trees_conceal: f64,
    pub trees_full_share: f64,
    pub rail_bank: f64,
    pub rail_conceal: f64,
    pub fold_per_m: f64,
    pub fold_max: f64,
    pub fold_radius_m: f64,
    pub hull_mask: f64,
}

impl FeatureCoverParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            wall: f.get("sim.cover.wall"),
            wall_conceal: f.get("sim.cover.wall_conceal"),
            wall_reach_m: f.get("sim.cover.wall_reach_m"),
            trees: f.get("sim.cover.trees"),
            trees_conceal: f.get("sim.cover.trees_conceal"),
            trees_full_share: f.get("sim.cover.trees_full_share"),
            rail_bank: f.get("sim.cover.rail_bank"),
            rail_conceal: f.get("sim.cover.rail_conceal"),
            fold_per_m: f.get("sim.cover.fold_per_m"),
            fold_max: f.get("sim.cover.fold_max"),
            fold_radius_m: f.get("sim.cover.fold_radius_m"),
            hull_mask: f.get("sim.cover.hull_mask"),
        }
    }
}

/// What each feature gives at a point (0: not there), for the inspector and tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureCover {
    /// Distance to the nearest building found round the point, m.
    pub building_m: Option<f64>,
    /// Share of the ground within 10 m under trees.
    pub tree_share: f64,
    pub rail: bool,
    /// How far the point lies below the ground round it, m.
    pub fold_m: f64,
    /// Together: 1 − Π(1 − each).
    pub cover: f64,
    pub concealment: f64,
    /// The part of the cover from features that mask a vehicle's hull (walls,
    /// railway banks, folds): trees and hedges hide but do not stop rounds.
    pub masking: f64,
}

/// The eight directions round a point.
const D: f64 = std::f64::consts::FRAC_1_SQRT_2;
const RING: [(f64, f64); 8] = [(1.0, 0.0), (D, D), (0.0, 1.0), (-D, D), (-1.0, 0.0), (-D, -D), (0.0, -1.0), (D, -D)];

/// The cover the features round (e, n) give.
pub fn feature_cover(g: &Ground, e: f64, n: f64) -> FeatureCover {
    let p = &g.feature_cover;
    let mut out = FeatureCover::default();
    let mut cover = Vec::with_capacity(4);
    let mut conceal = Vec::with_capacity(4);
    let mut masks = Vec::with_capacity(3);
    // Buildings and trees, from the 5 m surface: the point and rings at 5 and 10 m.
    if let Some(s) = g.sight.as_ref() {
        let mut trees = 0usize;
        let mut samples = 0usize;
        for (r, dirs) in [(0.0, &RING[..1]), (5.0, &RING[..]), (10.0, &RING[..])] {
            for &(dx, dy) in dirs {
                let (h, tree) = s.at(e + dx * r, n + dy * r);
                samples += 1;
                if tree.is_some() {
                    trees += 1;
                }
                if h >= 2.0 && out.building_m.is_none_or(|d| r < d) {
                    out.building_m = Some(r);
                }
            }
        }
        out.tree_share = trees as f64 / samples as f64;
        if let Some(d) = out.building_m {
            // Fully within the reach, half at twice it, none beyond.
            let k = if d <= p.wall_reach_m {
                1.0
            } else if d <= 2.0 * p.wall_reach_m {
                0.5
            } else {
                0.0
            };
            cover.push(p.wall * k);
            masks.push(p.wall * k);
            conceal.push(p.wall_conceal * k);
        }
        if out.tree_share > 0.0 {
            let k = (out.tree_share / p.trees_full_share.max(1e-6)).min(1.0);
            cover.push(p.trees * k);
            conceal.push(p.trees_conceal * k);
        }
    }
    // A railway's bank: on the line or beside it.
    if g.rails.is_some() {
        let r = g.terrain.spec.cell * 0.5;
        out.rail = g.rail_at(e, n) || RING.iter().step_by(2).any(|&(dx, dy)| g.rail_at(e + dx * r, n + dy * r));
        if out.rail {
            cover.push(p.rail_bank);
            masks.push(p.rail_bank);
            conceal.push(p.rail_conceal);
        }
    }
    // A fold: the point below the ground round it.
    let r = p.fold_radius_m.max(g.terrain.spec.cell);
    let here = g.height(e, n);
    let round = RING.iter().map(|&(dx, dy)| g.height(e + dx * r, n + dy * r)).sum::<f64>() / RING.len() as f64;
    out.fold_m = (round - here).max(0.0);
    if out.fold_m > 0.0 {
        let c = (out.fold_m * p.fold_per_m).min(p.fold_max);
        cover.push(c);
        masks.push(c);
        conceal.push(c * 0.8);
    }
    let join = |v: &[f64]| (1.0 - v.iter().map(|c| 1.0 - c.clamp(0.0, 1.0)).product::<f64>()).min(0.95);
    out.cover = join(&cover);
    out.concealment = join(&conceal);
    out.masking = join(&masks);
    out
}

/// Whether a vehicle halted here is hull-down behind what masks it (wall,
/// railway bank, fold), whatever its settling time.
pub fn masks_hull(g: &Ground, e: f64, n: f64) -> bool {
    feature_cover(g, e, n).masking >= g.feature_cover.hull_mask
}

/// The class's draw with the features' cover: the better of the two.
pub fn with_features(m: MicroTerrain, f: &FeatureCover) -> MicroTerrain {
    MicroTerrain { cover: m.cover.max(f.cover), concealment: m.concealment.max(f.concealment) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::default_figures;
    use bgws_core::Geodetic;
    use bgws_env::surface::SightSurface;
    use bgws_env::{GridSpec, LandCover, SightView};
    use std::sync::Arc;

    fn ground() -> Ground {
        Ground::flat(Geodetic::new(54.7, 20.5, 0.0), 1000.0, &default_figures())
    }

    #[test]
    fn open_flat_ground_gives_nothing() {
        let g = ground();
        let f = feature_cover(&g, 0.0, 0.0);
        assert_eq!((f.cover, f.concealment, f.building_m, f.rail), (0.0, 0.0, None, false));
    }

    #[test]
    fn walls_tree_lines_and_folds_give_cover() {
        let mut g = ground();
        // A 5 m surface: a building at (100, 0), a line of trees along n = 300.
        let spec = GridSpec::new(400, 400, 5.0, -1000.0, -1000.0);
        let mut s = SightSurface::empty(spec);
        let near = |e: f64, n: f64| spec.nearest(e, n).map(|(i, j)| spec.idx(i, j)).unwrap();
        for de in [0.0, 5.0, 10.0] {
            for dn in [0.0, 5.0, 10.0] {
                s.building_m[near(100.0 + de, dn)] = 8;
            }
        }
        for k in -40..40 {
            s.tree[near(k as f64 * 5.0, 300.0)] = LandCover::ForestLight as u8 + 1;
        }
        g.sight = Some(SightView::same_frame(Arc::new(s)));
        let p = g.feature_cover;
        let by_wall = feature_cover(&g, 95.0, 0.0);
        assert_eq!(by_wall.building_m, Some(5.0));
        assert!((by_wall.cover - p.wall).abs() < 1e-9, "{by_wall:?}");
        let further = feature_cover(&g, 90.0, 0.0);
        assert!(further.cover < by_wall.cover && further.cover > 0.0, "{further:?}");
        let in_line = feature_cover(&g, 0.0, 300.0);
        assert!((in_line.cover - p.trees).abs() < 1e-9 && in_line.concealment > 0.6, "{in_line:?}");
        // Trees hide a vehicle but do not mask its hull; a wall does.
        assert_eq!(in_line.masking, 0.0);
        assert!(by_wall.masking >= p.hull_mask && masks_hull(&g, 95.0, 0.0));
        assert!(!masks_hull(&g, 0.0, 300.0));
        let open = feature_cover(&g, 0.0, 200.0);
        assert_eq!(open.cover, 0.0);
        // A hollow: the ground round (−500, −500) raised 2 m.
        let t = &mut g.terrain;
        let c = t.spec.nearest(-500.0, -500.0).unwrap();
        for j in 0..t.spec.height {
            for i in 0..t.spec.width {
                if (i as i64 - c.0 as i64).abs() + (j as i64 - c.1 as i64).abs() > 0 {
                    let k = t.spec.idx(i, j);
                    t.elevation.data[k] = 2.0;
                }
            }
        }
        let (e, n) = t.spec.pos(c.0, c.1);
        let hollow = feature_cover(&g, e, n);
        assert!(hollow.fold_m > 1.5 && hollow.cover > 0.4, "{hollow:?}");
        assert!(hollow.masking >= g.feature_cover.hull_mask, "a 2 m hollow masks a hull: {hollow:?}");
    }
}
