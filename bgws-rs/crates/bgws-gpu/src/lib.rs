//! The kernel layer (RUST_ENGINE.md §11): batched line of sight, viewsheds,
//! candidate pairs, path-loss and J/S fields, cost→flow fields and a Monte
//! Carlo hit-distribution kernel. Every kernel has a deterministic,
//! rayon-parallel CPU twin; behind the `gpu` feature, wgpu compute shaders
//! implement LOS, viewsheds and candidate pairs, selected by [`Backend::auto`]
//! when an adapter exists.
//!
//! Kernels take plain arrays only (this crate depends on `bgws-core` alone),
//! and outputs that feed decisions are quantised (§12): clearance in
//! centimetres, visibility from the quantised value, fields to 0.01 dB.
//! "The GPU narrows, the CPU decides" (rule 9).

pub mod backend;
pub mod field;
pub mod flow;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod grid;
pub mod los;
pub mod montecarlo;
pub mod pairs;
pub mod viewshed;

pub use backend::{Backend, CpuKernels, Kernels};
pub use field::{fspl_db, knife_edge_j, DbField, FieldEmitter, PathLossParams, PowerSource};
pub use flow::{FlowField, DIRS, NO_DIR};
pub use grid::{GridError, HeightGrid};
pub use los::{LosParams, LosQuery, LosResult};
pub use montecarlo::{HitDistribution, HitParams};
pub use pairs::{CandidatePair, PairEntity, PairParams};
pub use viewshed::{Viewshed, ViewshedObserver};

use bgws_core::{figure, Figure};

/// The figures this crate's kernels read.
pub fn figures() -> Vec<Figure> {
    vec![
        figure!("gpu.los.step_cells", 0.5, "fraction", 0.1, 1.0, "default", Default,
            "LOS/profile sample spacing as a fraction of the grid spacing; smaller is slower and catches thinner ridges"),
        figure!("gpu.los.clearance_quantum_m", 0.01, "m", 0.001, 0.1, "default", Default,
            "quantum of LOS clearance outputs (§12: centimetres)"),
        figure!("gpu.los.refine_margin_m", 0.5, "m", 0.0, 10.0, "default", Default,
            "GPU LOS results closer to grazing than this are recomputed on the CPU in f64"),
        figure!("gpu.field.quantum_db", 0.01, "dB", 0.001, 0.5, "default", Default,
            "quantum of path-loss, power and J/S field outputs"),
        figure!("gpu.pairs.hash_cell_m", 0.0, "m", 0.0, 1.0e6, "default", Default,
            "candidate-pair hash cell size; 0 = the largest sensor range"),
        figure!("gpu.pairs.gpu_slack", 1.0e-3, "fraction", 0.0, 0.05, "default", Default,
            "relative range/angle slack of the GPU pair superset before exact CPU refinement"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_register_and_build_params() {
        let f = bgws_core::Figures::from_list(figures()).unwrap();
        let p = LosParams::from_figures(&f, 1.15);
        assert_eq!(p.step_cells, 0.5);
        let _ = PairParams::from_figures(&f);
        let _ = PathLossParams::from_figures(&f, 2.0);
        assert!(f.unused().is_empty(), "unused: {:?}", f.unused());
    }

    #[test]
    fn backend_auto_runs_every_kernel() {
        let b = Backend::auto();
        let g = HeightGrid::flat(8, 8, 10.0, 0.0);
        let r = b.los_batch(&g, &[LosQuery { tgt_x: 70.0, obs_agl: 2.0, tgt_agl: 2.0, ..Default::default() }], &LosParams::default());
        assert!(r[0].visible);
        let v = b.viewshed(&g, &[ViewshedObserver { x: 0.0, y: 0.0, agl: 2.0, max_range: 1e9 }], 1.0, &LosParams::default());
        assert_eq!(v.coverage(), 1.0);
        let f = b.flow_field(8, 8, 10.0, &[1.0; 64], &[0]);
        assert_eq!(f.integration[0], 0.0);
    }
}
