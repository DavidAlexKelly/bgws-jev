import { describe, expect, it } from "vitest";

import { TerrainFlag } from "../../../shared/routing/types";
import { BOARD_SIZE_M, type LatLng } from "./board";
import { STANDARD_GROUND } from "./proceduralTerrain";
import {
  centreOf,
  coversBoard,
  describeTerrainSource,
  rasterTerrain,
} from "./rasterTerrain";

const CENTRE: LatLng = { lat: 54.71, lng: 20.51 };

/** A raster covering a box around CENTRE, with a wood and a lake in it. */
function fakeRaster(halfSpanDeg = 0.5) {
  return {
    flagAt(lat: number, lng: number): number | null {
      if (
        Math.abs(lat - CENTRE.lat) > halfSpanDeg ||
        Math.abs(lng - CENTRE.lng) > halfSpanDeg
      ) {
        return null; // outside coverage
      }
      if (lng > CENTRE.lng + 0.01) return TerrainFlag.Forest;
      if (lat > CENTRE.lat + 0.01) return 255; // deep water
      return TerrainFlag.Open;
    },
  };
}

describe("terrain from the real raster", () => {
  it("classifies from the raster's flags", () => {
    const terrain = rasterTerrain(fakeRaster());
    expect(terrain.classify(CENTRE)).toBe("open");
    expect(terrain.classify({ lat: CENTRE.lat, lng: CENTRE.lng + 0.02 })).toBe("woodsLight");
    expect(terrain.classify({ lat: CENTRE.lat + 0.02, lng: CENTRE.lng })).toBe("water");
  });

  it("is flat without relief, because the raster carries no elevation", () => {
    const terrain = rasterTerrain(fakeRaster());
    expect(terrain.groundHeightM(CENTRE)).toBe(0);
    expect(terrain.groundHeightM({ lat: CENTRE.lat + 0.005, lng: CENTRE.lng })).toBe(0);
  });

  it("takes real heights from an injected sampler", () => {
    // The seam the `relief` option's comment promised: a height function, with
    // no DEM, dataset or decoder anywhere in the module under test. In the app
    // this is @acc/decho-elevation's heightAtLoaded; here it is a closure.
    const terrain = rasterTerrain(fakeRaster(), {
      heightAt: (point) => (point.lat > CENTRE.lat ? 210 : 140),
    });
    expect(terrain.groundHeightM(CENTRE)).toBe(140);
    expect(terrain.groundHeightM({ lat: CENTRE.lat + 0.005, lng: CENTRE.lng })).toBe(210);
    // Cover still comes from the raster.
    expect(terrain.classify({ lat: CENTRE.lat, lng: CENTRE.lng + 0.02 })).toBe("woodsLight");
  });

  it("passes NaN through rather than flattening it to sea level", () => {
    // ⚠ NaN IS COVERAGE INFORMATION, NOT A FAILURE. The DEM answers NaN for a
    // cell that is ocean or simply not resident yet, and `lineOfSight` reads a
    // non-finite height as `noCoverage` and refuses the shot. A sampler that
    // helpfully substituted 0 would put the board at sea level and report it
    // as ordinary flat ground — every sight line clear, and nothing to show
    // that the DEM had not loaded.
    const terrain = rasterTerrain(fakeRaster(), { heightAt: () => Number.NaN });
    expect(Number.isNaN(terrain.groundHeightM(CENTRE))).toBe(true);
  });

  it("prefers real heights to invented ones when given both", () => {
    // Mixing a DEM with generated relief would produce ground that is neither
    // measured nor repeatable, and no caller means to ask for it.
    const terrain = rasterTerrain(fakeRaster(), {
      relief: STANDARD_GROUND,
      heightAt: () => 175,
    });
    expect(terrain.groundHeightM(CENTRE)).toBe(175);
    expect(terrain.groundHeightM({ lat: CENTRE.lat + 0.008, lng: CENTRE.lng })).toBe(175);
  });

  it("takes relief from the generator when asked, and says so elsewhere", () => {
    // Real cover, invented hills. Defensible only because it is labelled: flat
    // ground was measured to disable a third of the rulebook, and the raster
    // has no DEM to offer instead.
    const terrain = rasterTerrain(fakeRaster(), { relief: STANDARD_GROUND });
    const heights = [0, 0.004, 0.008, 0.012].map((d) =>
      terrain.groundHeightM({ lat: CENTRE.lat + d, lng: CENTRE.lng }),
    );
    expect(new Set(heights).size).toBeGreaterThan(1);
    // Cover still comes from the raster, not from the relief generator.
    expect(terrain.classify({ lat: CENTRE.lat, lng: CENTRE.lng + 0.02 })).toBe("woodsLight");
  });
});

describe("coverage is checked before anything is played on it", () => {
  it("accepts a board that fits inside the data", () => {
    expect(coversBoard(fakeRaster(), CENTRE, BOARD_SIZE_M)).toBe(true);
  });

  it("rejects a board whose corners fall off the edge", () => {
    // THE FAILURE THIS PREVENTS: unknown ground reads as open ground by the
    // time it reaches the game, so a board half off the raster would play as
    // though its flank were a billiard table.
    const narrow = fakeRaster(0.02); // ~2 km, smaller than a 10 km board
    expect(coversBoard(narrow, CENTRE, BOARD_SIZE_M)).toBe(false);
  });

  it("rejects a board centred outside the data entirely", () => {
    // The BGWS default board is near Gdansk; the default raster is Kaliningrad.
    expect(coversBoard(fakeRaster(), { lat: 54.2, lng: 18.6 }, BOARD_SIZE_M)).toBe(false);
  });

  it("finds the middle of the covered area", () => {
    const centre = centreOf({ minLat: 54, minLon: 20, maxLat: 55, maxLon: 21 });
    expect(centre).toEqual({ lat: 54.5, lng: 20.5 });
  });
});

describe("the player is told what the ground is", () => {
  it("never describes generated relief as real", () => {
    expect(describeTerrainSource("raster+relief", "Kaliningrad", "baltic-v1-wet")).toContain(
      "relief generated",
    );
    expect(describeTerrainSource("raster", "Kaliningrad", "x")).toContain("no DEM");
    expect(describeTerrainSource("generated", "Kaliningrad", "baltic-v1-wet")).toContain(
      "not a real place",
    );
  });
});
