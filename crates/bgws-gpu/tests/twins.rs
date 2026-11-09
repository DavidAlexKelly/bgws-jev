//! GPU/CPU twin tests (§11.3, §17): random inputs through both backends.
//! Without an adapter (CI, this container) each test passes with a message.
//! The WGSL is always parsed and validated with naga, so a shader error is
//! caught even where no GPU exists.
#![cfg(feature = "gpu")]

use bgws_gpu::gpu::{self, GpuKernels};
use bgws_gpu::*;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

fn validate(src: &str) {
    use wgpu::naga;
    let module = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
        .validate(&module)
        .unwrap_or_else(|e| panic!("{e:?}"));
}

#[test]
fn shaders_validate() {
    validate(&gpu::los_source());
    validate(&gpu::viewshed_source());
    validate(gpu::PAIRS_WGSL);
}

fn device() -> Option<GpuKernels> {
    match GpuKernels::new() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping GPU twin test: {e}");
            None
        }
    }
}

fn random_grid(rng: &mut ChaCha8Rng) -> HeightGrid {
    let (w, h) = (64, 64);
    let mut hs = vec![0f32; w * h];
    for (k, v) in hs.iter_mut().enumerate() {
        let (i, j) = ((k % w) as f32, (k / w) as f32);
        *v = 100.0 + 40.0 * (i * 0.21).sin() * (j * 0.17).cos() + rng.gen_range(0.0..5.0);
    }
    HeightGrid::new(w, h, 30.0, 1000.0, -500.0, hs).unwrap()
}

#[test]
fn los_twin() {
    let Some(g) = device() else { return };
    let mut rng = ChaCha8Rng::seed_from_u64(1);
    let grid = random_grid(&mut rng);
    let (x0, y0, x1, y1) = grid.bounds();
    let qs: Vec<LosQuery> = (0..2000)
        .map(|_| LosQuery {
            obs_x: rng.gen_range(x0..x1),
            obs_y: rng.gen_range(y0..y1),
            obs_agl: rng.gen_range(1.0..30.0),
            tgt_x: rng.gen_range(x0..x1),
            tgt_y: rng.gen_range(y0..y1),
            tgt_agl: rng.gen_range(1.0..30.0),
        })
        .collect();
    let p = LosParams::default();
    let cpu = CpuKernels.los_batch(&grid, &qs, &p);
    let gpu = g.los_batch(&grid, &qs, &p);
    for (a, b) in cpu.iter().zip(&gpu) {
        assert_eq!(a.visible, b.visible);
        assert!((a.clearance_cm - b.clearance_cm).abs() <= 5, "{a:?} {b:?}");
    }
}

#[test]
fn viewshed_twin() {
    let Some(g) = device() else { return };
    let mut rng = ChaCha8Rng::seed_from_u64(2);
    let grid = random_grid(&mut rng);
    let obs: Vec<ViewshedObserver> = (0..4)
        .map(|_| ViewshedObserver { x: rng.gen_range(1000.0..2800.0), y: rng.gen_range(-500.0..1300.0), agl: 5.0, max_range: 1500.0 })
        .collect();
    let p = LosParams::default();
    let a = CpuKernels.viewshed(&grid, &obs, 2.0, &p);
    let b = g.viewshed(&grid, &obs, 2.0, &p);
    let diff = a.count.iter().zip(&b.count).filter(|(x, y)| x != y).count();
    assert!((diff as f64) < 0.01 * a.count.len() as f64, "{diff} nodes differ");
}

#[test]
fn pairs_twin() {
    let Some(g) = device() else { return };
    let mut rng = ChaCha8Rng::seed_from_u64(3);
    let es: Vec<PairEntity> = (0..3000)
        .map(|_| PairEntity {
            x: rng.gen_range(-50_000.0..50_000.0),
            y: rng.gen_range(-50_000.0..50_000.0),
            z: rng.gen_range(0.0..3000.0),
            layer: rng.gen_range(0..3),
            sees_layers: rng.gen_range(0..8),
            sensor_range: rng.gen_range(0.0..8000.0),
            heading: rng.gen_range(0.0..std::f64::consts::TAU),
            half_fov: rng.gen_range(0.2..4.0),
        })
        .collect();
    let p = PairParams::default();
    assert_eq!(CpuKernels.candidate_pairs(&es, &p), g.candidate_pairs(&es, &p));
}
