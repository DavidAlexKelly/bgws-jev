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
import { counterLabel, counterSidc } from "./counterSymbol";
import type { PlatformProfile } from "../data/profiles";

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

function platform(overrides: Partial<PlatformProfile>): PlatformProfile {
  return {
    assetId: "x",
    displayName: "Uk Challenger 2 Tes",
    domain: "armour",
    nation: "united_kingdom",
    moveType: "T",
    targetClass: "armoured_vehicle",
    protectionBand: "heavy",
    capabilities: ["apers", "atk"],
    hasAtgm: false,
    hasSmoke: false,
    massCombatT: 74.8,
    massSource: "combat_mass_takeoff",
    engineHp: 1314,
    hpPerTonne: 17.6,
    hpPerTonneMethod: "engine_hp_over_combat_mass",
    mobilityClass: "adequate",
    gearLimitedKmh: 59.2,
    gearLimitedConfidence: "low_estimate_no_gear_table",
    statcardSpeedKmh: 75,
    statcardSpeedIsTemplate: true,
    mobilityModifier: 0,
    armourMaxMm: 600,
    crewSize: 4,
    bestPenMm1000m: null,
    csIndex: 8.6,
    confidence: 1,
    ...overrides,
  };
}

describe("placeholder counter symbology", () => {
  it("produces a 15-character SIDC with the right affiliation", () => {
    const blue = counterSidc(platform({}), "blue");
    const red = counterSidc(platform({}), "red");
    expect(blue).toHaveLength(15);
    expect(blue[1]).toBe("F");
    expect(red[1]).toBe("H");
  });

  it("reads the role from capability, not from the name", () => {
    // The source calls this "Uk M1a2 Sep3 Abrams". Nation and name are a game
    // tech tree; what it can shoot is data.
    const antiTank = counterSidc(platform({ capabilities: ["atm", "apers"] }), "blue");
    const armour = counterSidc(platform({ capabilities: ["atk", "apers"] }), "blue");
    expect(antiTank).not.toBe(armour);
  });

  it("leaves echelon unset, because a platform has none", () => {
    // Position 11 is the echelon field. A company's bar over a single vehicle
    // would be a lie the map tells convincingly.
    expect(counterSidc(platform({}), "blue")[11]).toBe("-");
  });

  it("drops the nation prefix from the label", () => {
    expect(counterLabel(platform({}))).toBe("Challenger 2 Tes");
    expect(counterLabel(platform({ displayName: "Us M1a2 Sep3 Abrams" }))).toBe(
      "M1a2 Sep3 Abrams",
    );
  });
});
