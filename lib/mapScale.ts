// ── bgws/lib/mapScale.ts ───────────────────────────────────────────────────
// Counters that occupy GROUND rather than screen.
//
// A MapLibre Marker is screen-fixed: it stays the same number of pixels at
// every zoom, so a counter looks identical whether the board fills the window
// or is a postage stamp. That is right for a pin and wrong for a unit. A
// company occupies a frontage, and at this scale the frontage is the whole
// point — two counters that overlap on the ground are in contact, and a
// screen-fixed symbol hides that at low zoom and invents it at high zoom.
//
// So the symbol is sized in METRES and converted to pixels for the current
// camera. Zoom out and it shrinks, exactly as the terrain does.

/** Earth's circumference at the equator, in metres, as Web Mercator uses it. */
const EQUATOR_M = 40075016.686;

/** MapLibre's zoom is defined against 256px tiles. */
const TILE_PX = 256;

/**
 * Ground distance covered by one screen pixel.
 *
 * Mercator stretches with latitude, so this is a function of BOTH zoom and
 * where you are: the same zoom covers less ground per pixel near the poles.
 * Ignoring the latitude term would size counters correctly on the equator and
 * progressively wrongly everywhere else — and this game is played in the
 * Baltic.
 */
export function metresPerPixel(zoom: number, latitude: number): number {
  const shrink = Math.cos((latitude * Math.PI) / 180);
  return (EQUATOR_M * shrink) / (TILE_PX * Math.pow(2, zoom));
}

export interface GroundScaleOptions {
  /** How much ground the symbol should cover, edge to edge. */
  groundMetres: number;
  /** The symbol's natural size in pixels, unscaled. */
  basePx: number;
  zoom: number;
  latitude: number;
  /**
   * Never shrink below this many pixels.
   *
   * ⚠ A DELIBERATE DEPARTURE FROM TRUE GROUND SCALE, AND THE ONLY ONE.
   *
   * Zoomed far enough out, honest scaling makes a counter a fraction of a
   * pixel: invisible, unclickable, and indistinguishable from a unit that is
   * not there. A floor keeps it findable. It is set low enough that it only
   * engages when the board is already too small to play on, so at every zoom
   * anyone actually uses, the size is true.
   */
  minPx?: number;
}

/**
 * The CSS scale factor for a symbol that should occupy a fixed patch of
 * ground.
 *
 * Returns a multiplier rather than a pixel size because the symbol is drawn
 * once by milsymbol at its natural size and then transformed — re-rendering
 * an SVG per counter per zoom frame would be the expensive way to do this.
 */
export function groundFixedScale({
  groundMetres,
  basePx,
  zoom,
  latitude,
  minPx = 6,
}: GroundScaleOptions): number {
  if (basePx <= 0) return 1;
  const wantedPx = groundMetres / metresPerPixel(zoom, latitude);
  return Math.max(wantedPx, minPx) / basePx;
}
