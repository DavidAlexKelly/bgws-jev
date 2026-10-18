/**
 * Ramer-Douglas-Peucker polyline simplification.
 *
 * Reduces route waypoints from hundreds of graph nodes down to ~5-10
 * spine points suitable for MilX tactical arrow rendering.
 */

/* ------------------------------------------------------------------ */
/*  Perpendicular distance helper                                     */
/* ------------------------------------------------------------------ */

/**
 * Compute the perpendicular distance from `point` to the line segment
 * defined by `lineStart` → `lineEnd` using the cross-product formula.
 */
function perpendicularDistance(
  point: [number, number],
  lineStart: [number, number],
  lineEnd: [number, number],
): number {
  const [px, py] = point;
  const [ax, ay] = lineStart;
  const [bx, by] = lineEnd;

  const dx = bx - ax;
  const dy = by - ay;

  const lenSq = dx * dx + dy * dy;

  // Degenerate case: start and end are the same point
  if (lenSq === 0) {
    const ex = px - ax;
    const ey = py - ay;
    return Math.sqrt(ex * ex + ey * ey);
  }

  // |cross product| / |line length|
  const cross = Math.abs((dy * (px - ax)) - (dx * (py - ay)));
  return cross / Math.sqrt(lenSq);
}

/* ------------------------------------------------------------------ */
/*  Ramer-Douglas-Peucker simplification                              */
/* ------------------------------------------------------------------ */

/**
 * Simplify a polyline using the Ramer-Douglas-Peucker algorithm.
 *
 * @param points   Array of [lon, lat] (or [x, y]) coordinates.
 * @param epsilon  Distance threshold – points closer than this to the
 *                 simplified line are discarded.  Default 0.0005 (~55 m
 *                 at the equator for lon/lat coordinates).
 * @returns        Simplified array of [lon, lat] coordinates.
 */
export function rdpSimplify(
  points: [number, number][],
  epsilon: number = 0.0005,
): [number, number][] {
  if (points.length <= 2) return points;

  // Find the point with the maximum perpendicular distance from the
  // start→end line segment.
  let maxDist = 0;
  let maxIdx = 0;

  const first = points[0];
  const last = points[points.length - 1];

  for (let i = 1; i < points.length - 1; i++) {
    const dist = perpendicularDistance(points[i], first, last);
    if (dist > maxDist) {
      maxDist = dist;
      maxIdx = i;
    }
  }

  // If the furthest point exceeds the threshold, recursively simplify
  // the two halves and merge (avoiding duplicate at the split point).
  if (maxDist > epsilon) {
    const left = rdpSimplify(points.slice(0, maxIdx + 1), epsilon);
    const right = rdpSimplify(points.slice(maxIdx), epsilon);
    return [...left.slice(0, -1), ...right];
  }

  // All intermediate points are within tolerance – drop them.
  return [first, last];
}
