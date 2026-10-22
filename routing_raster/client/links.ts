// ── routing_raster/client/links.ts ─────────────────────────────────────────
// Reading links.bin (format 2): river walls and road links between pixels.
//
// The builder (myproject/terrain/links.py) stores each move between two
// neighbouring pixels once, on the pixel it leaves going east or north. This
// module turns those bits back into the three answers a pathfinder needs:
// may I step there off-road, may I step there by road, and is this pixel on
// a road at all.
//
// COORDINATES: x east, y NORTH, exactly as the files are laid out (rows from
// the south edge). `LinksAt` hides chunks: give it global pixel coordinates
// and it answers from whichever chunk holds them, or 0 where none is loaded.
// Moves across a chunk seam then work like any other move.
//
// WHAT THIS DOES NOT DECIDE: whether the ground itself can be entered. An
// off-road move into water, nodata or a building is still the terrain
// plane's call, as before. A road move ignores the terrain plane entirely:
// that is what lets a bridge cross water and a road pass a DEM gap.

export const WALL_E = 0x01;
export const WALL_N = 0x02;
export const ROAD_E = 0x04;
export const ROAD_N = 0x08;
export const ROAD_NE = 0x10;
export const ROAD_NW = 0x20;
export const ROAD_BITS = ROAD_E | ROAD_N | ROAD_NE | ROAD_NW;

/** The links byte of the pixel at (x, y), y counting north; 0 where unknown. */
export type LinksAt = (x: number, y: number) => number;

/** The eight moves, east first, anticlockwise. */
export const DIRECTIONS: readonly (readonly [number, number])[] = [
  [1, 0],
  [1, 1],
  [0, 1],
  [-1, 1],
  [-1, 0],
  [-1, -1],
  [0, -1],
  [1, -1],
];

/** A wall on the orthogonal move from (x, y) by (dx, dy)? */
export function wall(at: LinksAt, x: number, y: number, dx: number, dy: number): boolean {
  if (dx === 1) return (at(x, y) & WALL_E) !== 0;
  if (dx === -1) return (at(x - 1, y) & WALL_E) !== 0;
  if (dy === 1) return (at(x, y) & WALL_N) !== 0;
  return (at(x, y - 1) & WALL_N) !== 0;
}

/**
 * Is the off-road move from (x, y) by (dx, dy) across a river?
 *
 * A diagonal is blocked only when both right-angle routes round its corner
 * are walled: exactly when the walls separate the two pixels, so a river
 * one pixel wide cannot be slipped through at a corner.
 */
export function offRoadBlocked(at: LinksAt, x: number, y: number, dx: number, dy: number): boolean {
  if (dx === 0 || dy === 0) return wall(at, x, y, dx, dy);
  const viaX = wall(at, x, y, dx, 0) || wall(at, x + dx, y, 0, dy);
  const viaY = wall(at, x, y, 0, dy) || wall(at, x, y + dy, dx, 0);
  return viaX && viaY;
}

/** Is there a road link on the move from (x, y) by (dx, dy)? */
export function roadLinked(at: LinksAt, x: number, y: number, dx: number, dy: number): boolean {
  if (dy === 0) return dx === 1 ? (at(x, y) & ROAD_E) !== 0 : (at(x - 1, y) & ROAD_E) !== 0;
  if (dx === 0) return dy === 1 ? (at(x, y) & ROAD_N) !== 0 : (at(x, y - 1) & ROAD_N) !== 0;
  if (dx === dy) return dx === 1 ? (at(x, y) & ROAD_NE) !== 0 : (at(x - 1, y - 1) & ROAD_NE) !== 0;
  return dy === 1 ? (at(x, y) & ROAD_NW) !== 0 : (at(x + 1, y - 1) & ROAD_NW) !== 0;
}

/** Does any road link touch this pixel? */
export function onRoad(at: LinksAt, x: number, y: number): boolean {
  return DIRECTIONS.some(([dx, dy]) => roadLinked(at, x, y, dx, dy));
}

export type MoveKind = "road" | "offRoad" | "blocked";

/**
 * How the move from (x, y) by (dx, dy) can be made, as far as rivers and
 * roads are concerned: along a road link, off-road, or not at all. The
 * caller still applies the terrain plane to an "offRoad" move.
 */
export function moveKind(at: LinksAt, x: number, y: number, dx: number, dy: number): MoveKind {
  if (roadLinked(at, x, y, dx, dy)) return "road";
  return offRoadBlocked(at, x, y, dx, dy) ? "blocked" : "offRoad";
}

/** A LinksAt over one chunk's links.bin (south-first rows); 0 outside it. */
export function chunkLinks(bytes: Uint8Array, width: number, height: number): LinksAt {
  if (bytes.length !== width * height) {
    throw new Error(`links.bin has ${bytes.length} bytes, expected ${width} x ${height}`);
  }
  return (x, y) => (x >= 0 && x < width && y >= 0 && y < height ? bytes[y * width + x] : 0);
}
