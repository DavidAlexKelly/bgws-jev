import { describe, expect, it } from "vitest";

import { groundFixedScale, metresPerPixel } from "./mapScale";

// The board is played in the Baltic; 54.7°N is the scenario origin.
const BALTIC = 54.7;

describe("ground per pixel", () => {
  it("halves with every zoom level", () => {
    // The defining property of the zoom scale. If this is wrong, everything
    // built on it is wrong by a power of two.
    const z10 = metresPerPixel(10, 0);
    const z11 = metresPerPixel(11, 0);
    expect(z11).toBeCloseTo(z10 / 2, 6);
  });

  it("matches the known figure at the equator", () => {
    // Zoom 0, 256px tile, equator: ~156.5 km per pixel. A standard reference
    // value, so a typo in the constant shows up here rather than as counters
    // that are subtly the wrong size everywhere.
    expect(metresPerPixel(0, 0)).toBeCloseTo(156543.03, 1);
  });

  it("covers less ground per pixel away from the equator", () => {
    // ⚠ THE LATITUDE TERM IS NOT OPTIONAL. Mercator stretches with latitude,
    // so dropping it sizes counters correctly on the equator and
    // progressively wrongly everywhere else — including everywhere this game
    // is played. At 54.7°N the error would be about 42%.
    expect(metresPerPixel(12, BALTIC)).toBeLessThan(metresPerPixel(12, 0));
    expect(metresPerPixel(12, BALTIC)).toBeCloseTo(
      metresPerPixel(12, 0) * Math.cos((BALTIC * Math.PI) / 180),
      6,
    );
  });
});

describe("sizing a counter to the ground it covers", () => {
  const counter = { groundMetres: 250, basePx: 26, latitude: BALTIC };

  it("shrinks as you zoom out", () => {
    // The whole point of the change: zoomed out, a unit is further away.
    const close = groundFixedScale({ ...counter, zoom: 14 });
    const far = groundFixedScale({ ...counter, zoom: 12 });
    expect(far).toBeLessThan(close);
  });

  it("halves for each zoom level out", () => {
    const z14 = groundFixedScale({ ...counter, zoom: 14 });
    const z13 = groundFixedScale({ ...counter, zoom: 13 });
    expect(z13).toBeCloseTo(z14 / 2, 4);
  });

  it("puts a 250m counter at roughly its own size at play zoom", () => {
    // A sanity anchor rather than a tuning target: at zoom 14 in the Baltic a
    // pixel is about 6.6 m, so 250 m is ~38 px against a 26 px symbol. If
    // this drifts far from 1x, the base size and the frontage have stopped
    // agreeing and counters will be unreadable before anyone checks the maths.
    const scale = groundFixedScale({ ...counter, zoom: 14 });
    expect(scale).toBeGreaterThan(0.8);
    expect(scale).toBeLessThan(2.5);
  });

  it("never shrinks a counter below a findable size", () => {
    // Honest scaling at zoom 4 is a fraction of a pixel: invisible,
    // unclickable, and indistinguishable from a unit that is not there.
    const scale = groundFixedScale({ ...counter, zoom: 4, minPx: 6 });
    expect(scale * counter.basePx).toBeCloseTo(6, 5);
  });

  it("leaves the floor out of the way at any zoom worth playing at", () => {
    // The floor must be a safety net, not a silent cap that makes the scaling
    // a lie wherever it is actually looked at.
    const withFloor = groundFixedScale({ ...counter, zoom: 12, minPx: 6 });
    const without = groundFixedScale({ ...counter, zoom: 12, minPx: 0 });
    expect(withFloor).toBe(without);
  });

  it("does not divide by a zero base size", () => {
    expect(groundFixedScale({ ...counter, basePx: 0, zoom: 14 })).toBe(1);
  });
});
