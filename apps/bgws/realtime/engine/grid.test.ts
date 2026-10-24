import { describe, expect, it } from "vitest";
import { distanceM, type LatLng } from "../../lib/board";
import { createRng } from "../../rules/dice";
import { PositionGrid } from "./grid";

describe("PositionGrid", () => {
  // The grid may only narrow: every point truly within the radius must come back.
  it.each([
    ["the Baltic", 54.2, 18.6],
    ["near the equator", 1, 30],
    ["far north", 69, 20],
    ["south", -34, 151],
  ])("never misses a point within the radius (%s), and returns indices in order", (_where, lat, lng) => {
    const rng = createRng(`grid-${lat}`);
    const spread = 0.25; // ~25 km: wider than any board
    const points: LatLng[] = Array.from({ length: 400 }, () => ({
      lat: lat + (rng.int(1_000_000) / 1_000_000 - 0.5) * spread,
      lng: lng + (rng.int(1_000_000) / 1_000_000 - 0.5) * spread,
    }));
    for (const radius of [150, 300, 3000]) {
      const grid = new PositionGrid(points, radius);
      for (const at of points.slice(0, 80)) {
        const near = grid.near(at);
        expect(near).toEqual([...near].sort((a, b) => a - b));
        const found = new Set(near);
        points.forEach((p, i) => {
          if (distanceM(p, at) <= radius) expect(found.has(i)).toBe(true);
        });
      }
    }
  });

  it("leaves out what is far away", () => {
    const grid = new PositionGrid([{ lat: 54, lng: 18 }, { lat: 54.5, lng: 18 }], 3000);
    expect(grid.near({ lat: 54, lng: 18 })).toEqual([0]);
  });
});
