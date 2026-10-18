import { describe, expect, it } from "vitest";

import { TerrainFlag } from "../../../shared/routing/types";
import { distanceM } from "./board";
import { consumeAllowance } from "./movement";
import {
  DEFAULT_TURN_MINUTES,
  VEHICLE_CONTACT_FRACTION,
  allowanceFromSpeeds,
  legsFromWaypoints,
  mobilityClassFor,
  samplerFromFlags,
  terrainClassForFlag,
  IMPASSABLE_FLAG,
} from "./terrainAdapter";

describe("flag translation", () => {
  it("maps every flag the raster can produce", () => {
    expect(terrainClassForFlag(TerrainFlag.Open)).toBe("open");
    expect(terrainClassForFlag(TerrainFlag.Road)).toBe("road");
    expect(terrainClassForFlag(TerrainFlag.Forest)).toBe("woodsLight");
    expect(terrainClassForFlag(TerrainFlag.Marsh)).toBe("marsh");
    expect(terrainClassForFlag(IMPASSABLE_FLAG)).toBe("water");
  });

  it("returns null outside coverage rather than guessing open ground", () => {
    // "No data" and "open ground" are opposite answers for a wargame: one
    // means unknown, the other means drive across it.
    expect(terrainClassForFlag(null)).toBeNull();
    expect(terrainClassForFlag(99)).toBeNull();
  });

  it("maps move types onto the router's mobility classes", () => {
    expect(mobilityClassFor("F")).toBe("foot");
    expect(mobilityClassFor("W")).toBe("wheeled");
    expect(mobilityClassFor("T")).toBe("tracked");
  });
});

describe("the derived allowance table", () => {
  const table = allowanceFromSpeeds(DEFAULT_TURN_MINUTES);

  it("reproduces the one allowance the rulebook actually states", () => {
    // "1,000 m along a road is half a Foot FE's road allowance" → 2,000 m.
    expect(table.F.road).toBe(2000);
  });

  it("runs exactly 25% high on the other figure the example implies", () => {
    // The book's example implies 1,000 m of open ground for a Foot FE; the
    // repo's 5 km/h over fifteen minutes gives 1,250 — a quarter too
    // generous, and generous in the direction that lets units reach further
    // than doctrine allows, which is the direction worth knowing about.
    //
    // Asserted as an equality rather than a bound because this number is
    // documentation: when the real Player Aid table arrives, this test should
    // fail loudly and be deleted, not quietly keep passing.
    expect(table.F.open).toBe(1250);
    expect((table.F.open! - 1000) / 1000).toBeCloseTo(0.25, 6);
  });

  it("keeps wheeled vehicles out of the marsh", () => {
    // The speed table has this at 0 km/h, and 0 has to survive the conversion
    // as impassable rather than becoming "0 m, which rounds to nothing".
    expect(table.W.marsh).toBe(0);
    const blocked = consumeAllowance(
      "W",
      [{ terrain: "marsh", distanceM: 10, to: { lat: 0, lng: 0 } }],
      table,
      { lat: 0, lng: 0 },
    );
    expect(blocked.outcome).toBe("blocked");
  });

  it("scales with turn length", () => {
    expect(allowanceFromSpeeds(30).F.road).toBe(4000);
  });

  it("scales vehicles to a contact rate and leaves foot alone", () => {
    // The source is a route planner's table: 20 km/h cross-country for tracks
    // is a march, not a bound under fire. Foot is NOT scaled, because the
    // rulebook states the Foot allowance and the unscaled table reproduces it.
    const contact = allowanceFromSpeeds(DEFAULT_TURN_MINUTES, 0.3);
    expect(contact.F.road).toBe(table.F.road);
    expect(contact.F.open).toBe(table.F.open);
    expect(contact.T.open).toBe(1500);
    expect(contact.W.open).toBe(1875);
  });

  it("puts the SHIPPING fraction on the record", () => {
    // Documentation with teeth: these are the allowances every game is
    // actually played with, swept over 5 candidates x 4 force lists. If a
    // change to the fraction or the speed table moves them, this test should
    // fail and the new figures should be argued for, not discovered later.
    const shipped = allowanceFromSpeeds(DEFAULT_TURN_MINUTES, VEHICLE_CONTACT_FRACTION);
    expect(VEHICLE_CONTACT_FRACTION).toBe(0.45);
    expect(shipped.T.open).toBe(2250);
    expect(shipped.T.woodsLight).toBe(450);
    expect(shipped.T.road).toBe(5063);
    expect(shipped.W.open).toBe(2813);
    expect(shipped.F.open).toBe(1250);
  });

  it("keeps impassable impassable however it is scaled", () => {
    // "Cannot" and "slowly" are different rules. A fraction that turned 0 into
    // a small positive number would quietly let trucks ford rivers, and a
    // rounding that turned a small positive number into 0 would quietly make
    // woodland a wall.
    const tiny = allowanceFromSpeeds(DEFAULT_TURN_MINUTES, 0.001);
    expect(tiny.W.marsh).toBe(0);
    expect(tiny.W.water).toBe(0);
    expect(tiny.T.marsh).toBeGreaterThan(0);
    expect(tiny.T.woodsLight).toBeGreaterThan(0);
  });

  it("gives tracked vehicles more cross-country reach than wheeled", () => {
    // Not a rule, a sanity check: if this ever inverts, the mobility mapping
    // has been wired up backwards.
    expect(table.T.woodsLight!).toBeGreaterThan(table.W.woodsLight!);
  });
});

describe("classifying a route", () => {
  const flags = {
    // A road that becomes forest east of a line.
    flagAt: (_lat: number, lng: number) =>
      lng > 20.52 ? TerrainFlag.Forest : TerrainFlag.Road,
  };

  it("classifies each leg at its midpoint, not its start", () => {
    // A leg that starts on the road and ends in the trees is mostly trees;
    // classifying by the start point would make it a road for free.
    const legs = legsFromWaypoints(
      [
        [54.71, 20.5],
        [54.71, 20.51],
        [54.71, 20.54],
      ],
      flags,
      distanceM,
    );
    expect(legs).toHaveLength(2);
    expect(legs[0].terrain).toBe("road");
    expect(legs[1].terrain).toBe("woodsLight");
    expect(legs[1].distanceM).toBeGreaterThan(0);
  });

  it("produces nothing from a single waypoint", () => {
    expect(legsFromWaypoints([[54.71, 20.5]], flags, distanceM)).toEqual([]);
  });
});

describe("the line-of-sight sampler", () => {
  it("blocks on woods even with no elevation data at all", () => {
    // The degraded case: flat ground, so hills are missing, but a wood is
    // still a wood. This is what the board does before a DEM is granted.
    const sampler = samplerFromFlags({
      flagAt: () => TerrainFlag.Forest,
    });
    expect(sampler.classify({ lat: 54.71, lng: 20.51 })).toBe("woodsLight");
    expect(sampler.groundHeightM({ lat: 54.71, lng: 20.51 })).toBe(0);
  });

  it("uses the elevation source when there is one", () => {
    const sampler = samplerFromFlags({ flagAt: () => TerrainFlag.Open }, () => 137);
    expect(sampler.groundHeightM({ lat: 54.71, lng: 20.51 })).toBe(137);
  });
});
