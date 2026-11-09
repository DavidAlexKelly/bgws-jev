//! Urban blocks: the town as the fight sees it. A block is a run of
//! built-over terrain cells bounded by streets (cells a road crosses) or
//! open ground, found when a pack's sight surface is loaded. Each block
//! keeps what the fight needs of it:
//!
//! - its footprint (the ground under roofs), area and density;
//! - its buildings' mean and greatest height, and floors;
//! - an estimated material by height: low houses are light (timber, thin
//!   brick), mid-rise is masonry, high-rise reinforced concrete. OSM rarely
//!   says what a building is made of, so this is a rule of thumb;
//! - how much of it still stands.
//!
//! What it does:
//!
//! - **cover and concealment:** a unit in a block has its material's cover
//!   (against direct fire and suppression) and the block's concealment;
//! - **protection from indirect fire:** troops halted in a block are under
//!   roofs and walls; a burst's lethality on them falls by the material's
//!   protection, times how much still stands (rubble has no roof);
//! - **upper floors:** troops halted in a block look out from its upper
//!   floors, up to `sim.urban.observe_floors`;
//! - **damage and rubble:** a heavy round in a block brings down part of its
//!   footprint (less of stronger material). Sight lines see its buildings
//!   lower by the share fallen, and rubble slows the going through it.

use bgws_core::{figure, Figure, Figures};
use bgws_env::{GridSpec, Raster};
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Material {
    Light,
    Masonry,
    Concrete,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: u32,
    /// Centre of its cells, m.
    pub e: f64,
    pub n: f64,
    /// Ground area of its cells, m².
    pub area_m2: f64,
    /// Ground under roofs, m².
    pub footprint_m2: f64,
    /// Footprint over area.
    pub density: f64,
    pub mean_height_m: f64,
    pub max_height_m: f64,
    pub floors: u32,
    pub material: Material,
    /// Share of its footprint still standing (1 untouched, 0 all rubble).
    pub standing: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UrbanParams {
    pub cover: [f64; 3],
    pub protect: [f64; 3],
    pub conceal: f64,
    pub floor_m: f64,
    pub observe_floors: f64,
    pub rubble_m2: f64,
    pub resist: [f64; 3],
    pub rubble_slow: f64,
}

pub fn figures() -> Vec<Figure> {
    vec![
        figure!("sim.urban.cover_light", 0.6, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire in a block of light buildings (houses, timber, thin brick)"),
        figure!("sim.urban.cover_masonry", 0.8, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire in a block of masonry buildings"),
        figure!("sim.urban.cover_concrete", 0.9, "fraction", 0.0, 1.0, "default", Low, "cover from direct fire in a block of reinforced-concrete buildings"),
        figure!("sim.urban.protect_light", 0.5, "fraction", 0.0, 1.0, "default", Low, "share of a burst's lethality stopped for troops halted in light buildings"),
        figure!("sim.urban.protect_masonry", 0.75, "fraction", 0.0, 1.0, "default", Low, "share of a burst's lethality stopped for troops halted in masonry buildings"),
        figure!("sim.urban.protect_concrete", 0.9, "fraction", 0.0, 1.0, "default", Low, "share of a burst's lethality stopped for troops halted in concrete buildings"),
        figure!("sim.urban.conceal", 0.85, "fraction", 0.0, 1.0, "default", Low, "concealment of a unit among a block's buildings"),
        figure!("sim.urban.floor_m", 3.0, "m", 2.5, 4.5, "default", Default, "height of one storey"),
        figure!("sim.urban.observe_floors", 4.0, "count", 1.0, 20.0, "default", Low, "highest floor troops in a block climb to to watch from"),
        figure!("sim.urban.rubble_m2_per_heavy_round", 60.0, "m2", 5.0, 400.0, "default", Low, "footprint of light buildings one 152/155 mm shell or heavy rocket brings down"),
        figure!("sim.urban.resist_masonry", 0.5, "fraction", 0.05, 1.0, "default", Low, "masonry brought down per round, against light buildings"),
        figure!("sim.urban.resist_concrete", 0.25, "fraction", 0.05, 1.0, "default", Low, "reinforced concrete brought down per round, against light buildings"),
        figure!("sim.urban.rubble_slow", 0.6, "fraction", 0.0, 0.95, "default", Low, "speed lost by vehicles crossing a block that is all rubble (half that on foot)"),
    ]
}

impl UrbanParams {
    pub fn from_figures(f: &Figures) -> Self {
        Self {
            cover: [f.get("sim.urban.cover_light"), f.get("sim.urban.cover_masonry"), f.get("sim.urban.cover_concrete")],
            protect: [f.get("sim.urban.protect_light"), f.get("sim.urban.protect_masonry"), f.get("sim.urban.protect_concrete")],
            conceal: f.get("sim.urban.conceal"),
            floor_m: f.get("sim.urban.floor_m"),
            observe_floors: f.get("sim.urban.observe_floors"),
            rubble_m2: f.get("sim.urban.rubble_m2_per_heavy_round"),
            resist: [1.0, f.get("sim.urban.resist_masonry"), f.get("sim.urban.resist_concrete")],
            rubble_slow: f.get("sim.urban.rubble_slow"),
        }
    }
}

impl Material {
    /// By the buildings' mean height: houses, mid-rise, high-rise.
    pub fn by_height(mean_m: f64) -> Material {
        if mean_m < 7.5 {
            Material::Light
        } else if mean_m < 15.0 {
            Material::Masonry
        } else {
            Material::Concrete
        }
    }
    fn k(self) -> usize {
        self as usize
    }
}

/// A terrain cell's buildings, from the sight surface.
#[derive(Clone, Copy, Debug, Default)]
pub struct CellBuilt {
    /// Share of the cell under roofs.
    pub share: f64,
    pub mean_height_m: f64,
    pub max_height_m: f64,
}

/// Every block on the ground: the urban part of the feature store.
#[derive(Clone, Debug)]
pub struct Urban {
    /// Block id + 1 in each terrain cell (0 none).
    pub id: Arc<Raster<u32>>,
    pub blocks: Vec<Block>,
    /// Each block's terrain cells.
    pub cells: Arc<Vec<Vec<u32>>>,
    /// Standing share by terrain cell (×255), for sight lines: shared with the sight view.
    pub standing: Arc<Raster<u8>>,
    pub p: UrbanParams,
}

impl Urban {
    /// Label the blocks: built-over cells joined to their eight neighbours,
    /// broken where a road crosses (`road[k]`).
    pub fn find(spec: GridSpec, built: &[CellBuilt], road: impl Fn(usize) -> bool, p: UrbanParams) -> Option<Urban> {
        let (w, h) = (spec.width, spec.height);
        let mut id = vec![0u32; spec.len()];
        let mut blocks = Vec::new();
        let mut cells: Vec<Vec<u32>> = Vec::new();
        let in_block = |k: usize| built[k].share > 0.0 && !road(k);
        let mut stack = Vec::new();
        for start in 0..spec.len() {
            if id[start] != 0 || !in_block(start) {
                continue;
            }
            let b = blocks.len() as u32;
            id[start] = b + 1;
            stack.push(start);
            let mut mine = Vec::new();
            while let Some(k) = stack.pop() {
                mine.push(k as u32);
                let (i, j) = ((k % w) as i64, (k / w) as i64);
                for dj in -1..=1i64 {
                    for di in -1..=1i64 {
                        let (x, y) = (i + di, j + dj);
                        if (di, dj) == (0, 0) || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                            continue;
                        }
                        let q = y as usize * w + x as usize;
                        if id[q] == 0 && in_block(q) {
                            id[q] = b + 1;
                            stack.push(q);
                        }
                    }
                }
            }
            let cell_m2 = spec.cell * spec.cell;
            let (mut e, mut n, mut fp, mut hsum, mut hmax) = (0.0, 0.0, 0.0, 0.0, 0.0f64);
            for &k in &mine {
                let (x, y) = spec.pos(k as usize % w, k as usize / w);
                e += x;
                n += y;
                let c = built[k as usize];
                fp += c.share * cell_m2;
                hsum += c.share * cell_m2 * c.mean_height_m;
                hmax = hmax.max(c.max_height_m);
            }
            let m = mine.len() as f64;
            let mean = if fp > 0.0 { hsum / fp } else { 0.0 };
            let floors = (mean / p.floor_m).round().max(1.0) as u32;
            blocks.push(Block {
                id: b,
                e: e / m,
                n: n / m,
                area_m2: m * cell_m2,
                footprint_m2: fp,
                density: fp / (m * cell_m2),
                mean_height_m: mean,
                max_height_m: hmax,
                floors,
                material: Material::by_height(mean),
                standing: 1.0,
            });
            cells.push(mine);
        }
        if blocks.is_empty() {
            return None;
        }
        Some(Urban { id: Arc::new(Raster { spec, data: id }), blocks, cells: Arc::new(cells), standing: Arc::new(Raster::filled(spec, 255)), p })
    }

    /// The block at a point, if any.
    pub fn at(&self, e: f64, n: f64) -> Option<&Block> {
        let s = &self.id.spec;
        let (i, j) = s.nearest(e, n)?;
        let b = self.id.data[s.idx(i, j)];
        (b > 0).then(|| &self.blocks[b as usize - 1])
    }

    /// Cover from direct fire in a block's buildings.
    pub fn cover(&self, b: &Block) -> f64 {
        self.p.cover[b.material.k()]
    }

    /// The share of a burst's lethality its buildings stop for troops halted in it.
    pub fn protection(&self, b: &Block) -> f64 {
        self.p.protect[b.material.k()] * b.standing
    }

    /// Eye height above the ground for troops watching from its upper floors.
    pub fn eye_m(&self, b: &Block) -> f64 {
        let floors = (b.floors as f64 * b.standing).floor().min(self.p.observe_floors).max(1.0);
        (floors - 1.0) * self.p.floor_m + 1.5
    }

    /// Going through a block's rubble: the factor its speed keeps.
    pub fn going(&self, e: f64, n: f64, foot: bool) -> f64 {
        match self.at(e, n) {
            Some(b) if b.standing < 1.0 => 1.0 - self.p.rubble_slow * (1.0 - b.standing) * if foot { 0.5 } else { 1.0 },
            _ => 1.0,
        }
    }

    /// A heavy round bursts at a point: part of its block comes down. The
    /// block's id and what is left standing, when it hit one.
    pub fn strike(&mut self, e: f64, n: f64) -> Option<(u32, f64)> {
        let s = self.id.spec;
        let (i, j) = s.nearest(e, n)?;
        let b = self.id.data[s.idx(i, j)];
        if b == 0 {
            return None;
        }
        let k = b as usize - 1;
        let blk = &mut self.blocks[k];
        if blk.footprint_m2 <= 0.0 {
            return None;
        }
        let down = self.p.rubble_m2 * self.p.resist[blk.material.k()] / blk.footprint_m2;
        blk.standing = (blk.standing - down).max(0.0);
        let v = (blk.standing * 255.0).round() as u8;
        let st = Arc::make_mut(&mut self.standing);
        for &c in &self.cells[k] {
            st.data[c as usize] = v;
        }
        Some((blk.id, blk.standing))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> UrbanParams {
        UrbanParams::from_figures(&Figures::from_list(figures()).unwrap())
    }

    #[test]
    fn streets_split_blocks_and_rounds_bring_them_down() {
        // A 10 × 4 strip of 30 m cells, all built over at 9 m, with a road down column 5.
        let spec = GridSpec::new(10, 4, 30.0, 0.0, 0.0);
        let built: Vec<CellBuilt> = (0..spec.len()).map(|_| CellBuilt { share: 0.5, mean_height_m: 9.0, max_height_m: 12.0 }).collect();
        let mut u = Urban::find(spec, &built, |k| k % 10 == 5, params()).unwrap();
        assert_eq!(u.blocks.len(), 2);
        let west = u.at(30.0, 30.0).unwrap().clone();
        assert_eq!(west.material, Material::Masonry);
        assert_eq!(west.floors, 3);
        assert!((west.footprint_m2 - 0.5 * 900.0 * 20.0).abs() < 1e-6);
        assert!(u.at(150.0, 30.0).is_none(), "the street is not in a block");
        // Masonry: 30 m² down per round, of a 9,000 m² footprint.
        let (_, left) = u.strike(30.0, 30.0).unwrap();
        assert!((left - (1.0 - 30.0 / 9_000.0)).abs() < 1e-9);
        assert!(u.standing.data[0] < 255);
        assert_eq!(u.standing.data[9], 255, "the other block stands");
        // Rubble: less overhead protection, slower going, lower windows.
        for _ in 0..200 {
            u.strike(30.0, 30.0);
        }
        let b = u.at(30.0, 30.0).unwrap();
        assert!(b.standing < 0.4);
        assert!(u.protection(b) < 0.3);
        assert!(u.going(30.0, 30.0, false) < 0.7);
        assert!(u.eye_m(b) < u.eye_m(u.at(270.0, 30.0).unwrap()));
    }
}
