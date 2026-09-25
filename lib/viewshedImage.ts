// ── bgws/lib/viewshedImage.ts ──────────────────────────────────────────────
// A viewshed, as pixels.
//
// @acc/decho-elevation answers "what can this position see" as a square grid
// of bytes — one of VISIBLE, HIDDEN, NO_DATA or OUTSIDE per cell, row-major
// from the north-west. Drawing it needs RGBA, and MapLibre needs a URL.
//
// THE COLOURING IS PURE AND THE ENCODING IS NOT. `viewshedRgba` is arithmetic
// over two typed arrays and is tested; `viewshedDataUrl` wraps it in a canvas,
// which needs a DOM and is therefore the thinnest possible shell around it.
// The split exists because every interesting decision — which states are drawn
// at all, and what "cannot see" looks like — lives in the pure half.

import {
  VIEWSHED_HIDDEN,
  VIEWSHED_NO_DATA,
  VIEWSHED_VISIBLE,
  type Viewshed,
} from "@acc/decho-elevation";

/** Red, green, blue, alpha. 0-255. */
export type Rgba = readonly [number, number, number, number];

export interface ViewshedColours {
  visible: Rgba;
  hidden: Rgba;
  noData: Rgba;
}

/**
 * ⚠ DEAD GROUND IS THE THING WORTH DRAWING, NOT VISIBLE GROUND.
 *
 * The instinct is to wash what an observer CAN see, and on a 3 km radius that
 * covers most of the picture: a bright overlay over almost everything, hiding
 * the map to tell the reader what they already assume. What a commander needs
 * is the opposite — the dead ground, the folds an enemy can cross unseen —
 * because that is the information that changes a decision.
 *
 * So `visible` is fully transparent and `hidden` is the mark. A viewshed on
 * this board reads as shadow where a hill hides the ground behind it.
 *
 * NO_DATA is drawn too, and distinctly. A cell with no DEM coverage is not
 * dead ground, it is ground nobody has measured, and the two must not look
 * alike — one is a tactical fact and the other is a gap in the dataset.
 */
export const DEAD_GROUND_COLOURS: ViewshedColours = {
  visible: [0, 0, 0, 0],
  hidden: [12, 14, 20, 110],
  noData: [140, 110, 180, 70],
};

/**
 * Colour a viewshed's cells into RGBA, row-major from the north-west.
 *
 * `OUTSIDE` — the corners of the square that fall beyond the radius — is left
 * fully transparent along with everything unrecognised, so the overlay reads
 * as a disc rather than as a box drawn over the map.
 */
export function viewshedRgba(
  cells: Uint8Array,
  colours: ViewshedColours = DEAD_GROUND_COLOURS,
): Uint8ClampedArray {
  const rgba = new Uint8ClampedArray(cells.length * 4);
  for (let i = 0; i < cells.length; i++) {
    const colour =
      cells[i] === VIEWSHED_HIDDEN
        ? colours.hidden
        : cells[i] === VIEWSHED_NO_DATA
          ? colours.noData
          : cells[i] === VIEWSHED_VISIBLE
            ? colours.visible
            : null;
    if (!colour) continue;
    rgba[i * 4] = colour[0];
    rgba[i * 4 + 1] = colour[1];
    rgba[i * 4 + 2] = colour[2];
    rgba[i * 4 + 3] = colour[3];
  }
  return rgba;
}

/**
 * The four corners MapLibre's `image` source wants, in its order.
 *
 * Top-left, top-right, bottom-right, bottom-left — and the viewshed's own rows
 * run north to south, so the image's first row is the NORTH edge. Getting this
 * pair the wrong way round flips the overlay vertically, which on symmetrical
 * terrain looks almost right, so it is worth doing in one named place.
 */
export function viewshedCorners(
  bounds: Viewshed["bounds"],
): [[number, number], [number, number], [number, number], [number, number]] {
  return [
    [bounds.west, bounds.north],
    [bounds.east, bounds.north],
    [bounds.east, bounds.south],
    [bounds.west, bounds.south],
  ];
}

/** The same pixels as a data URL, for an `image` source. Needs a DOM. */
export function viewshedDataUrl(
  view: Viewshed,
  colours: ViewshedColours = DEAD_GROUND_COLOURS,
): string {
  const canvas = document.createElement("canvas");
  canvas.width = view.size;
  canvas.height = view.size;
  const context = canvas.getContext("2d");
  if (!context) return "";
  context.putImageData(
    new ImageData(viewshedRgba(view.cells, colours), view.size, view.size),
    0,
    0,
  );
  return canvas.toDataURL("image/png");
}
