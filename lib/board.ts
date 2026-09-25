// ── bgws/lib/board.ts ──────────────────────────────────────────────────────
// The sheet BGWS is played on.
//
// A game is fought on a 10 x 10 km piece of the real world divided into
// 1,000 m grid squares. Both numbers are load-bearing rather than cosmetic:
// the grid square is the unit of movement allowance, and the edge of the sheet
// is a real boundary — a Force Element cannot be ordered off it, and an
// objective outside it is not an objective.
//
// The 1 km graticule itself is not drawn here. MGRS grid lines at z12+ are
// already 1 km squares (shared/components/MgrsOverlay), they are the squares a
// real map sheet prints, and their labels are grid references a player can
// read out. Drawing a second, arbitrary 1 km grid over the top of the correct
// one would be worse than drawing none.
//
// Pure: no map, no React. Everything here is arithmetic on lon/lat.

/** Metres per degree of latitude. Constant enough at this scale. */
const M_PER_DEG_LAT = 110_574;

/** Metres per degree of longitude at the equator; shrinks with cos(lat). */
const M_PER_DEG_LON_EQUATOR = 111_320;

/** BGWS plays on a 10 x 10 km sheet. */
export const BOARD_SIZE_M = 10_000;

/** Grid squares are 1,000 m, and so is the movement allowance's unit. */
export const GRID_SQUARE_M = 1_000;

/**
 * Line of sight is capped at 3 km, which is also the longest any weapon may
 * fire (BGWS 2.1.14). Kept here rather than in the rules engine because the
 * board is where it is enforced visually.
 */
export const LOS_CAP_M = 3_000;

export interface LatLng {
  lat: number;
  lng: number;
}

export interface BoardBounds {
  west: number;
  south: number;
  east: number;
  north: number;
}

export function metresPerDegreeLon(lat: number): number {
  return M_PER_DEG_LON_EQUATOR * Math.cos((lat * Math.PI) / 180);
}

/** The sheet centred on a point, as a bounding box. */
export function boardBounds(centre: LatLng, sizeM: number = BOARD_SIZE_M): BoardBounds {
  const halfLat = sizeM / 2 / M_PER_DEG_LAT;
  const halfLon = sizeM / 2 / metresPerDegreeLon(centre.lat);
  return {
    west: centre.lng - halfLon,
    south: centre.lat - halfLat,
    east: centre.lng + halfLon,
    north: centre.lat + halfLat,
  };
}

/** Closed ring, for a GeoJSON polygon or line. */
export function boardRing(bounds: BoardBounds): [number, number][] {
  return [
    [bounds.west, bounds.south],
    [bounds.east, bounds.south],
    [bounds.east, bounds.north],
    [bounds.west, bounds.north],
    [bounds.west, bounds.south],
  ];
}

export function isOnBoard(point: LatLng, bounds: BoardBounds): boolean {
  return (
    point.lng >= bounds.west &&
    point.lng <= bounds.east &&
    point.lat >= bounds.south &&
    point.lat <= bounds.north
  );
}

const EARTH_RADIUS_M = 6_371_000;

/** Great-circle distance. Every BGWS range test goes through this. */
export function distanceM(from: LatLng, to: LatLng): number {
  const dLat = ((to.lat - from.lat) * Math.PI) / 180;
  const dLng = ((to.lng - from.lng) * Math.PI) / 180;
  const lat1 = (from.lat * Math.PI) / 180;
  const lat2 = (to.lat * Math.PI) / 180;
  const a =
    Math.sin(dLat / 2) ** 2 +
    Math.sin(dLng / 2) ** 2 * Math.cos(lat1) * Math.cos(lat2);
  return 2 * EARTH_RADIUS_M * Math.asin(Math.sqrt(a));
}

/**
 * CO-LOCATION MOVED TO THE RULESET, and this note is the signpost.
 *
 * `CO_LOCATION_M = 250` and `isCoLocated` used to live here. They were read
 * by nothing but their own test, and 250 m decides outcomes — who may fire
 * together (9.2.1), who is drawn into an assault (9.3), who an HQ steadies —
 * so by ruleset.ts's own first rule it may not be hidden in a geometry
 * helper: "Anything that decides an outcome lives here, is named, is
 * attributable to a ruleset id, and can be changed without editing a
 * resolver."
 *
 * It is `RuleSet.coLocatedM` now. This file stays pure geometry, which is
 * also why it does not import the ruleset to get it.
 */

/**
 * Bearing from one point to another, in degrees clockwise from north.
 *
 * Needed for ASPECT: which face of a vehicle a shot arrives at. Without it
 * `flank` was a declared modifier that nothing could ever set — see
 * RuleSet.frontArcDeg.
 */
export function bearingDeg(from: LatLng, to: LatLng): number {
  const dLat = to.lat - from.lat;
  // Longitude degrees are shorter than latitude degrees away from the
  // equator, and ignoring that would tilt every bearing by up to 30° at this
  // latitude — enough to turn a frontal shot into a flank one.
  const dLng = (to.lng - from.lng) * Math.cos((((from.lat + to.lat) / 2) * Math.PI) / 180);
  const degrees = (Math.atan2(dLng, dLat) * 180) / Math.PI;
  return (degrees + 360) % 360;
}

/** The smaller angle between two bearings, 0-180. */
export function bearingDeltaDeg(a: number, b: number): number {
  const delta = Math.abs(a - b) % 360;
  return delta > 180 ? 360 - delta : delta;
}
