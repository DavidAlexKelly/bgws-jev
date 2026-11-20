// A building's ghost: where the satellite photographed it on the ground.
//
// Satellite imagery is draped flat on the ground, and it holds every
// building as photographed: its roof shifted by its height times the lean,
// and between that and its footprint the wall that faced the satellite. With
// the building standing up in 3D, that flat copy lies on the ground beside
// it. Its outline is the footprint swept along the shift: the footprint, the
// shifted footprint and every edge's sweep between them, as one shape (so a
// tint laid over it darkens it once).

import polygonClipping from "polygon-clipping";

type Pair = [number, number];

/** The footprint `ring` (no closing point) swept along `d`: a multipolygon,
 * rings closed. Empty when there is no shift. */
export function ghostOutline(ring: Pair[], d: Pair): Pair[][][] {
  if (ring.length < 3 || (d[0] === 0 && d[1] === 0)) return [];
  const close = (r: Pair[]): Pair[] => [...r, r[0]];
  const moved = ring.map((p) => [p[0] + d[0], p[1] + d[1]] as Pair);
  const parts: Pair[][][] = [[close(ring)], [close(moved)]];
  for (let i = 0; i < ring.length; i++) {
    const a = ring[i];
    const b = ring[(i + 1) % ring.length];
    parts.push([close([a, b, moved[(i + 1) % ring.length], moved[i]])]);
  }
  return polygonClipping.union(parts[0], ...parts.slice(1)) as Pair[][][];
}

/** A multipolygon's area (outer rings less holes), in its own units squared. */
export function area(mp: Pair[][][]): number {
  const ringArea = (r: Pair[]) => {
    let s = 0;
    for (let i = 0; i < r.length - 1; i++) s += r[i][0] * r[i + 1][1] - r[i + 1][0] * r[i][1];
    return Math.abs(s) / 2;
  };
  return mp.reduce((t, poly) => t + ringArea(poly[0]) - poly.slice(1).reduce((h, r) => h + ringArea(r), 0), 0);
}
