// Batched line of sight (§11.1). One invocation per query; writes the raw
// clearance in metres. The host quantises and re-checks near-zero cases in f64.

struct Query {
    ox: f32, oy: f32, oagl: f32,
    tx: f32, ty: f32, tagl: f32,
    pad0: f32, pad1: f32,
};

@group(0) @binding(2) var<storage, read> Q: array<Query>;
@group(0) @binding(3) var<storage, read_write> OUT: array<f32>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    if (k >= P.count) {
        return;
    }
    let q = Q[k];
    OUT[k] = clearance(q.ox, q.oy, q.oagl, q.tx, q.ty, q.tagl);
}
