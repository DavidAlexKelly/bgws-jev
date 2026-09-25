// ── bgws/lib/state.ts ──────────────────────────────────────────────────────
// The game's state, in BGWS's own vocabulary.
//
// Pure data. No React, no map, no Foundry, no dice — the modules that resolve
// things take a state and return a state plus a log, so a game can be replayed
// from a seed and a list of intents.
//
// WHAT IS AND IS NOT MODELLED YET
// -------------------------------
// Everything here is either in the Core Rulebook in words, or is runtime
// bookkeeping. Nothing here encodes a NUMBER from a resolution table, because
// those live on Player Aids 1-7 and are not in the rulebook. Combat Strength,
// Troop Quality and the markers exist as fields; what a die roll does to them
// does not exist yet, and should arrive as tables rather than as code.

import type { CapabilityClass, MoveType, TargetClass } from "../data/profiles";
import type { LatLng } from "./board";
import type { PlannedRoute } from "./routePlan";

export type Side = "blue" | "red";

export function opposing(side: Side): Side {
  return side === "blue" ? "red" : "blue";
}

/** BGWS 2.1.10. One step per hit, or per 'S' result. */
export type Morale = "good" | "suppressed1" | "suppressed2" | "disrupted" | "broken";

export const MORALE_LADDER: readonly Morale[] = [
  "good",
  "suppressed1",
  "suppressed2",
  "disrupted",
  "broken",
];

/** One step down the ladder, stopping at broken. */
export function degradeMorale(morale: Morale, steps = 1): Morale {
  const index = MORALE_LADDER.indexOf(morale);
  const next = Math.min(MORALE_LADDER.length - 1, index + Math.max(0, steps));
  return MORALE_LADDER[next];
}

/** One step up, stopping at good. Rally is the only thing that does this. */
export function improveMorale(morale: Morale, steps = 1): Morale {
  const index = MORALE_LADDER.indexOf(morale);
  const next = Math.max(0, index - Math.max(0, steps));
  return MORALE_LADDER[next];
}

/**
 * Markers a Force Element carries within a turn. They gate what it may do
 * next, which is why they are a set on the FE rather than a log entry.
 */
export type Marker =
  | "moved"
  | "fired"
  /**
   * Reorganising after an assault (7.1.2, 9.3.10). May only DirF and Attempt
   * Sighting; "This requires a full subsequent Turn."
   */
  | "reorg"
  /**
   * Placed alongside `reorg` on the turn the assault happened.
   *
   * Clean-up (8.0) keeps a REORG placed THIS turn and removes one placed
   * LAST turn — so the marker has to carry which of those it is, and a
   * marker set is the only per-element memory that survives to clean-up.
   */
  | "reorgPlacedThisTurn"
  /**
   * Locked in close combat (9.3.8). "The MELEE marker is not removed at the
   * end of the Turn" — the one marker that outlives its turn.
   */
  | "melee"
  /** Used its activation this turn. Counted against command capacity. */
  | "activated"
  /** Chose to do nothing. NOT the same as having moved. */
  | "held"
  /**
   * Has Reactive Fired this turn (Core Rules 7.1.3).
   *
   * The rulebook's cost for reacting is severe and is the whole balance of the
   * mechanic: "The FE/Group that has Reactive Fired cannot take any further
   * Action for the remainder of the Turn." That is enforced by the FIRED
   * marker this is always set alongside; `reacted` exists so the log and the
   * caps can tell a reaction apart from a deliberate shot.
   */
  | "reacted"
  /** Has taken its Counteraction Round DirF (7.2.2). One per element, per turn. */
  | "counteracted"
  /**
   * Nominated as Reserve for this turn (2.1.13).
   *
   * A marker rather than a field on the FE because Reserve is an ORDER that a
   * side re-declares each turn in the Command Sub-phase, not a property of the
   * equipment. Clean-up clears it, which is correct: next turn's reserve is
   * next turn's decision.
   *
   * "Only one-third of FE/Groups in a side may be given a Reserve Order."
   */
  | "reserve"
  /** Has taken its Reserve Move in the Counteraction Round (7.2.1). */
  | "reserveMoved";

/**
 * How well one side can see a Force Element belonging to the other.
 *
 * Not a boolean, because BGWS distinguishes a partial sighting — enough to
 * call indirect fire onto, not enough to engage directly — from a full one.
 */
export type SightingLevel = "none" | "veryPartial" | "partial" | "full";

/**
 * How a round defeats armour, which decides two different things.
 *
 * `ke`  A kinetic penetrator. Loses penetration with range, and is resisted
 *       by the kinetic figure.
 * `ce`  A shaped charge. DOES NOT LOSE PENETRATION WITH RANGE — the jet is
 *       formed on impact — and is resisted by the chemical-energy figure,
 *       which composite and reactive armour raise well above the kinetic one.
 * `ceTandem`  A tandem shaped charge, whose precursor strips reactive armour
 *       before the main jet arrives. Resisted by the chemical figure reduced
 *       by PenetrationRule.tandemEraDefeatFraction where ERA is fitted.
 */
export type MunitionKind = "ke" | "ce" | "ceTandem";

/**
 * Armour by the aspect a round arrives from.
 *
 * ⚠ THE ROOF IS WHY THIS EXISTS. It is 20-30 mm on vehicles carrying 700 mm
 * at the front, so a top-attack missile is not fighting the same tank a sabot
 * round is. Holding one frontal number made every shot a frontal shot, which
 * silently deleted both flanking and top attack as tactics.
 *
 * Kinetic and chemical resistance are separate because they diverge: a
 * Challenger 2's side is 140 mm against a sabot round and 400 mm against a
 * shaped charge.
 */
export interface ArmourByAspect {
  frontKeMm?: number;
  frontCeMm?: number;
  sideKeMm?: number;
  sideCeMm?: number;
  rearKeMm?: number;
  rearCeMm?: number;
  roofKeMm?: number;
  roofCeMm?: number;
}

export interface Capability {
  kind: CapabilityClass;
  /**
   * How this capability's round defeats armour.
   *
   * Undefined behaves as `ke`, which is what the engine assumed for
   * everything before munitions were modelled.
   */
  munition?: MunitionKind;
  /** Attacks the roof rather than the aspect it is fired from. */
  topAttack?: boolean;
  /** Metres. Direct fire is capped at the 3 km line-of-sight limit regardless. */
  maxRangeM: number;
  /** Under 51% of Max Range is short range (BGWS 2.1.8). */
  shortRangeM: number;
  /**
   * Combat Strength modifiers at short and long range.
   *
   * Populated from a scenario's Force Management Chart. The equipment profiles
   * can suggest the long-range one from a round's penetration falloff, but the
   * authoritative values are the chart's.
   */
  csShort?: number;
  csLong?: number;
  /**
   * Armour this capability defeats, in millimetres at 1 km.
   *
   * UNDEFINED MEANS UNKNOWN, NOT ZERO, and the difference is load-bearing.
   * The source has no penetration curve for a large part of the catalogue —
   * Challenger 2 and Type 59 among them, both marked
   * `fragmentation_not_penetration` upstream. Reading absent as "cannot
   * penetrate" would quietly render a Challenger 2 unable to damage a
   * Warrior. See PenetrationRule for how that is handled.
   */
  penetrationMm?: number;
}

export interface ForceElement {
  id: string;
  side: Side;
  /** Task group. Re-assignable in the Command sub-phase. */
  groupId?: string;
  label: string;
  /** Rendered by milsymbol. Comes from the ORBAT, not from the equipment. */
  sidc: string;

  moveType: MoveType;
  /**
   * A march this element is committed to, spent a turn at a time.
   *
   * State rather than a per-turn decision because a route round a lake only
   * means anything if it is still being followed three turns later. Cleared on
   * contact: a march is a plan made in the absence of the enemy. See
   * lib/routePlan.ts.
   */
  route?: PlannedRoute;
  /** Which capability class may engage this FE at all (BGWS 2.1.8). */
  targetClass: TargetClass;
  capabilities: Capability[];

  /** 1-8. Training and cohesion; from the scenario, never from equipment. */
  troopQuality: number;
  /**
   * Frontal armour in millimetres, for the penetration check.
   *
   * Undefined is unknown rather than unarmoured, same as penetrationMm.
   *
   * KEPT AS THE FALLBACK. `armour` below supersedes it where the profile
   * carries facings; this remains for elements built from the L6 profile,
   * which has only a frontal figure, and for hand-declared platforms.
   */
  armourMm?: number;
  /**
   * Armour by aspect, where the profile carries it.
   *
   * When present the penetration check uses the aspect the shot arrived
   * from, so manoeuvring onto a flank changes what happens rather than only
   * shifting the dice.
   */
  armour?: ArmourByAspect;
  /**
   * Explosive reactive armour is fitted.
   *
   * Only tandem warheads care: their precursor strips it. Without this a
   * Kornet and an RPG-7 would meet the same protection, which is the whole
   * reason tandem warheads were built.
   */
  eraFitted?: boolean;
  /** Hits reduce it. Zero is Eliminated. */
  combatStrength: number;
  combatStrengthStart: number;
  /** HQs only: how many subordinate activations it can command. */
  commandRating?: number;

  morale: Morale;
  markers: Marker[];
  /** Rounds remaining, by capability class. Absent means not tracked. */
  ammo?: Partial<Record<CapabilityClass, number>>;

  /** Concealed FEs are not on the opposing player's map at all. */
  concealed: boolean;
  /** A Dummy looks exactly like an FE until something makes it reveal. */
  isDummy: boolean;
  /** Mounted in another FE (a section in its carrier). */
  mountedIn?: string;

  position: LatLng;
  /**
   * Which way it is pointing, in degrees clockwise from north.
   *
   * Set from the bearing of its last move. UNDEFINED MEANS "FACING THE
   * THREAT": an element that has not moved is assumed to be oriented on its
   * arc, so it cannot be flanked by someone who simply walked around a
   * stationary tank that was watching them the whole time. Aspect is
   * something you lose by manoeuvring, which is the trade the rule is for.
   */
  facing?: number;
  /** Height above ground of the observer, for line of sight. */
  observerHeightM?: number;
}

export function hasMarker(fe: ForceElement, marker: Marker): boolean {
  return fe.markers.includes(marker);
}

export function withMarker(fe: ForceElement, marker: Marker): ForceElement {
  return hasMarker(fe, marker) ? fe : { ...fe, markers: [...fe.markers, marker] };
}

/** Clean-up removes every marker. Turn boundaries are the only thing that does. */
export function clearMarkers(fe: ForceElement): ForceElement {
  return { ...fe, markers: [] };
}

export function isEliminated(fe: ForceElement): boolean {
  return fe.combatStrength <= 0;
}

/**
 * The sequence of play (Core Rules 2.3, 5.0-8.0). Preparation happens once;
 * the rest repeat, and one completed Execution Phase is one Turn.
 *
 * ARC (7.0) IS TWO ROUNDS, AND THEY ARE DIFFERENT GAMES.
 *
 *   Action-Reaction (7.1)  Both sides alternate Activating FEs, Initiative
 *                          side first. An FE Activates ONCE. While a side is
 *                          activating, the other may not act except to
 *                          Attempt Sighting, Reactive Fire (7.1.3) or
 *                          Defensive Fire.
 *
 *   Counteraction (7.2)    Alternating again, Initiative side first, but only
 *                          two things are possible: Reserve Movement (7.2.1)
 *                          by FEs with a Reserve Order and no FIRED marker,
 *                          who may then DirF or Hasty Assault; and then
 *                          Counteraction Fire (7.2.2), in which ANY FE
 *                          without a FIRED marker may DirF, at a penalty.
 *                          Passing is final for the Turn.
 *
 * `arcReaction` is therefore not a round of its own — it is the phase a
 * Reactive Fire resolution is logged under, so an interrupting shot can be
 * told apart from the action it interrupted. Both rounds are entered by both
 * sequences of play (rules/turnLoop.ts and rules/orders.ts).
 */
export type Phase =
  | "preparation"
  | "command"
  | "initiative"
  | "arcAction"
  /** The opposing side answering an action. The R in ARC. */
  | "arcReaction"
  | "arcCounteraction"
  | "cleanup";

export const TURN_PHASES: readonly Phase[] = [
  "command",
  "initiative",
  "arcAction",
  "arcReaction",
  "arcCounteraction",
  "cleanup",
];

export interface SideState {
  /** Radio transmissions this turn. Drives EW chits and initiative. */
  transmissions: number;
  transmissionsLastTurn: number;
  /** EW chits drawn and held rather than played immediately. */
  chitsHeld: number;
  /** FEs eliminated last turn — an initiative modifier. */
  eliminatedLastTurn: number;
}

/**
 * A cloud of smoke on the map (9.2.2.4).
 *
 * Placed on an AREA rather than on a Force Element — "A SMOKE marker is
 * placed on an area location on the map – rather than an FE" — which is why
 * this is a list on the game state and not a marker on a counter. Everything
 * within 250 m of it is affected, whoever it belongs to.
 */
export interface SmokeMarker {
  id: string;
  position: LatLng;
  /** The turn it was fired. Clean-up removes it at the end of that turn. */
  placedTurn: number;
}

export interface GameState {
  gameId: string;
  scenarioId: string;
  turn: number;
  phase: Phase;
  /** Null until the Initiative test has been taken this turn. */
  initiative: Side | null;
  sides: Record<Side, SideState>;
  forceElements: Record<string, ForceElement>;
  /** How each side currently sees each of the other's FEs. */
  sighting: Record<Side, Record<string, SightingLevel>>;
  /**
   * Where each side is trying to get to.
   *
   * SCENARIO knowledge, not intelligence — it is where you were ordered to go,
   * and it is legitimate to know it with no enemy in sight.
   *
   * Without this, movement could only be offered towards a SIGHTED contact,
   * and sighting fails at long range. A force deployed 5 km apart therefore
   * could never advance to contact at all: 40 turns of both sides holding
   * position, every game a draw. An army that cannot advance without already
   * seeing the enemy is not modelling war, it is modelling a firing range.
   */
  objectives?: Record<Side, { lat: number; lng: number }>;
  /** Smoke on the map (9.2.2.4). Absent is the same as empty. */
  smoke?: SmokeMarker[];
  /** Seeded, so a game replays exactly from (seed, intents). */
  rng: { seed: string; cursor: number };
}

export function forceElementsOf(state: GameState, side: Side): ForceElement[] {
  return Object.values(state.forceElements).filter((fe) => fe.side === side);
}

export function sightingOf(
  state: GameState,
  viewer: Side,
  feId: string,
): SightingLevel {
  return state.sighting[viewer]?.[feId] ?? "none";
}
