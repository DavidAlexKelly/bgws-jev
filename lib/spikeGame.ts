// ── bgws/lib/spikeGame.ts ──────────────────────────────────────────────────
// Enough game state to put the rules on screen, before there is a scenario.
//
// The board lets you drop equipment on the map. The rules want Force Elements
// in a GameState. This bridges the two so fog of war and line of sight can be
// SEEN rather than only unit-tested — which matters, because both are rules
// whose bugs are invisible in a passing test suite and obvious on a map.
//
// ⚠ PROVISIONAL NUMBERS LIVE HERE, AND ONLY HERE.
//
// A Force Element needs a Troop Quality and a Combat Strength. Neither is a
// property of equipment: they come from a scenario's Force Management Chart.
// Until there is one, this module supplies placeholders — and it is the single
// place in the app that does, so that deleting it is how they leave.

import type { PlatformProfile } from "../data/profiles";
import type { LatLng } from "./board";
import { lineOfSight, type TerrainSampler } from "./lineOfSight";
import type { Capability, ForceElement, GameState, Side, SightingLevel } from "./state";
import { opposing } from "./state";

/** Placeholder Troop Quality. Mid-scale: a competent regular sub-unit. */
export const PROVISIONAL_TROOP_QUALITY = 4;

export interface PlacedUnit {
  id: string;
  side: Side;
  platform: PlatformProfile;
  position: LatLng;
}

function capabilitiesOf(platform: PlatformProfile): Capability[] {
  // Ranges come from the equipment profile, which is real data. The Combat
  // Strength modifiers do not: they are the Force Management Chart's.
  const ranges: Partial<Record<string, number>> = {
    apers: platform.capabilities.includes("apers") ? 3000 : undefined,
    atk: platform.capabilities.includes("atk") ? 3000 : undefined,
    atm: platform.capabilities.includes("atm") ? 3000 : undefined,
  };
  return Object.entries(ranges)
    .filter(([, maxRangeM]) => maxRangeM != null)
    .map(([kind, maxRangeM]) => ({
      kind: kind as Capability["kind"],
      maxRangeM: maxRangeM as number,
      shortRangeM: (maxRangeM as number) / 2,
    }));
}

/**
 * A Force Element from a placed platform.
 *
 * Combat Strength is seeded from the profile's relative strength index, which
 * is explicitly a calibration aid rather than a CS — but it does at least
 * order a tank above a truck, which is the property the board needs to look
 * sane while the real chart is missing.
 */
export function toForceElement(unit: PlacedUnit, sidc: string): ForceElement {
  const strength = Math.max(1, Math.round(unit.platform.csIndex ?? 1));
  return {
    id: unit.id,
    side: unit.side,
    label: unit.platform.displayName,
    sidc,
    moveType: unit.platform.moveType,
    targetClass: unit.platform.targetClass,
    capabilities: capabilitiesOf(unit.platform),
    troopQuality: PROVISIONAL_TROOP_QUALITY,
    combatStrength: strength,
    combatStrengthStart: strength,
    morale: "good",
    markers: [],
    concealed: false,
    isDummy: false,
    position: unit.position,
  };
}

export function toGameState(
  units: readonly PlacedUnit[],
  sidcFor: (unit: PlacedUnit) => string,
  gameId = "spike",
): GameState {
  const forceElements: Record<string, ForceElement> = {};
  for (const unit of units) {
    forceElements[unit.id] = toForceElement(unit, sidcFor(unit));
  }
  return {
    gameId,
    scenarioId: "spike",
    turn: 1,
    phase: "preparation",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements,
    sighting: { blue: {}, red: {} },
    rng: { seed: gameId, cursor: 0 },
  };
}

/**
 * Set sighting from line of sight alone.
 *
 * ⚠ THIS IS NOT THE SIGHTING TEST. BGWS resolves sighting with 2D6 against
 * Player Aid 6, with modifiers for range, movement, cover and smoke, and it
 * produces three degrees of contact. This produces two, from geometry only:
 * if an FE can be seen, it is fully sighted.
 *
 * It is here so the fog-of-war projection has something real to filter on, and
 * so a player can watch contacts appear and disappear as units move behind
 * woodland — which is the behaviour the real test will refine, not replace.
 */
export function sightingFromLineOfSight(
  state: GameState,
  terrain: TerrainSampler,
): GameState {
  const sighting: Record<Side, Record<string, SightingLevel>> = { blue: {}, red: {} };

  for (const viewer of ["blue", "red"] as const) {
    const observers = Object.values(state.forceElements).filter(
      (fe) => fe.side === viewer && fe.combatStrength > 0,
    );
    const targets = Object.values(state.forceElements).filter(
      (fe) => fe.side === opposing(viewer) && fe.combatStrength > 0,
    );

    for (const target of targets) {
      const seen = observers.some((observer) => {
        const counters = Object.values(state.forceElements).filter(
          (fe) => fe.id !== observer.id && fe.id !== target.id,
        );
        return lineOfSight(terrain, {
          from: observer.position,
          to: target.position,
          counters: counters.map((fe) => ({ position: fe.position })),
        }).visible;
      });
      sighting[viewer][target.id] = seen ? "full" : "none";
    }
  }

  return { ...state, sighting };
}

export interface SightLine {
  targetId: string;
  to: LatLng;
  visible: boolean;
  reason: string;
  rangeM: number;
}

/** Every sight line from one FE to the opposing force, for drawing. */
export function sightLinesFrom(
  state: GameState,
  observerId: string,
  terrain: TerrainSampler,
): SightLine[] {
  const observer = state.forceElements[observerId];
  if (!observer) return [];

  return Object.values(state.forceElements)
    .filter((fe) => fe.side !== observer.side && fe.combatStrength > 0)
    .map((target) => {
      const counters = Object.values(state.forceElements)
        .filter((fe) => fe.id !== observer.id && fe.id !== target.id)
        .map((fe) => ({ position: fe.position }));
      const result = lineOfSight(terrain, {
        from: observer.position,
        to: target.position,
        counters,
      });
      return {
        targetId: target.id,
        to: target.position,
        visible: result.visible,
        reason: result.reason,
        rangeM: Math.round(result.rangeM),
      };
    });
}
