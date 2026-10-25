//! The wgpu backend (§11.3, feature `gpu`): WGSL compute shaders for batched
//! LOS, viewsheds and candidate pairs; every other kernel falls back to its
//! CPU twin through the `Kernels` default methods.
//!
//! Floating point on the GPU is f32 in grid-local coordinates. Decisions
//! stay exact (§12, rule 9): LOS results within `refine_margin_m` of zero
//! are recomputed on the CPU in f64, and candidate pairs are a slack-widened
//! superset that the CPU filters with the exact test. Viewsheds (overlays,
//! routing) are used as computed.
//!
//! The height grid is uploaded per call; keeping tiles resident as texture
//! arrays paged by area of interest is the next step.

use crate::backend::Kernels;
use crate::grid::HeightGrid;
use crate::los::{self, LosParams, LosQuery, LosResult};
use crate::pairs::{self, CandidatePair, PairEntity, PairParams};
use crate::viewshed::{self, Viewshed, ViewshedObserver};
use bgws_core::geo::EARTH_RADIUS;
use bytemuck::{Pod, Zeroable};
use thiserror::Error;
use wgpu::util::DeviceExt;

#[derive(Debug, Error)]
pub enum GpuError {
    #[error("no GPU adapter is available")]
    NoAdapter,
    #[error("device request failed: {0}")]
    Device(String),
    #[error("buffer readback failed: {0}")]
    Readback(String),
    #[error("input too large for one dispatch")]
    TooLarge,
}

const COMMON: &str = include_str!("shaders/common.wgsl");
const LOS_WGSL: &str = include_str!("shaders/los.wgsl");
const VIEWSHED_WGSL: &str = include_str!("shaders/viewshed.wgsl");
/// Pairs does not use the terrain bindings, so it stands alone.
pub const PAIRS_WGSL: &str = include_str!("shaders/pairs.wgsl");

/// Full source of the LOS shader (common + kernel), for validation.
pub fn los_source() -> String {
    format!("{COMMON}\n{LOS_WGSL}")
}
/// Full source of the viewshed shader.
pub fn viewshed_source() -> String {
    format!("{COMMON}\n{VIEWSHED_WGSL}")
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridParams {
    width: u32,
    height: u32,
    count: u32,
    count2: u32,
    cell: f32,
    inv_2kr: f32,
    step_cells: f32,
    extra: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GQuery {
    v: [f32; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GObs {
    v: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GPairParams {
    count: u32,
    cap: u32,
    pad0: u32,
    pad1: u32,
    rel_slack: f32,
    abs_slack: f32,
    ang_slack: f32,
    pad2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GEnt {
    x: f32,
    y: f32,
    z: f32,
    range: f32,
    heading: f32,
    half_fov: f32,
    layer: u32,
    sees: u32,
}

const WG: u32 = 64;
const MAX_GROUPS: u32 = 65_535;

/// A wgpu device with the compiled kernels.
pub struct GpuKernels {
    adapter_name: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    los: wgpu::ComputePipeline,
    viewshed: wgpu::ComputePipeline,
    pairs: wgpu::ComputePipeline,
}

impl GpuKernels {
    /// Open the first adapter and compile the shaders.
    pub fn new() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::util::backend_bits_from_env().unwrap_or(wgpu::Backends::PRIMARY),
            ..Default::default()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        }))
        .ok_or(GpuError::NoAdapter)?;
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("bgws-gpu"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
            },
            None,
        ))
        .map_err(|e| GpuError::Device(e.to_string()))?;
        let make = |label: &str, src: &str| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: "main",
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let los = make("los", &los_source());
        let viewshed = make("viewshed", &viewshed_source());
        let pairs = make("pairs", PAIRS_WGSL);
        Ok(Self { adapter_name: adapter.get_info().name, device, queue, los, viewshed, pairs })
    }

    fn storage<T: Pod>(&self, data: &[T], extra: wgpu::BufferUsages) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(data),
            usage: wgpu::BufferUsages::STORAGE | extra,
        })
    }

    fn uniform<T: Pod>(&self, v: &T) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::bytes_of(v),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    /// Bind `buffers` to group 0 in order, dispatch, and read `readback` back.
    fn run(&self, pipe: &wgpu::ComputePipeline, buffers: &[&wgpu::Buffer], groups: (u32, u32), readback: &[&wgpu::Buffer]) -> Result<Vec<Vec<u8>>, GpuError> {
        let layout = pipe.get_bind_group_layout(0);
        let entries: Vec<wgpu::BindGroupEntry> = buffers
            .iter()
            .enumerate()
            .map(|(i, b)| wgpu::BindGroupEntry { binding: i as u32, resource: b.as_entire_binding() })
            .collect();
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &layout, entries: &entries });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: None, timestamp_writes: None });
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(groups.0, groups.1, 1);
        }
        let staging: Vec<wgpu::Buffer> = readback
            .iter()
            .map(|b| {
                let s = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: b.size(),
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                enc.copy_buffer_to_buffer(b, 0, &s, 0, b.size());
                s
            })
            .collect();
        self.queue.submit(Some(enc.finish()));
        let mut out = Vec::with_capacity(staging.len());
        for s in &staging {
            let slice = s.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            self.device.poll(wgpu::Maintain::Wait);
            rx.recv()
                .map_err(|e| GpuError::Readback(e.to_string()))?
                .map_err(|e| GpuError::Readback(e.to_string()))?;
            out.push(slice.get_mapped_range().to_vec());
            s.unmap();
        }
        Ok(out)
    }

    fn grid_params(grid: &HeightGrid, count: u32, count2: u32, p: &LosParams, extra: f32) -> GridParams {
        GridParams {
            width: grid.width as u32,
            height: grid.height as u32,
            count,
            count2,
            cell: grid.cell as f32,
            inv_2kr: (1.0 / (2.0 * p.k_factor * EARTH_RADIUS)) as f32,
            step_cells: p.step_cells as f32,
            extra,
        }
    }

    /// Raw GPU clearances (metres, f32) for each query.
    pub fn los_raw(&self, grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Result<Vec<f32>, GpuError> {
        if queries.is_empty() {
            return Ok(Vec::new());
        }
        let n = queries.len() as u32;
        if n.div_ceil(WG) > MAX_GROUPS {
            return Err(GpuError::TooLarge);
        }
        let (ox, oy) = (grid.origin_x, grid.origin_y);
        let qs: Vec<GQuery> = queries
            .iter()
            .map(|q| GQuery {
                v: [
                    (q.obs_x - ox) as f32,
                    (q.obs_y - oy) as f32,
                    q.obs_agl as f32,
                    (q.tgt_x - ox) as f32,
                    (q.tgt_y - oy) as f32,
                    q.tgt_agl as f32,
                    0.0,
                    0.0,
                ],
            })
            .collect();
        let params = self.uniform(&Self::grid_params(grid, n, 0, p, 0.0));
        let h = self.storage(&grid.heights, wgpu::BufferUsages::empty());
        let qb = self.storage(&qs, wgpu::BufferUsages::empty());
        let out = self.storage(&vec![0f32; queries.len()], wgpu::BufferUsages::COPY_SRC);
        let bytes = self.run(&self.los, &[&params, &h, &qb, &out], (n.div_ceil(WG), 1), &[&out])?;
        Ok(bytemuck::cast_slice::<u8, f32>(&bytes[0]).to_vec())
    }

    fn try_los(&self, grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Result<Vec<LosResult>, GpuError> {
        let raw = self.los_raw(grid, queries, p)?;
        Ok(raw
            .iter()
            .zip(queries)
            .map(|(&c, q)| {
                if (c as f64).abs() <= p.refine_margin_m || !c.is_finite() {
                    los::los_one(grid, q, p)
                } else {
                    los::quantise(c as f64, p.quantum_m)
                }
            })
            .collect())
    }

    fn try_viewshed(&self, grid: &HeightGrid, observers: &[ViewshedObserver], target_agl: f64, p: &LosParams) -> Result<Viewshed, GpuError> {
        let nodes = (grid.width * grid.height) as u32;
        let n_obs = observers.len() as u32;
        if observers.is_empty() {
            return Ok(Viewshed { width: grid.width, height: grid.height, count: vec![0; nodes as usize] });
        }
        if nodes.div_ceil(WG) > MAX_GROUPS || n_obs > MAX_GROUPS {
            return Err(GpuError::TooLarge);
        }
        let obs: Vec<GObs> = observers
            .iter()
            .map(|o| GObs { v: [(o.x - grid.origin_x) as f32, (o.y - grid.origin_y) as f32, o.agl as f32, o.max_range.min(f32::MAX as f64) as f32] })
            .collect();
        let params = self.uniform(&Self::grid_params(grid, n_obs, 0, p, target_agl as f32));
        let h = self.storage(&grid.heights, wgpu::BufferUsages::empty());
        let ob = self.storage(&obs, wgpu::BufferUsages::empty());
        let out = self.storage(&vec![0u32; nodes as usize], wgpu::BufferUsages::COPY_SRC);
        let bytes = self.run(&self.viewshed, &[&params, &h, &ob, &out], (nodes.div_ceil(WG), n_obs), &[&out])?;
        Ok(Viewshed { width: grid.width, height: grid.height, count: bytemuck::cast_slice::<u8, u32>(&bytes[0]).to_vec() })
    }

    fn try_pairs(&self, entities: &[PairEntity], p: &PairParams) -> Result<Vec<CandidatePair>, GpuError> {
        if entities.is_empty() {
            return Ok(Vec::new());
        }
        let n = entities.len() as u32;
        if n.div_ceil(WG) > MAX_GROUPS {
            return Err(GpuError::TooLarge);
        }
        // Centre coordinates so f32 keeps centimetres over a theatre.
        let cx = entities.iter().map(|e| e.x).sum::<f64>() / n as f64;
        let cy = entities.iter().map(|e| e.y).sum::<f64>() / n as f64;
        let ents: Vec<GEnt> = entities
            .iter()
            .map(|e| GEnt {
                x: (e.x - cx) as f32,
                y: (e.y - cy) as f32,
                z: e.z as f32,
                range: e.sensor_range.min(f32::MAX as f64) as f32,
                heading: e.heading as f32,
                half_fov: e.half_fov.min(10.0) as f32,
                layer: e.layer,
                sees: e.sees_layers,
            })
            .collect();
        let eb = self.storage(&ents, wgpu::BufferUsages::empty());
        let mut cap = (n * 16).max(1024);
        loop {
            let pp = GPairParams {
                count: n,
                cap,
                pad0: 0,
                pad1: 0,
                rel_slack: p.gpu_slack as f32,
                abs_slack: 1.0,
                ang_slack: p.gpu_slack as f32,
                pad2: 0.0,
            };
            let params = self.uniform(&pp);
            let counter = self.storage(&[0u32], wgpu::BufferUsages::COPY_SRC);
            let out = self.storage(&vec![[0u32; 2]; cap as usize], wgpu::BufferUsages::COPY_SRC);
            let bytes = self.run(&self.pairs, &[&params, &eb, &counter, &out], (n.div_ceil(WG), 1), &[&counter, &out])?;
            let found = bytemuck::cast_slice::<u8, u32>(&bytes[0])[0];
            if found > cap {
                cap = found;
                continue;
            }
            let pairs: Vec<(u32, u32)> = bytemuck::cast_slice::<u8, [u32; 2]>(&bytes[1])[..found as usize].iter().map(|v| (v[0], v[1])).collect();
            return Ok(pairs::refine_pairs(entities, pairs));
        }
    }
}

impl Kernels for GpuKernels {
    fn name(&self) -> String {
        format!("gpu:{}", self.adapter_name)
    }
    fn is_gpu(&self) -> bool {
        true
    }
    fn los_batch(&self, grid: &HeightGrid, queries: &[LosQuery], p: &LosParams) -> Vec<LosResult> {
        self.try_los(grid, queries, p).unwrap_or_else(|_| los::los_batch_cpu(grid, queries, p))
    }
    fn viewshed(&self, grid: &HeightGrid, observers: &[ViewshedObserver], target_agl: f64, p: &LosParams) -> Viewshed {
        self.try_viewshed(grid, observers, target_agl, p).unwrap_or_else(|_| viewshed::viewshed_cpu(grid, observers, target_agl, p))
    }
    fn candidate_pairs(&self, entities: &[PairEntity], p: &PairParams) -> Vec<CandidatePair> {
        self.try_pairs(entities, p).unwrap_or_else(|_| pairs::candidate_pairs_cpu(entities, p))
    }
}
