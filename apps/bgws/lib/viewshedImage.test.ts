import { describe, expect, it } from "vitest";

import {
  VIEWSHED_HIDDEN,
  VIEWSHED_NO_DATA,
  VIEWSHED_OUTSIDE,
  VIEWSHED_VISIBLE,
} from "@acc/decho-elevation";

import { DEAD_GROUND_COLOURS, viewshedCorners, viewshedRgba } from "./viewshedImage";

const alphaAt = (rgba: Uint8ClampedArray, index: number) => rgba[index * 4 + 3];

describe("a viewshed, as pixels", () => {
  it("marks dead ground and leaves visible ground alone", () => {
    // ⚠ THE DECISION THIS TEST EXISTS TO PIN. Washing what an observer CAN see
    // covers most of a 3 km disc and hides the map to say what the reader
    // already assumes. The information that changes a decision is the dead
    // ground, so that is what carries the ink.
    const cells = new Uint8Array([VIEWSHED_VISIBLE, VIEWSHED_HIDDEN]);
    const rgba = viewshedRgba(cells);
    expect(alphaAt(rgba, 0)).toBe(0);
    expect(alphaAt(rgba, 1)).toBeGreaterThan(0);
  });

  it("tells unmeasured ground apart from hidden ground", () => {
    // One is a tactical fact, the other is a hole in the dataset. Drawing them
    // the same would let a missing DEM cell read as a fold in the ground.
    const rgba = viewshedRgba(new Uint8Array([VIEWSHED_HIDDEN, VIEWSHED_NO_DATA]));
    const hidden = [0, 1, 2].map((c) => rgba[0 * 4 + c]);
    const noData = [0, 1, 2].map((c) => rgba[1 * 4 + c]);
    expect(hidden).not.toEqual(noData);
    expect(alphaAt(rgba, 1)).toBeGreaterThan(0);
  });

  it("leaves the square's corners transparent, so it reads as a disc", () => {
    // OUTSIDE is the part of the grid beyond the radius. Painted, the overlay
    // would be a box drawn over the map with a circle inside it.
    expect(alphaAt(viewshedRgba(new Uint8Array([VIEWSHED_OUTSIDE])), 0)).toBe(0);
  });

  it("colours every cell it is given", () => {
    const cells = new Uint8Array(9).fill(VIEWSHED_HIDDEN);
    expect(viewshedRgba(cells)).toHaveLength(9 * 4);
  });

  it("puts the image's first row on the NORTH edge", () => {
    // The viewshed's rows run north to south. MapLibre wants top-left first,
    // so the first corner has to be the northern one — transposing the pair
    // flips the overlay, which on symmetrical terrain looks almost right.
    const corners = viewshedCorners({ west: 1, south: 2, east: 3, north: 4 });
    expect(corners[0]).toEqual([1, 4]);
    expect(corners[2]).toEqual([3, 2]);
  });

  it("keeps visible ground fully clear in the shipped colours", () => {
    expect(DEAD_GROUND_COLOURS.visible[3]).toBe(0);
  });
});
