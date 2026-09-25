// ── bgws/lib/forceBuilder.ts ───────────────────────────────────────────────
// A force list into a GameState the rules can play.
//
// This is the piece that was missing. The rules engine was complete, the
// equipment profiles were real, and between them sat spikeGame.toForceElement
// — which took ONE platform, gave it a hardcoded Troop Quality of 4 and
// flattened every weapon range to 3,000 m. So the harness could measure a
// mechanic perfectly and still be telling you about a fiction.
//
// What this does differently, and all three matter:
//
//   SUB-UNIT COMBAT STRENGTH.  A troop of four Challengers is not one tank.
//   Combat Strength is the per-platform index times the platform count, which
//   is the granularity decision the project actually took.
//
//   REAL WEAPON RANGES.  Per capability, from the L6 capability profile. A
//   Challenger's coax reaches 2,000 m and its gun 3,000; a T-72's coax 3,000.
//   Under the old flat 3,000 m those differences did not exist, so no rule
//   about range could possibly have shown an effect.
//
//   DECLARED TROOP QUALITY.  Whatever the force list says, because nothing in
//   a JSON file about a tank knows how good its crew is.
//
// Pure. No Foundry call, so a batch can run offline and in CI.

import type { LatLng } from "./board";
import type { ForceElement, GameState, Side } from "./state";
import {
  PLATFORM_SNAPSHOT,
  TROOP_QUALITY,
  type ForceElementSpec,
  type ForceList,
  type PlatformSnapshot,
  type TroopQualityName,
} from "../rules/forceList";
import {
  combatStrengthFor as rulesetCombatStrength,
  type RuleSet,
} from "../rules/ruleset";

/**
 * Where a scenario's origin sits.
 *
 * Arbitrary but fixed: northern Poland, which is where the Baltic Shield
 * scenario data in this repo lives. It only matters that it is consistent,
 * because line of sight works in metres.
 */
export const DEFAULT_ORIGIN: LatLng = { lat: 54.2, lng: 18.6 };

const METRES_PER_DEGREE_LAT = 111_320;

/**
 * A metre offset into a position.
 *
 * Flat-earth approximation, which is correct to well under a metre over the
 * few kilometres a BGWS board spans, and wrong enough to matter over a
 * continent. Nothing here spans a continent.
 */
export function offsetToLatLng(origin: LatLng, eastM: number, northM: number): LatLng {
  const latitude = origin.lat + northM / METRES_PER_DEGREE_LAT;
  const metresPerDegreeLng = METRES_PER_DEGREE_LAT * Math.cos((origin.lat * Math.PI) / 180);
  return {
    lat: latitude,
    lng: origin.lng + eastM / metresPerDegreeLng,
  };
}

/**
 * A generic symbol, used only when the force list does not name one.
 *
 * SIDCs belong to the ORBAT, not to equipment — state.ts says so on the field
 * itself. This is a placeholder so a force list can be written without one,
 * not an attempt to derive symbology from a tank's stats.
 */
function defaultSidc(side: Side): string {
  return side === "blue" ? "10031000141211000000" : "10061000141211000000";
}

/**
 * Combat Strength for a sub-unit.
 *
 * Delegates to the RULESET, which owns the mapping. This function briefly
 * owned it instead, as `csIndex * platformCount`, and the result was a total
 * silent failure — see CombatStrengthRule in rules/ruleset.ts. A number that
 * decides outcomes does not belong in a builder.
 */
export function combatStrengthFor(
  ruleset: RuleSet,
  platform: PlatformSnapshot,
  platformCount: number,
): number {
  return rulesetCombatStrength(
    ruleset,
    platform.csIndex,
    platformCount,
    platform.protectionBand,
  );
}

export function toForceElement(
  spec: ForceElementSpec,
  origin: LatLng,
  ruleset: RuleSet,
): ForceElement {
  const platform = PLATFORM_SNAPSHOT[spec.platform];
  if (!platform) {
    // Loud, because the alternative is an FE with no capabilities that loses
    // every engagement for a reason nobody would find.
    throw new Error(
      `Force list references platform "${spec.platform}", which is not in ` +
        `PLATFORM_SNAPSHOT. Add it, with its figures and the date they were read.`,
    );
  }

  const strength = combatStrengthFor(ruleset, platform, spec.platformCount);

  return {
    id: spec.id,
    side: spec.side,
    label: spec.label,
    sidc: defaultSidc(spec.side),
    moveType: platform.moveType,
    targetClass: platform.targetClass,
    // Copied, not shared: the snapshot is module-level and a game must not be
    // able to mutate the next game's equipment.
    capabilities: platform.capabilities.map((capability) => ({ ...capability })),
    armourMm: platform.armourMm,
    armour: platform.armour,
    eraFitted: platform.eraFitted,
    troopQuality: TROOP_QUALITY[spec.troopQuality],
    combatStrength: strength,
    combatStrengthStart: strength,
    morale: "good",
    markers: [],
    concealed: spec.concealed ?? false,
    commandRating: spec.commandRating,
    isDummy: spec.isDummy ?? false,
    position: offsetToLatLng(
      origin,
      spec.offsetM.east,
      spec.offsetM.north,
    ),
  };
}

export interface BuildOptions {
  origin?: LatLng;
  gameId?: string;
  /**
   * Swap which side deploys where.
   *
   * ⚠ THIS IS HOW YOU TELL A GROUND BIAS FROM A RULES BIAS, AND THERE IS NO
   * OTHER WAY. A "symmetric" force list is only symmetric in its FORCES: both
   * sides field the same troops, but they fight over different ground, and on
   * the standard ground one approach is wetter, more wooded or more overlooked
   * than the other. So a symmetric list coming out 28/71 says nothing by
   * itself — it could be a side-biased rule, or it could be the map.
   *
   * Play both orientations and the two explanations separate cleanly: if the
   * advantage follows the GROUND it swaps with the deployments, and if it
   * follows a RULE it stays with the colour.
   */
  mirrored?: boolean;
}

/**
 * A fresh GameState from a force list.
 *
 * The ruleset is REQUIRED, not defaulted, because it decides Combat Strength.
 * Defaulting it would let a caller build a force under one set of rules and
 * play it under another, and the mismatch would look like a balance problem.
 */
export function toGameState(
  list: ForceList,
  ruleset: RuleSet,
  options: BuildOptions = {},
): GameState {
  const origin = options.origin ?? DEFAULT_ORIGIN;
  const forceElements: Record<string, ForceElement> = {};

  // Mirrored: each side deploys where the other would have. The forces are
  // unchanged, only the ground they start on and advance over.
  const deploymentFor = (side: Side) =>
    list.deployment[options.mirrored ? (side === "blue" ? "red" : "blue") : side];

  for (const spec of list.elements) {
    const deployment = deploymentFor(spec.side);
    const placed: ForceElementSpec = {
      ...spec,
      offsetM: {
        east: deployment.east + spec.offsetM.east,
        north: deployment.north + spec.offsetM.north,
      },
    };
    forceElements[spec.id] = toForceElement(placed, origin, ruleset);
  }

  const gameId = options.gameId ?? list.id;

  return {
    gameId,
    scenarioId: list.id,
    turn: 1,
    phase: "preparation",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements,
    sighting: { blue: {}, red: {} },
    // Each side advances on where the other deployed. Scenario knowledge, and
    // the reason a force can move before it can see anything.
    objectives: {
      blue: offsetToLatLng(origin, deploymentFor("red").east, deploymentFor("red").north),
      red: offsetToLatLng(origin, deploymentFor("blue").east, deploymentFor("blue").north),
    },
    rng: { seed: gameId, cursor: 0 },
  };
}

/**
 * A scenario factory for the harness, which needs a FRESH state per game.
 *
 * Returning the same object would let game two inherit game one's casualties,
 * and a batch would report a steadily collapsing force as a finding about the
 * rules.
 */
export function scenarioFactory(
  list: ForceList,
  ruleset: RuleSet,
  options: BuildOptions = {},
): () => GameState {
  return () => toGameState(list, ruleset, options);
}

/**
 * An element placed by hand, at a position rather than an offset.
 *
 * The force lists express position as a metre offset from a deployment point,
 * which is right for a repeatable experiment. A player dropping counters on a
 * map has a latitude and longitude and no deployment point, so this is the
 * same element with the position already resolved.
 */
export interface PlacedElement {
  id: string;
  label: string;
  side: Side;
  /** Key into PLATFORM_SNAPSHOT. */
  platform: string;
  platformCount: number;
  troopQuality: TroopQualityName;
  position: LatLng;
  concealed?: boolean;
  commandRating?: number;
  isDummy?: boolean;
}

/** A GameState from hand-placed elements. */
export function toGameStateFromPlaced(
  placed: readonly PlacedElement[],
  ruleset: RuleSet,
  options: BuildOptions & { objectives?: Record<Side, LatLng> } = {},
): GameState {
  const forceElements: Record<string, ForceElement> = {};

  for (const element of placed) {
    // Reuse toForceElement by handing it a zero offset and the real position
    // as the origin, so Combat Strength, capabilities and troop quality all
    // come from exactly the same code the force lists use. A second builder
    // would be a second place for the protection bonus to be forgotten.
    forceElements[element.id] = toForceElement(
      {
        id: element.id,
        label: element.label,
        side: element.side,
        platform: element.platform,
        platformCount: element.platformCount,
        troopQuality: element.troopQuality,
        offsetM: { east: 0, north: 0 },
        concealed: element.concealed,
        commandRating: element.commandRating,
        isDummy: element.isDummy,
      },
      element.position,
      ruleset,
    );
  }

  const gameId = options.gameId ?? "placed";

  return {
    gameId,
    scenarioId: "hand-placed",
    turn: 1,
    phase: "preparation",
    initiative: null,
    sides: {
      blue: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
      red: { transmissions: 0, transmissionsLastTurn: 0, chitsHeld: 0, eliminatedLastTurn: 0 },
    },
    forceElements,
    sighting: { blue: {}, red: {} },
    // Each side advances on the centre of mass of the other's deployment,
    // which is what they would have been briefed. Without an objective a
    // force that has sighted nothing cannot advance at all.
    objectives: options.objectives ?? centresOfMass(placed),
    rng: { seed: gameId, cursor: 0 },
  };
}

function centresOfMass(placed: readonly PlacedElement[]): Record<Side, LatLng> {
  const mean = (side: Side): LatLng => {
    const mine = placed.filter((element) => element.side === side);
    if (mine.length === 0) return DEFAULT_ORIGIN;
    return {
      lat: mine.reduce((sum, e) => sum + e.position.lat, 0) / mine.length,
      lng: mine.reduce((sum, e) => sum + e.position.lng, 0) / mine.length,
    };
  };
  // Each side's objective is where the OTHER side is.
  return { blue: mean("red"), red: mean("blue") };
}

/** Starting Combat Strength of hand-placed elements, per side. */
export function placedStrength(
  placed: readonly PlacedElement[],
  ruleset: RuleSet,
): Record<Side, number> {
  const totals: Record<Side, number> = { blue: 0, red: 0 };
  for (const element of placed) {
    const platform = PLATFORM_SNAPSHOT[element.platform];
    if (!platform) continue;
    totals[element.side] += combatStrengthFor(ruleset, platform, element.platformCount);
  }
  return totals;
}

/** Turn a force list into hand-placed elements, as a starting point to edit. */
export function placedFromList(list: ForceList): PlacedElement[] {
  return list.elements.map((spec) => {
    const deployment = list.deployment[spec.side];
    return {
      id: spec.id,
      label: spec.label,
      side: spec.side,
      platform: spec.platform,
      platformCount: spec.platformCount,
      troopQuality: spec.troopQuality,
      position: offsetToLatLng(
        DEFAULT_ORIGIN,
        deployment.east + spec.offsetM.east,
        deployment.north + spec.offsetM.north,
      ),
      concealed: spec.concealed,
      commandRating: spec.commandRating,
      isDummy: spec.isDummy,
    };
  });
}

/** Starting Combat Strength per side — for reporting a batch honestly. */
export function startingStrength(list: ForceList, ruleset: RuleSet): Record<Side, number> {
  const totals: Record<Side, number> = { blue: 0, red: 0 };
  for (const spec of list.elements) {
    const platform = PLATFORM_SNAPSHOT[spec.platform];
    if (!platform) continue;
    totals[spec.side] += combatStrengthFor(ruleset, platform, spec.platformCount);
  }
  return totals;
}

/** Every proxy in use, so a report can name them rather than imply accuracy. */
export function proxiesIn(list: ForceList): { label: string; proxyFor: string }[] {
  const proxies: { label: string; proxyFor: string }[] = [];
  for (const spec of list.elements) {
    const platform = PLATFORM_SNAPSHOT[spec.platform];
    if (platform?.proxyFor) {
      proxies.push({ label: `${spec.label} (${platform.displayName})`, proxyFor: platform.proxyFor });
    }
  }
  return proxies;
}
