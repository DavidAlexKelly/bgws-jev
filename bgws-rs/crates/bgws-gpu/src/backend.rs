//! Backend selection (§11.3): the `Kernels` trait every caller uses, a CPU
//! implementation that is always available, and — behind the `gpu` feature —
//! a wgpu implementation chosen when an adapter exists. Kernels without a
//! GPU version fall through to the CPU twin by the trait's default methods.

use crate::field::{self, DbField, FieldEmitter, PathLossParams, PowerSource};
use crate::flow::{self, FlowField};
use crate::grid::HeightGrid;
use crate::los::{self, LosParams, LosQuery, LosResult};
use crate::montecarlo::{self, HitDistribution, HitParams};
use crate::pairs::{self, CandidatePair, PairEntity, PairParams};
use crate::viewshed::{self, Viewshed, ViewshedObserver};

/// The kernel layer. Default methods are the CPU twins.
pub trait Kernels: Send + Sync {
    /// "cpu" or the GPU adapter's name.
    fn name(&self) -> String {
        "cpu".into()
    }
    fn is_gpu(&self) -> bool {
        false
    }
    fn los_batch(&self, grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Vec<LosResult> {
        los::los_batch_cpu(grid, queries, p)
    }
    fn viewshed(&self, grid: &HeightGrid, observers: &[ViewshedObserver], target_agl: f64, p: &LosParams) -> Viewshed {
        viewshed::viewshed_cpu(grid, observers, target_agl, p)
    }
    fn candidate_pairs(&self, entities: &[PairEntity], p: &PairParams) -> Vec<CandidatePair> {
        pairs::candidate_pairs_cpu(entities, p)
    }
    fn path_loss_field(&self, grid: &HeightGrid, e: &FieldEmitter, p: &PathLossParams) -> DbField {
        field::path_loss_field_cpu(grid, e, p)
    }
    fn received_power_field(&self, sources: &[PowerSource<'_>], rx_gain_db: f64, quantum_db: f64) -> Option<DbField> {
        field::received_power_field_cpu(sources, rx_gain_db, quantum_db)
    }
    fn js_field(&self, jamming: &DbField, signal: &DbField, quantum_db: f64) -> Option<DbField> {
        field::js_field_cpu(jamming, signal, quantum_db)
    }
    fn flow_field(&self, width: usize, height: usize, cell: f64, cost: &[f32], goals: &[usize]) -> FlowField {
        flow::flow_field_cpu(width, height, cell, cost, goals)
    }
    fn hit_distribution(&self, p: &HitParams) -> HitDistribution {
        montecarlo::hit_distribution_cpu(p)
    }
}

/// The CPU twins (rayon-parallel, deterministic).
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuKernels;

impl Kernels for CpuKernels {}

/// The selected backend.
pub enum Backend {
    Cpu(CpuKernels),
    #[cfg(feature = "gpu")]
    Gpu(Box<crate::gpu::GpuKernels>),
}

impl std::fmt::Debug for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Backend({})", self.name())
    }
}

impl Default for Backend {
    fn default() -> Self {
        Backend::Cpu(CpuKernels)
    }
}

impl Backend {
    pub fn cpu() -> Self {
        Backend::Cpu(CpuKernels)
    }

    /// The GPU when the `gpu` feature is on and an adapter exists, else the CPU.
    pub fn auto() -> Self {
        #[cfg(feature = "gpu")]
        {
            if let Ok(g) = crate::gpu::GpuKernels::new() {
                return Backend::Gpu(Box::new(g));
            }
        }
        Backend::cpu()
    }

    fn inner(&self) -> &dyn Kernels {
        match self {
            Backend::Cpu(c) => c,
            #[cfg(feature = "gpu")]
            Backend::Gpu(g) => g.as_ref(),
        }
    }
}

impl Kernels for Backend {
    fn name(&self) -> String {
        self.inner().name()
    }
    fn is_gpu(&self) -> bool {
        self.inner().is_gpu()
    }
    fn los_batch(&self, grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Vec<LosResult> {
        self.inner().los_batch(grid, queries, p)
    }
    fn viewshed(&self, grid: &HeightGrid, observers: &[ViewshedObserver], target_agl: f64, p: &LosParams) -> Viewshed {
        self.inner().viewshed(grid, observers, target_agl, p)
    }
    fn candidate_pairs(&self, entities: &[PairEntity], p: &PairParams) -> Vec<CandidatePair> {
        self.inner().candidate_pairs(entities, p)
    }
    fn path_loss_field(&self, grid: &HeightGrid, e: &FieldEmitter, p: &PathLossParams) -> DbField {
        self.inner().path_loss_field(grid, e, p)
    }
    fn received_power_field(&self, sources: &[PowerSource<'_>], rx_gain_db: f64, quantum_db: f64) -> Option<DbField> {
        self.inner().received_power_field(sources, rx_gain_db, quantum_db)
    }
    fn js_field(&self, jamming: &DbField, signal: &DbField, quantum_db: f64) -> Option<DbField> {
        self.inner().js_field(jamming, signal, quantum_db)
    }
    fn flow_field(&self, width: usize, height: usize, cell: f64, cost: &[f32], goals: &[usize]) -> FlowField {
        self.inner().flow_field(width, height, cell, cost, goals)
    }
    fn hit_distribution(&self, p: &HitParams) -> HitDistribution {
        self.inner().hit_distribution(p)
    }
}
