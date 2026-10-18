import { describe, expect, it } from "vitest";

import {
  BOARD_SIZE_M,
  bearingDeg,
  bearingDeltaDeg,
  boardBounds,
  boardRing,
  distanceM,
  isOnBoard,
} from "./board";

const KALININGRAD = { lat: 54.71, lng: 20.51 };

describe("the sheet", () => {
  const bounds = boardBounds(KALININGRAD);

  it("is 10 km across in both directions", () => {
    const acrossM = distanceM(
      { lat: KALININGRAD.lat, lng: bounds.west },
      { lat: KALININGRAD.lat, lng: bounds.east },
    );
    const upM = distanceM(
      { lat: bounds.south, lng: KALININGRAD.lng },
      { lat: bounds.north, lng: KALININGRAD.lng },
    );
    // Within 1% — the constants are a local approximation, not a projection.
    expect(acrossM).toBeGreaterThan(BOARD_SIZE_M * 0.99);
    expect(acrossM).toBeLessThan(BOARD_SIZE_M * 1.01);
    expect(upM).toBeGreaterThan(BOARD_SIZE_M * 0.99);
    expect(upM).toBeLessThan(BOARD_SIZE_M * 1.01);
  });

  it("stays square as latitude rises", () => {
    // A degree of longitude is half as wide at 60°N as at the equator. If the
    // board were built from a fixed degree offset it would be a letterbox in
    // the arctic; this is the test that catches that.
    const arctic = boardBounds({ lat: 69.6, lng: 18.9 });
    const acrossM = distanceM(
      { lat: 69.6, lng: arctic.west },
      { lat: 69.6, lng: arctic.east },
    );
    expect(acrossM).toBeGreaterThan(BOARD_SIZE_M * 0.99);
    expect(acrossM).toBeLessThan(BOARD_SIZE_M * 1.01);
  });

  it("closes its ring so a polygon renders", () => {
    const ring = boardRing(bounds);
    expect(ring).toHaveLength(5);
    expect(ring[0]).toEqual(ring[4]);
  });

  it("knows what is off the sheet", () => {
    expect(isOnBoard(KALININGRAD, bounds)).toBe(true);
    expect(isOnBoard({ lat: KALININGRAD.lat, lng: KALININGRAD.lng + 0.2 }, bounds)).toBe(false);
  });
});

describe("bearings", () => {
  // Aspect is the whole point of these: which face of a vehicle a shot
  // arrives at. See RuleSet.frontArcDeg.
  const origin = { lat: 54.71, lng: 20.51 };

  it("reads clockwise from north", () => {
    expect(bearingDeg(origin, { lat: 54.72, lng: 20.51 })).toBeCloseTo(0, 0);
    expect(bearingDeg(origin, { lat: 54.71, lng: 20.53 })).toBeCloseTo(90, 0);
    expect(bearingDeg(origin, { lat: 54.7, lng: 20.51 })).toBeCloseTo(180, 0);
    expect(bearingDeg(origin, { lat: 54.71, lng: 20.49 })).toBeCloseTo(270, 0);
  });

  it("corrects for longitude being shorter than latitude up here", () => {
    // A degree of longitude at 54.71°N is about 58% of a degree of latitude.
    // Ignoring that would report this as 45° and turn frontal shots into
    // flank ones.
    const northEast = bearingDeg(origin, { lat: 54.72, lng: 20.52 });
    expect(northEast).toBeGreaterThan(20);
    expect(northEast).toBeLessThan(40);
  });

  it("measures the smaller angle between two bearings", () => {
    expect(bearingDeltaDeg(10, 350)).toBeCloseTo(20, 0);
    expect(bearingDeltaDeg(0, 180)).toBeCloseTo(180, 0);
    expect(bearingDeltaDeg(90, 90)).toBeCloseTo(0, 0);
  });
});
