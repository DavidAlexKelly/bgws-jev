// Candidate pairs (§11.1): one invocation per observer, brute force over
// targets with slack on range and angle. The host refines in f64 and sorts,
// so the order atomics impose here never reaches a decision.

struct PParams {
    count: u32,
    cap: u32,
    pad0: u32,
    pad1: u32,
    rel_slack: f32,
    abs_slack: f32,
    ang_slack: f32,
    pad2: f32,
};

struct Ent {
    x: f32, y: f32, z: f32, range: f32,
    heading: f32, half_fov: f32, layer: u32, sees: u32,
};

@group(0) @binding(0) var<uniform> PP: PParams;
@group(0) @binding(1) var<storage, read> E: array<Ent>;
@group(0) @binding(2) var<storage, read_write> N: atomic<u32>;
@group(0) @binding(3) var<storage, read_write> OUT: array<vec2<u32>>;

const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= PP.count) {
        return;
    }
    let o = E[i];
    if (o.sees == 0u) {
        return;
    }
    let lim = o.range * (1.0 + PP.rel_slack) + PP.abs_slack;
    for (var j = 0u; j < PP.count; j = j + 1u) {
        if (j == i) {
            continue;
        }
        let t = E[j];
        if (((o.sees >> (t.layer & 31u)) & 1u) == 0u) {
            continue;
        }
        let dx = t.x - o.x;
        let dy = t.y - o.y;
        let dz = t.z - o.z;
        if (dx * dx + dy * dy + dz * dz > lim * lim) {
            continue;
        }
        if (o.half_fov < PI && (dx != 0.0 || dy != 0.0)) {
            let a = atan2(dx, dy) - o.heading;
            let w = a - TAU * floor((a + PI) / TAU);
            if (abs(w) > o.half_fov + PP.ang_slack) {
                continue;
            }
        }
        let k = atomicAdd(&N, 1u);
        if (k < PP.cap) {
            OUT[k] = vec2<u32>(i, j);
        }
    }
}
