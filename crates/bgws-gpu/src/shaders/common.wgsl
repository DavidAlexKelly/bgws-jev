// Shared terrain sampling (§11.3). Must match grid.rs: node (i, j) at
// (i·cell, j·cell) in grid-local metres, bilinear between nodes, clamped.

struct Params {
    width: u32,
    height: u32,
    count: u32,
    count2: u32,
    cell: f32,
    inv_2kr: f32,
    step_cells: f32,
    extra: f32,
};

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> H: array<f32>;

fn node(i: i32, j: i32) -> f32 {
    let ii = clamp(i, 0, i32(P.width) - 1);
    let jj = clamp(j, 0, i32(P.height) - 1);
    return H[u32(jj) * P.width + u32(ii)];
}

fn height_at(x: f32, y: f32) -> f32 {
    let gx = clamp(x / P.cell, 0.0, f32(P.width - 1u));
    let gy = clamp(y / P.cell, 0.0, f32(P.height - 1u));
    let i0 = i32(floor(gx));
    let j0 = i32(floor(gy));
    let fx = gx - f32(i0);
    let fy = gy - f32(j0);
    let a = mix(node(i0, j0), node(i0 + 1, j0), fx);
    let b = mix(node(i0, j0 + 1), node(i0 + 1, j0 + 1), fx);
    return mix(a, b, fy);
}

// Smallest clearance of the chord from (ox, oy, oagl) to (tx, ty, tagl)
// over the curvature-raised terrain, metres. Mirrors los.rs.
fn clearance(ox: f32, oy: f32, oagl: f32, tx: f32, ty: f32, tagl: f32) -> f32 {
    let dx = tx - ox;
    let dy = ty - oy;
    let d = sqrt(dx * dx + dy * dy);
    let zo = height_at(ox, oy) + oagl;
    let zt = height_at(tx, ty) + tagl;
    let step = max(P.cell * P.step_cells, 1e-6);
    let n = max(u32(ceil(d / step)), 1u);
    if (n < 2u) {
        return min(oagl, tagl);
    }
    var m = 3.0e38;
    for (var s = 1u; s < n; s = s + 1u) {
        let t = f32(s) / f32(n);
        let ground = height_at(ox + t * dx, oy + t * dy);
        let bulge = t * (1.0 - t) * d * d * P.inv_2kr;
        m = min(m, zo + t * (zt - zo) - (ground + bulge));
    }
    return m;
}
