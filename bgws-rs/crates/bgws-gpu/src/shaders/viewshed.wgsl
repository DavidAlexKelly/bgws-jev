// Viewshed (§11.1). x: node index, y: observer index. Each visible
// (node, observer) adds one to the node's count; the sum is order-free.
// P.extra is the target height above ground.

struct Obs {
    x: f32, y: f32, agl: f32, max_range: f32,
};

@group(0) @binding(2) var<storage, read> O: array<Obs>;
@group(0) @binding(3) var<storage, read_write> COUNT: array<atomic<u32>>;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let k = gid.x;
    let oi = gid.y;
    if (k >= P.width * P.height || oi >= P.count) {
        return;
    }
    let o = O[oi];
    let x = f32(k % P.width) * P.cell;
    let y = f32(k / P.width) * P.cell;
    let dx = x - o.x;
    let dy = y - o.y;
    if (sqrt(dx * dx + dy * dy) > o.max_range) {
        return;
    }
    if (clearance(o.x, o.y, o.agl, x, y, P.extra) >= -0.005) {
        atomicAdd(&COUNT[k], 1u);
    }
}
