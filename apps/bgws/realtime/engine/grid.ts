// ── grid.ts ──────────────────────────────────────────────────────────────
// Who is near whom, without asking every pair.
//
// Detection asked every observer about every enemy each step, only to throw
// away every pair beyond the 3 km sight cap. Close contact asked every
// moving unit about every enemy, for a 150–300 m halt. At 600 units that is
// ~180,000 distance checks a second for the first alone. A grid of cells
// one search-radius wide answers "who could be within r of here" from the
// 3×3 cells around a point.
//
// IT ONLY NARROWS, NEVER DECIDES. Callers still measure the real distance
// (`distanceM`) and keep every check they had; the grid just leaves out
// pairs that cannot pass. To be safe it errs wide:
//   - positions are projected to flat metres with the cosine of the latitude
//     FARTHEST from the equator among the points, which shrinks east–west
//     distances, never stretches them;
//   - the search radius is padded by MARGIN.
// And it returns indices in ascending order, so a caller that walks them
// walks in its own original order — the dice are drawn in the same order as
// before (decisions.golden.test.ts).

import type { LatLng } from "../../lib/board";

/** Metres per degree of latitude: a little under the haversine's, so projected distances run short. */
const M_PER_DEG = 111_000;
/** Padding on the search radius, for the flat projection's error over a board's width. */
const MARGIN = 1.05;

export class PositionGrid {
  private readonly cells = new Map<string, number[]>();
  private readonly cosLat: number;
  private readonly cellM: number;

  /**
   * @param positions points to index, by position in this array
   * @param radiusM   the largest radius `near` will be asked for
   */
  constructor(
    positions: readonly LatLng[],
    radiusM: number,
  ) {
    let farthest = 0;
    for (const p of positions) farthest = Math.max(farthest, Math.abs(p.lat));
    this.cosLat = Math.cos((Math.min(farthest, 89) * Math.PI) / 180);
    this.cellM = radiusM * MARGIN;
    positions.forEach((p, index) => {
      const key = this.key(this.cx(p), this.cy(p));
      const cell = this.cells.get(key);
      if (cell) cell.push(index);
      else this.cells.set(key, [index]);
    });
  }

  private cx(p: LatLng): number {
    return Math.floor((p.lng * M_PER_DEG * this.cosLat) / this.cellM);
  }

  private cy(p: LatLng): number {
    return Math.floor((p.lat * M_PER_DEG) / this.cellM);
  }

  private key(x: number, y: number): string {
    return `${x},${y}`;
  }

  /**
   * Indices of every point that could be within the grid's radius of `at`
   * (and some that are not), in ascending order.
   */
  near(at: LatLng): number[] {
    const x = this.cx(at);
    const y = this.cy(at);
    const out: number[] = [];
    for (let dx = -1; dx <= 1; dx++) {
      for (let dy = -1; dy <= 1; dy++) {
        const cell = this.cells.get(this.key(x + dx, y + dy));
        if (cell) for (const index of cell) out.push(index);
      }
    }
    return out.sort((a, b) => a - b);
  }
}
