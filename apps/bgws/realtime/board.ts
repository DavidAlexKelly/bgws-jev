// ── bgws/realtime/board.ts ─────────────────────────────────────────────────
// Keeping placed units on the board when the board moves.

import { metresPerDegreeLon, type LatLng } from "../lib/board";
import { DEFAULT_ORIGIN, offsetToLatLng, type PlacedElement } from "../lib/forceBuilder";

/**
 * Units placed around one board centre, moved to the same spots around another.
 *
 * On real ground the board recentres onto the raster's coverage, which can be
 * a long way from where it started. Everything already placed keeps its
 * layout and moves with it, rather than being stranded off the edge.
 */
export function moveBoard(placed: PlacedElement[], from: LatLng, to: LatLng): PlacedElement[] {
  if (from.lat === to.lat && from.lng === to.lng) return placed;
  return placed.map((element) => ({
    ...element,
    position: offsetToLatLng(
      to,
      (element.position.lng - from.lng) * metresPerDegreeLon(from.lat),
      (element.position.lat - from.lat) * 111_320,
    ),
  }));
}

/** A force list (laid out around DEFAULT_ORIGIN), moved onto the board at `origin`. */
export function onBoard(placed: PlacedElement[], origin: LatLng): PlacedElement[] {
  return moveBoard(placed, DEFAULT_ORIGIN, origin);
}
