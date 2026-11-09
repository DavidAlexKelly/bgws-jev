//! Drawing vector shapes onto the engine's grid: polygons by even-odd
//! scanline at cell centres, and line clipping to a box.

/// Call `paint(i, j)` for every cell whose centre is inside the polygon
/// (rings in grid units: x = column, y = row, cell centres at integers).
/// Even-odd over all rings, so holes and multipolygons need no sorting.
pub fn fill_polygon(rings: &[Vec<[f64; 2]>], width: usize, height: usize, mut paint: impl FnMut(usize, usize)) {
    let (mut y0, mut y1) = (f64::MAX, f64::MIN);
    for r in rings {
        for p in r {
            y0 = y0.min(p[1]);
            y1 = y1.max(p[1]);
        }
    }
    if y1 < 0.0 || y0 > (height - 1) as f64 {
        return;
    }
    let j0 = y0.ceil().max(0.0) as usize;
    let j1 = (y1.floor() as i64).min(height as i64 - 1);
    if j1 < 0 {
        return;
    }
    let mut xs: Vec<f64> = Vec::new();
    for j in j0..=j1 as usize {
        let y = j as f64;
        xs.clear();
        for r in rings {
            for w in r.windows(2) {
                let (a, b) = (w[0], w[1]);
                // Half-open on y, so a vertex on the scanline counts once.
                if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                    xs.push(a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]));
                }
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        for pair in xs.as_chunks::<2>().0 {
            let i0 = pair[0].ceil().max(0.0) as i64;
            let i1 = (pair[1].floor() as i64).min(width as i64 - 1);
            for i in i0..=i1 {
                paint(i as usize, j);
            }
        }
    }
}

/// Signed area of a ring (shoelace).
pub fn ring_area(r: &[[f64; 2]]) -> f64 {
    r.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f64>() / 2.0
}

/// A line clipped to the box [x0, x1]×[y0, y1], as the pieces inside it.
pub fn clip_line(line: &[[f64; 2]], x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Vec<[f64; 2]>> {
    let inside = |p: [f64; 2]| p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1;
    let mut out: Vec<Vec<[f64; 2]>> = Vec::new();
    let mut cur: Vec<[f64; 2]> = Vec::new();
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        // Liang–Barsky on the segment.
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        let mut keep = true;
        for (p, q) in [(-dx, a[0] - x0), (dx, x1 - a[0]), (-dy, a[1] - y0), (dy, y1 - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    keep = false;
                    break;
                }
            } else {
                let t = q / p;
                if p < 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if !keep || t0 > t1 {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            continue;
        }
        let pa = [a[0] + t0 * dx, a[1] + t0 * dy];
        let pb = [a[0] + t1 * dx, a[1] + t1 * dy];
        if cur.last().is_none_or(|l| *l != pa) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
            cur.push(pa);
        }
        cur.push(pb);
        if t1 < 1.0 || !inside(b) {
            if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            }
            cur.clear();
        }
    }
    if cur.len() >= 2 {
        out.push(cur);
    }
    out.retain(|l| l.windows(2).any(|w| w[0] != w[1]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_a_square_with_a_hole() {
        let outer = vec![[0.5, 0.5], [6.5, 0.5], [6.5, 6.5], [0.5, 6.5], [0.5, 0.5]];
        let hole = vec![[2.5, 2.5], [4.5, 2.5], [4.5, 4.5], [2.5, 4.5], [2.5, 2.5]];
        let mut n = 0;
        let mut in_hole = false;
        fill_polygon(&[outer, hole], 10, 10, |i, j| {
            n += 1;
            in_hole |= (3..=4).contains(&i) && (3..=4).contains(&j);
        });
        assert_eq!(n, 36 - 4);
        assert!(!in_hole);
        assert_eq!(ring_area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 2.0], [0.0, 2.0], [0.0, 0.0]]), 4.0);
    }

    #[test]
    fn clips_a_line_into_pieces() {
        // In, out across the top, back in.
        let l = vec![[1.0, 1.0], [5.0, 1.0], [5.0, 20.0], [8.0, 20.0], [8.0, 2.0]];
        let parts = clip_line(&l, 0.0, 0.0, 10.0, 10.0);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], vec![[1.0, 1.0], [5.0, 1.0], [5.0, 10.0]]);
        assert_eq!(parts[1], vec![[8.0, 10.0], [8.0, 2.0]]);
        assert!(clip_line(&[[20.0, 20.0], [30.0, 30.0]], 0.0, 0.0, 10.0, 10.0).is_empty());
    }
}
