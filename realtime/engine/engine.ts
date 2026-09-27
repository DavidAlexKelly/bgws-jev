// ── bgws/realtime/engine/engine.ts ─────────────────────────────────────────
// One simulated second at a time: the autopilot.
//
// `tick` is pure. Given the board and the rules it returns the next board,
// the events that happened to units, and the shots that were fired. It never
// decides anything a commander would — every steady unit does what its
// current order says — so a decider can be swapped, faked or left out
// without the engine noticing.
//
// What the autopilot DOES decide is what a crew does without being told
// (docs/REALTIME_REALISM.md): the react-to-contact drill, returning fire,
// how it moves in each movement mode, and everything a shaken or broken unit
// does until it rallies. Those take seconds; Jev's answer takes longer.
//
// Each tick, in order:
//   1. contact reports reach the rest of the side
//   2. movement, simultaneous, by movement mode (bounding pairs alternate);
//      pinned units cannot advance; arriving and getting stuck are events
//   3. running into the enemy: halt (or, in an assault, close to 150 m) and
//      run the react-to-contact drill
//   4. sighting: a chance per second of spotting each enemy in sight
//      (detection.ts), tracking once found; a unit knows what it saw at once,
//      its side after `reportDelayS`; faded contacts leave a last-known position
//   5. fire, simultaneous: each weapon at its own rate of fire, targets by threat,
//      self-defence always allowed unless the ROE is "never"
//   6. what the fire did: rounds on target, penetration, knock-outs, suppression,
//      the drill for a new attacker
//   7. suppression fades; posture
//   8. break tests at loss thresholds and when pinned; broken units fall back
//      once to a rally point
//   9. rally checks
//  10. idle units, off their mission, are asked again
//  11. is a side past its breakpoint? has time run out?
//
// Everything random comes from `config.rng`, in a fixed order, so the same
// seed and the same decisions give the same game.

import { bearingDeg, bearingDeltaDeg, distanceM, LOS_CAP_M, type LatLng } from "../../lib/board";
import { lineOfSight } from "../../lib/lineOfSight";
import { inCover } from "../../lib/proceduralTerrain";
import {
  forceElementsOf,
  opposing,
  sightingOf,
  type ForceElement,
  type GameState,
  type Morale,
  type Side,
  type SightingLevel,
} from "../../lib/state";
import { applyEffects } from "../../rules/apply";
import { canAdvance } from "../../rules/resolvers";
import { isFlankShot, weaponFor } from "../../rules/turnLoop";
import { judgeVictory } from "../../rules/victory";
import { bearingRoutePlanner } from "../../lib/routePlan";
import { IDENTIFY_M, detectChance } from "./detection";
import { acquisitionS, aimedIntervalS, hitChance, type HitConditions } from "./fire";
import {
  allowanceAt,
  compass,
  hullDownAgainst,
  hullDownSpot,
  offsetBy,
  platformSpeedFactor,
  slopeFactor,
  towards,
} from "./geometry";
import { aspectOf, describeStrike, rollStrike, strikeOdds, type StrikeResult } from "./lethality";
import {
  ASSAULT_CONTACT_M,
  ATTACKER_BREAK_AT,
  BOUND_COVER_S,
  BOUND_M,
  BREAK_STEP,
  CLOSE_CONTACT_M,
  DEFENDER_BREAK_AT,
  DRILL_COVER_M,
  HISTORY_LENGTH,
  HQ_RADIUS_M,
  PINNED_AT,
  PINNED_TEST_S,
  REVIEW_S,
  SETTLE_S,
  SIDE_BREAKPOINT,
  SUPPRESSED_AT,
  SUPPRESSION_DECAY_PER_S,
  SUPPRESSION_GRACE_S,
} from "./timing";
import {
  CUE_INTERVAL_S,
  CUE_RANGE_M,
  HEARD_M,
  SHOOTER_MOVED_M,
  compassWord,
  cueOf,
  locateChance,
  tryingToHide,
} from "./knowledge";
import type {
  Boundary,
  Cohesion,
  DecisionMemory,
  MoveMode,
  Posture,
  ReportLevel,
  RtConfig,
  RtEvent,
  RtOrder,
  RtShot,
  RtState,
  RtUnit,
  Trigger,
} from "./types";

const SIGHT_RANK: Record<SightingLevel, number> = { none: 0, veryPartial: 1, partial: 2, full: 3 };
/** How long "fired upon" lasts, for the ifFiredUpon rule and self-defence. */
const FIRED_UPON_MEMORY_S = 60;
/** Who counts as "close by" when a friend is lost. */
const FRIEND_RADIUS_M = 1000;
/** Look again this often when a unit's weapon is ready and there is nothing to shoot. */
const RETARGET_S = 5;
/** The drill's return fire: a weapon that was not ready is ready this soon. */
const RETURN_FIRE_S = 5;
/** A unit on a "take" mission that moved this recently is still the attacker. */
const ATTACKING_S = 120;
/** No incoming fire for this long before a rally can be tried. */
const QUIET_S = 30;
/** A knocked-out vehicle still draws fire for this long, until it is seen to be dead. DECLARED. */
const WRECK_DRAWS_FIRE_S = 15;
/** A friend's loss shakes nerves for this long. */
const FRIEND_LOST_S = 60;
/** How far a broken unit will go to reach its rally point, and how far if there is none. */
const RALLY_SEARCH_M = 2500;
const FALL_BACK_M = 800;
/** On its ground: close enough to the objective, or to the ground it holds. */
const ON_OBJECTIVE_M = 300;
const ON_POSITION_M = 150;
/** Break-test and rally scores: d6 + TQ/2 + modifiers. */
const BREAK_PASS = 6;
const BREAK_SHAKEN = 4;
const RALLY_PASS = 7;

/** Speed as a fraction of the ground's, by how the unit is moving. */
const SPEED: Record<MoveMode | "withdraw", number> = {
  march: 1,
  tactical: 0.6,
  assault: 0.8,
  bound: 0.8,
  withdraw: 1,
};

/** A search covers this many degrees either side of its bearing. */
export const SEARCH_ARC_DEG = 45;
/** Friends within this are the ones a unit can help (D9). */
const HELP_RADIUS_M = 1500;
/** A unit is offered the chance to help the same friend at most this often. */
const HELP_INTERVAL_S = 60;

/** Suppression one shot adds, before cover and quality. */
const SUPPRESSION_FOR = { miss: 8, suppress: 18, hit: 30, damaged: 15 };

export interface TickResult {
  state: RtState;
  events: RtEvent[];
  shots: RtShot[];
}

/** "3:05" for a span of seconds. */
function clockOf(seconds: number): string {
  const whole = Math.max(0, Math.round(seconds));
  return `${Math.floor(whole / 60)}:${String(whole % 60).padStart(2, "0")}`;
}

function higher<T extends SightingLevel>(a: T, b: T): T {
  return SIGHT_RANK[a] >= SIGHT_RANK[b] ? a : b;
}

export function isAlive(game: GameState, id: string): boolean {
  return (game.forceElements[id]?.combatStrength ?? 0) > 0;
}

/** Still in the fight: alive and not broken. */
export function isFighting(fe: ForceElement): boolean {
  return fe.combatStrength > 0 && fe.morale !== "broken";
}

/**
 * The turn game's morale, from cohesion and suppression. Written onto each
 * element every tick so the shared fire table's modifiers — a suppressed
 * firer shoots worse — and `canAdvance` see it.
 */
export function derivedMorale(unit: Pick<RtUnit, "cohesion" | "suppression">): Morale {
  if (unit.cohesion === "broken") return "broken";
  if (unit.cohesion === "shaken") return "disrupted";
  if (unit.suppression >= PINNED_AT) return "suppressed2";
  if (unit.suppression >= SUPPRESSED_AT) return "suppressed1";
  return "good";
}

/** Can this enemy hit that element, from where each of them is? */
export function canHit(enemy: ForceElement, target: ForceElement, at: LatLng, config: RtConfig): boolean {
  const range = distanceM(enemy.position, at);
  if (range > LOS_CAP_M) return false;
  if (!weaponFor(enemy, target, range)) return false;
  return lineOfSight(config.terrain, { from: enemy.position, to: at }).visible;
}

/**
 * What THIS unit knows of an enemy: its side's picture, or better if it has
 * seen the enemy itself and the report has not gone round yet.
 */
export function knownTo(
  game: GameState,
  units: Record<string, RtUnit>,
  id: string,
  enemyId: string,
): SightingLevel {
  const fe = game.forceElements[id];
  if (!fe) return "none";
  const own: SightingLevel = units[id]?.ownSeen[enemyId]?.level ?? "none";
  return higher(sightingOf(game, fe.side, enemyId), own);
}

/** Living enemies this unit knows about. */
export function knownEnemies(state: Pick<RtState, "game" | "units">, id: string): ForceElement[] {
  const fe = state.game.forceElements[id];
  if (!fe) return [];
  return forceElementsOf(state.game, opposing(fe.side)).filter(
    (enemy) => enemy.combatStrength > 0 && knownTo(state.game, state.units, id, enemy.id) !== "none",
  );
}

/** Cover, or hull-down, at a unit's position: what the fire table calls "target in cover". */
export function isCovered(
  state: Pick<RtState, "units">,
  fe: ForceElement,
  config: RtConfig,
  from?: LatLng,
): boolean {
  if (inCover(config.terrain, fe.position)) return true;
  if (from == null) return state.units[fe.id]?.posture === "hullDown";
  return hullDownFrom(state, fe, from, config);
}

/**
 * What degrades a round from `firer` at `target` right now (fire.ts): moving
 * this second, the target moving, cover or hull-down against this firer, and
 * the firer's own suppression and nerve. `from` is where the firer would be.
 */
export function hitConditions(
  state: Pick<RtState, "units" | "time">,
  firer: ForceElement,
  target: ForceElement,
  config: RtConfig,
  from: LatLng = firer.position,
): HitConditions {
  const own = state.units[firer.id];
  const their = state.units[target.id];
  const suppression = own?.suppression ?? 0;
  const hullDown = hullDownFrom(state, target, from, config);
  return {
    aspect: aspectOf(target, from, config.ruleset, { hullDown }),
    firerMoving: from === firer.position && own?.lastMovedAt === state.time,
    targetMoving: their?.lastMovedAt === state.time,
    targetInCover: inCover(config.terrain, target.position),
    targetHullDown: hullDown,
    firerSuppressed: suppression >= SUPPRESSED_AT && suppression < PINNED_AT,
    firerPinned: suppression >= PINNED_AT,
    firerShaken: own?.cohesion === "shaken",
  };
}

/** Hull-down against fire from `from`: halted behind a crest that hides the hull (from the DEM). */
export function hullDownFrom(state: Pick<RtState, "units">, fe: ForceElement, from: LatLng, config: RtConfig): boolean {
  const posture = state.units[fe.id]?.posture;
  if (posture === "moving") return false;
  return hullDownAgainst(fe.position, from, config);
}

/** Identified enemies that can already reach this element where it stands. */
export function exposureOf(state: RtState, id: string, config: RtConfig): string[] {
  const fe = state.game.forceElements[id];
  if (!fe) return [];
  return forceElementsOf(state.game, opposing(fe.side))
    .filter((enemy) => enemy.combatStrength > 0)
    .filter((enemy) => knownTo(state.game, state.units, id, enemy.id) === "full")
    .filter((enemy) => canHit(enemy, fe, fe.position, config))
    .map((enemy) => enemy.id);
}

/** Is the unit doing what it is for? A unit that is not, and is quiet, is asked again. */
export function onMission(unit: RtUnit, fe: ForceElement): boolean {
  const { mission, order } = unit;
  const at = mission.at;
  if (mission.task === "take") return at != null && distanceM(fe.position, at) <= ON_OBJECTIVE_M;
  if (mission.task === "hold") return at == null || distanceM(fe.position, at) <= ON_POSITION_M;
  return order.kind === "overwatch" || order.kind === "engage";
}

function freshUnit(fe: ForceElement): RtUnit {
  return {
    order: { kind: "hold" },
    roe: "withinShortRange",
    mission: { task: "hold", at: fe.position, purpose: "hold this ground" },
    weaponReadyAt: 0,
    lastMovedAt: -Infinity,
    suppression: 0,
    lastIncomingAt: -Infinity,
    pinnedSince: null,
    cohesion: "steady",
    breakTests: 0,
    fellBack: false,
    lastRallyCheckAt: 0,
    attackers: {},
    ownSeen: {},
    lastFriendLostAt: -Infinity,
    posture: "settled",
    lastShotAt: -Infinity,
    lastEventAt: 0,
    exposedTo: [],
    engagement: null,
    incoming: {},
    dealt: 0,
    lastReviewAt: 0,
    history: [],
    vehicles: { total: vehiclesIn(fe), fit: vehiclesIn(fe) },
    losses: [],
    suspects: {},
    locating: {},
    lastCue: null,
    cueAt: {},
    helpAt: {},
    orders: null,
  };
}

/** How many vehicles an element is: its platform count, or, for elements built without one, about one per 2.5 strength. */
export function vehiclesIn(fe: ForceElement): number {
  return Math.max(1, fe.platformCount ?? Math.round(fe.combatStrengthStart / 2.5));
}

/** Combat strength from the vehicles still fit: the fire table's column follows the losses. */
export function strengthFor(fe: ForceElement, vehicles: { total: number; fit: number }): number {
  if (vehicles.fit <= 0) return 0;
  return Math.max(1, Math.round((fe.combatStrengthStart * vehicles.fit) / vehicles.total));
}

/** Add a decision to a unit's memory, keeping the last few. */
export function remember(unit: RtUnit, entry: DecisionMemory): DecisionMemory[] {
  return [...unit.history, entry].slice(-HISTORY_LENGTH);
}

/** A fresh real-time game from a placed board. Everyone holds until ordered. */
export function createRealtimeState(game: GameState): RtState {
  const units: Record<string, RtUnit> = {};
  const forceElements: GameState["forceElements"] = {};
  const startStrength: Record<Side, number> = { blue: 0, red: 0 };
  for (const fe of Object.values(game.forceElements)) {
    units[fe.id] = freshUnit(fe);
    // Morale is derived from cohesion and suppression from here on.
    forceElements[fe.id] = { ...fe, morale: fe.combatStrength > 0 ? "good" : fe.morale };
    startStrength[fe.side] += Math.max(0, fe.combatStrength);
  }
  // Contacts already on the board count as seen at the start. Without this
  // they are "not seen for ever" and fade on the very first tick.
  const lastSeen: RtState["lastSeen"] = { blue: {}, red: {} };
  for (const side of ["blue", "red"] as const) {
    for (const [id, level] of Object.entries(game.sighting[side] ?? {})) {
      if (level !== "none") lastSeen[side][id] = 0;
    }
  }
  return {
    time: 0,
    game: { ...game, forceElements, phase: "arcAction" },
    units,
    lastSeen,
    lastFiredOn: { blue: {}, red: {} },
    reports: [],
    lastKnown: { blue: {}, red: {} },
    startStrength,
    plan: {},
  };
}

/**
 * The route for a move, from where the unit stands.
 *
 * Planned once, when the order is given — the raster's A* on real ground,
 * the bearing planner otherwise — and then followed waypoint by waypoint.
 * A plan that cannot be made leaves the order without a route, and the unit
 * walks straight and stops at the first ground it cannot enter.
 */
export function routed(order: RtOrder, fe: ForceElement, config: RtConfig): RtOrder {
  if (order.kind !== "move" && order.kind !== "withdraw") return order;
  if (order.route) return order;
  const planner = config.planner ?? bearingRoutePlanner(config.terrain, config.ruleset.movement);
  // Standing on ground it cannot move over (placed on a riverbank the raster
  // calls water, say): no route starts there, and it cannot take a step. So
  // it first gets out to the nearest ground it can move on, and the route is
  // planned from there.
  const out = allowanceAt(fe, fe.position, config) > 0 ? null : nearestPassable(fe, config, order.to);
  const from = out ?? fe.position;
  const planned = planner.plan(from, order.to, fe.moveType);
  const waypoints = out ? [out, ...(planned ?? [order.to])] : planned;
  if (!waypoints || waypoints.length === 0) return order;
  // A goal on impassable ground is snapped by the planner to the nearest
  // ground it can reach; that is where the unit is really going.
  const last = waypoints[waypoints.length - 1];
  return { ...order, to: last, route: waypoints };
}

/**
 * The nearest point within 400 m that this unit can move on, or null. Of
 * points equally near, the one towards `toward` (where it is going), so it
 * does not climb out on the wrong bank.
 */
export function nearestPassable(fe: ForceElement, config: RtConfig, toward?: LatLng): LatLng | null {
  for (const ring of [15, 30, 60, 100, 150, 220, 300, 400]) {
    const points = Array.from({ length: 16 }, (_, i) => offsetBy(fe.position, i * 22.5, ring)).filter(
      (point) => allowanceAt(fe, point, config) > 0,
    );
    if (points.length) {
      return toward ? points.sort((a, b) => distanceM(a, toward) - distanceM(b, toward))[0] : points[0];
    }
  }
  return null;
}

/** Give a unit a new order. Resets what it is already exposed to, and any bounding. */
export function setOrder(
  state: RtState,
  id: string,
  order: RtOrder,
  config: RtConfig,
  extra: Partial<RtUnit> = {},
): RtState {
  const unit = state.units[id];
  const fe = state.game.forceElements[id];
  if (!unit || !fe) return state;
  const planned = routed(order, fe, config);
  const next = {
    ...state,
    units: { ...state.units, [id]: { ...unit, ...extra, order: planned, bound: undefined } },
  };
  return {
    ...next,
    units: { ...next.units, [id]: { ...next.units[id], exposedTo: exposureOf(next, id, config) } },
  };
}

export function describeOrder(order: RtOrder): string {
  switch (order.kind) {
    case "move":
      return order.dash ? "dashing for cover" : `moving (${order.mode})`;
    case "withdraw":
      return "withdrawing";
    case "engage":
      return `engaging ${order.targetId}${order.volleys != null ? `, ${order.volleys} more volley${order.volleys === 1 ? "" : "s"}` : ""}`;
    case "wait":
      return `waiting on ${order.targetId} until ${describeTrigger(order.trigger)}${order.autoFire ? ", then firing" : ""}`;
    case "observe":
      return "observing, not firing";
    case "search":
      return "searching a bearing";
    default:
      return order.kind;
  }
}

/** A trigger, in words. */
export function describeTrigger(trigger: Trigger): string {
  switch (trigger.kind) {
    case "hitChance":
      return `its hit chance reaches ${Math.round(trigger.atLeast * 100)}% (about ${Math.round(trigger.aboutM / 100) * 100} m)`;
    case "range":
      return `it is inside ${trigger.withinM} m`;
    case "flank":
      return "it shows its side";
    case "reaches":
      return `it reaches ${trigger.label}`;
  }
}

/**
 * What a unit is doing, as one of the design's six activities (§3), for the
 * screen and for Jev.
 */
export function activityOf(unit: Pick<RtUnit, "order" | "cohesion">): string {
  if (unit.cohesion !== "steady") return unit.cohesion;
  const order = unit.order;
  if (order.phase != null) return "executing its order";
  switch (order.kind) {
    case "engage":
      return "engaging";
    case "wait":
      return "waiting for a trigger";
    case "move":
      return order.dash ? "taking cover" : "manoeuvring";
    case "withdraw":
      return "withdrawing";
    default:
      return "observing";
  }
}

/** Would standing at `at` break one of these lines? */
export function crosses(boundaries: readonly Boundary[], at: LatLng): Boundary | undefined {
  return boundaries.find((line) => {
    switch (line.keep) {
      case "north":
        return at.lat < line.at.lat;
      case "south":
        return at.lat > line.at.lat;
      case "east":
        return at.lng < line.at.lng;
      case "west":
        return at.lng > line.at.lng;
    }
  });
}

/** The nearest cover within `radius`, preferring points away from the threat. */
export function nearestCover(
  fe: ForceElement,
  config: RtConfig,
  radius: number,
  threat?: LatLng,
): LatLng | null {
  const rings = [radius / 3, (2 * radius) / 3, radius];
  const points = rings
    .flatMap((ring) => Array.from({ length: 8 }, (_, i) => offsetBy(fe.position, i * 45, ring)))
    .filter((point) => allowanceAt(fe, point, config) > 0 && inCover(config.terrain, point));
  const score = (point: LatLng) =>
    distanceM(fe.position, point) -
    (threat ? (distanceM(point, threat) - distanceM(fe.position, threat)) / 2 : 0);
  return points.sort((a, b) => score(a) - score(b))[0] ?? null;
}

/**
 * Is a waiting unit's trigger met? Game logic checks it every second (D4).
 * Nothing is met without a line of sight and a weapon that reaches.
 */
export function triggerMet(
  state: Pick<RtState, "units" | "time">,
  self: ForceElement,
  target: ForceElement,
  trigger: Trigger,
  config: RtConfig,
): boolean {
  if (!canHit(self, target, target.position, config)) return false;
  const range = distanceM(self.position, target.position);
  switch (trigger.kind) {
    case "range":
      return range <= trigger.withinM;
    case "reaches":
      return distanceM(target.position, trigger.at) <= trigger.withinM;
    case "flank": {
      const aspect = aspectOf(target, self.position, config.ruleset);
      return aspect === "side" || aspect === "rear";
    }
    case "hitChance": {
      const weapon = weaponFor(self, target, range);
      if (!weapon) return false;
      return hitChance(weapon, target, range, hitConditions(state, self, target, config), config.timing) >= trigger.atLeast;
    }
  }
}

export function tick(prev: RtState, config: RtConfig): TickResult {
  if (prev.over) return { state: prev, events: [], shots: [] };

  const timing = config.timing;
  const rng = config.rng;
  const time = prev.time + timing.tickS;
  const turn = Math.floor(time / timing.turnS) + 1;
  let game: GameState = { ...prev.game, turn };
  const units: Record<string, RtUnit> = { ...prev.units };
  const lastSeen = { blue: { ...prev.lastSeen.blue }, red: { ...prev.lastSeen.red } };
  const lastFiredOn = { blue: { ...prev.lastFiredOn.blue }, red: { ...prev.lastFiredOn.red } };
  const lastKnown = { blue: { ...prev.lastKnown.blue }, red: { ...prev.lastKnown.red } };
  let reports = [...prev.reports];
  const events: RtEvent[] = [];
  const shots: RtShot[] = [];

  const ids = Object.keys(game.forceElements).sort();
  const fe = (id: string) => game.forceElements[id];
  const alive = (id: string) => isAlive(game, id) && units[id] != null;
  const setFe = (id: string, patch: Partial<ForceElement>) => {
    game = { ...game, forceElements: { ...game.forceElements, [id]: { ...fe(id), ...patch } } };
  };
  const setUnit = (id: string, patch: Partial<RtUnit>) => {
    units[id] = { ...units[id], ...patch };
  };
  const emit = (
    unitId: string,
    kind: RtEvent["kind"],
    detail: string,
    severe = false,
    extra: Partial<Pick<RtEvent, "about" | "located" | "bearingDeg" | "info">> = {},
  ) => {
    if (!alive(unitId)) return;
    events.push({ time, unitId, kind, detail, severe, ...extra });
    if (!extra.info) setUnit(unitId, { lastEventAt: time });
  };
  /** Switch to what follows an order — its `then`, or hold — routed from where the unit is now. */
  const follow = (id: string, then: RtOrder | undefined) => {
    setUnit(id, { order: then ? routed(then, fe(id), config) : { kind: "hold" }, bound: undefined, laying: undefined });
  };
  /** A phase of its standing orders is done: on to the next with no decision, or out of orders (D10). */
  const nextPhase = (id: string, done: number, what: string) => {
    const orders = units[id].orders;
    const next = done + 1;
    if (orders && next < orders.phases.length) {
      setUnit(id, { orders: { ...orders, phase: next } });
      follow(id, { ...orders.phases[next].order, phase: next });
      emit(id, "phaseDone", `${what}; now: ${orders.phases[next].label}`, false, { info: true });
    } else {
      setUnit(id, { order: { kind: "hold" }, bound: undefined, ...(orders ? { orders: { ...orders, done: true } } : {}) });
      emit(id, "outOfOrders", `${what}: its orders are complete`);
    }
  };
  const friendsNear = (of: ForceElement) =>
    ids.filter(
      (id) =>
        id !== of.id &&
        fe(id).side === of.side &&
        alive(id) &&
        distanceM(fe(id).position, of.position) <= FRIEND_RADIUS_M,
    );
  const known = (id: string, enemyId: string) => knownTo(game, units, id, enemyId);
  const sees = (from: LatLng, to: LatLng) => lineOfSight(config.terrain, { from, to }).visible;
  const hqNear = (self: ForceElement) =>
    ids.some(
      (id) =>
        fe(id).side === self.side &&
        alive(id) &&
        (fe(id).commandRating ?? 0) > 0 &&
        units[id].cohesion !== "broken" &&
        distanceM(fe(id).position, self.position) <= HQ_RADIUS_M,
    );
  const clearMoved = (id: string) => {
    const self = fe(id);
    if (self.markers.includes("moved") && time - units[id].lastMovedAt > SETTLE_S) {
      setFe(id, { markers: self.markers.filter((m) => m !== "moved") });
    }
  };

  /** A unit saw an enemy: it knows at once; its side hears after the report delay. */
  const observe = (observerId: string, enemyId: string, level: ReportLevel) => {
    const side = fe(observerId).side;
    const own = units[observerId].ownSeen[enemyId];
    const { [enemyId]: _found, ...suspects } = units[observerId].suspects;
    const { [enemyId]: _done, ...locating } = units[observerId].locating;
    setUnit(observerId, {
      ownSeen: {
        ...units[observerId].ownSeen,
        [enemyId]: { time, level: own ? higher(own.level, level) : level },
      },
      ...(_found || _done ? { suspects, locating } : {}),
    });
    lastSeen[side][enemyId] = time;
    if (SIGHT_RANK[sightingOf(game, side, enemyId)] >= SIGHT_RANK[level]) return;
    if (reports.some((r) => r.side === side && r.enemyId === enemyId && SIGHT_RANK[r.level] >= SIGHT_RANK[level])) {
      return;
    }
    reports.push({ side, enemyId, level, dueAt: time + timing.reportDelayS });
  };

  /**
   * React to contact (Battle Drill 1): return fire at once, get into the
   * nearest cover, THEN let the commander decide. Only a steady unit that is
   * not assaulting or withdrawing; what it does depends on its movement mode.
   * Returns what it did, for the event's detail.
   */
  const drill = (id: string, threatId: string): string => {
    const did = runDrill(id, threatId);
    if (did && !did.startsWith("; pressing")) {
      setUnit(id, {
        history: remember(units[id], {
          time,
          chose: `drill: ${did.replace(/^; /, "")}`,
          by: "crew",
          because: `contact with ${threatId}`,
          strength: fe(id).combatStrength,
          dealt: units[id].dealt,
        }),
      });
    }
    return did;
  };
  const runDrill = (id: string, threatId: string): string => {
    const self = fe(id);
    const unit = units[id];
    if (unit.cohesion !== "steady") return "";
    const order = unit.order;
    if (order.kind === "withdraw") return "";
    if (order.kind === "move" && order.mode === "assault" && !order.dash) return "; pressing the assault";
    // Return fire only at a shooter it has located; otherwise it only knows a bearing.
    const located = known(id, threatId) !== "none";
    const answer = located ? "returning fire" : "shooter not located";
    if (located) setUnit(id, { weaponReadyAt: Math.min(unit.weaponReadyAt, time + RETURN_FIRE_S) });
    // Hidden and still, and now found: the crew shoots back at what it can see.
    if (located && (order.kind === "wait" || order.kind === "observe" || order.kind === "search")) {
      setUnit(id, { order: { kind: "engage", targetId: threatId } });
      return "; returning fire";
    }
    if (order.kind !== "move" || order.dash) return located ? "; returning fire" : "";
    if (order.mode === "bound") {
      setUnit(id, { bound: { moving: false, until: time + BOUND_COVER_S } });
      return `; ${answer}, halted to cover`;
    }
    const threat = fe(threatId);
    if (!inCover(config.terrain, self.position)) {
      const cover = nearestCover(self, config, DRILL_COVER_M, threat?.position);
      if (cover) {
        setUnit(id, {
          order: routed({ kind: "move", to: cover, mode: "march", dash: true }, self, config),
          bound: undefined,
        });
        return `; ${answer} and dashing ${Math.round(distanceM(self.position, cover))} m ${compass(
          self.position,
          cover,
        )} for cover (${config.terrain.classify(cover)})`;
      }
    }
    // No cover close by: a fold in the ground that hides the hull will do.
    if (threat && !inCover(config.terrain, self.position)) {
      const spot = hullDownSpot(self, threat.position, config, DRILL_COVER_M);
      if (spot && distanceM(spot, self.position) > 5) {
        setUnit(id, {
          order: routed({ kind: "move", to: spot, mode: "march", dash: true }, self, config),
          bound: undefined,
        });
        return `; ${answer} and backing ${Math.round(distanceM(self.position, spot))} m ${compass(
          self.position,
          spot,
        )} into a hull-down position`;
      }
    }
    setUnit(id, {
      order: located ? { kind: "engage", targetId: threatId } : { kind: "hold" },
      bound: undefined,
    });
    return `; ${answer} and halted`;
  };

  // ── 1. Contact reports reach the side ─────────────────────────────────────
  for (const report of reports.filter((r) => r.dueAt <= time)) {
    if (!isAlive(game, report.enemyId)) continue;
    if (SIGHT_RANK[sightingOf(game, report.side, report.enemyId)] >= SIGHT_RANK[report.level]) continue;
    game = applyEffects(game, [{ kind: "sighting", viewer: report.side, feId: report.enemyId, to: report.level }]);
  }
  reports = reports.filter((r) => r.dueAt > time);

  // ── 2. Movement ───────────────────────────────────────────────────────────
  //
  // ⚠ SIMULTANEOUS, like fire. Every unit steps from where it stood at the
  // start of the tick, and only then does anyone look at where everyone
  // ended up. Checking contact inside the loop let the first unit in the list
  // see the enemy where it WAS and halt first, which was enough to tip
  // identical engagements to blue.
  const moving: string[] = [];
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0) continue;
    const order = unit.order;
    if (order.kind !== "move" && order.kind !== "withdraw") {
      clearMoved(id);
      continue;
    }
    // An advance needs a steady, unpinned unit; a withdrawal only needs to be alive.
    if (order.kind === "move" && (unit.cohesion !== "steady" || !canAdvance(self.morale))) continue;

    const factor = order.kind === "move" ? SPEED[order.mode] : SPEED.withdraw;
    // The ground's allowance, how the unit is moving, the platform's own
    // speed (L7 stat card) and, uphill, its power to weight.
    // Getting out of ground it cannot move on, to the first point of its
    // route (see `routed`): it moves at the pace of the ground it is heading for.
    const hereAllowance = allowanceAt(self, self.position, config);
    const escapeTo = hereAllowance <= 0 ? order.route?.[0] : undefined;
    const escaping = escapeTo != null && allowanceAt(self, escapeTo, config) > 0 && distanceM(self.position, escapeTo) <= 450;
    const perSecond =
      ((escaping ? allowanceAt(self, escapeTo, config) : hereAllowance) / timing.turnS) *
      factor *
      platformSpeedFactor(self) *
      slopeFactor(self, self.position, towards(self.position, order.route?.[0] ?? order.to, 50), config);

    // Bounding overwatch: move a bound, then cover while the partner moves.
    if (order.kind === "move" && order.mode === "bound" && !order.dash) {
      const partner = ids
        .filter((other) => other !== id && alive(other) && fe(other).side === self.side)
        .filter((other) => {
          const o = units[other].order;
          return o.kind === "move" && o.mode === "bound" && units[other].bound != null;
        })
        .filter((other) => distanceM(fe(other).position, self.position) <= 1000)
        .sort((a, b) => distanceM(fe(a).position, self.position) - distanceM(fe(b).position, self.position))[0];
      const partnerBound = partner ? units[partner].bound : undefined;
      const boundS = Math.ceil(BOUND_M / Math.max(0.1, perSecond));
      let bound = unit.bound;
      if (!bound) {
        bound = partnerBound?.moving
          ? { moving: false, until: time + BOUND_COVER_S }
          : { moving: true, until: time + boundS };
      } else if (time >= bound.until) {
        if (bound.moving) bound = { moving: false, until: time + BOUND_COVER_S };
        else if (partnerBound?.moving && time < partnerBound.until) bound = { moving: false, until: partnerBound.until };
        else bound = { moving: true, until: time + boundS };
      }
      setUnit(id, { bound });
      if (!bound.moving) {
        clearMoved(id);
        continue;
      }
    }

    // Head for the next waypoint of the route, or straight for the goal.
    const route = order.route ?? [];
    const aim = route[0] ?? order.to;
    const step = perSecond * timing.tickS;
    const next = step > 0 ? towards(self.position, aim, step) : self.position;
    const broken = unit.cohesion === "broken";
    // The standing orders' lines are not crossed, whatever the order says. A
    // broken unit is running and does not read its orders.
    const line = !broken && unit.orders ? crosses(unit.orders.boundaries, next) : undefined;
    // A phase of its orders that cannot be carried out is remembered, so it
    // is not sent back into the same obstacle again and again.
    const stuck = (why: string) =>
      !broken && order.phase != null && unit.orders
        ? { orders: { ...unit.orders, blocked: { phase: order.phase, time, why } } }
        : {};
    if (line && !crosses([line], self.position)) {
      const why = `stopped short of ${line.label}: its orders keep it ${line.keep} of it`;
      setUnit(id, { order: { kind: "hold" }, bound: undefined, ...stuck(why) });
      emit(id, "blocked", why);
      continue;
    }
    if (step <= 0 || (allowanceAt(self, next, config) <= 0 && !escaping)) {
      // Say which: standing on it, no route found, or the route itself led there.
      const where =
        step <= 0
          ? "it is standing on it and there is no ground it can move on close by"
          : !order.route?.length
            ? "no route could be planned from here, so it went straight"
            : "on its planned route";
      const why = `cannot move on through ${config.terrain.classify(next)} (${where})`;
      setUnit(id, { order: { kind: "hold" }, bound: undefined, ...(broken ? { fellBack: true } : {}), ...stuck(why) });
      emit(id, "blocked", why);
      continue;
    }

    setFe(id, {
      position: next,
      facing: bearingDeg(self.position, aim),
      markers: self.markers.includes("moved") ? self.markers : [...self.markers, "moved"],
    });
    const reachedWaypoint = route.length > 0 && distanceM(next, aim) < 1;
    setUnit(id, {
      lastMovedAt: time,
      ...(reachedWaypoint ? { order: { ...order, route: route.slice(1) } } : {}),
    });

    if (distanceM(next, order.to) < 1) {
      if (!broken && order.then) {
        follow(id, order.then);
        continue;
      }
      if (!broken && order.phase != null) {
        nextPhase(id, order.phase, `reached the end of "${units[id].orders?.phases[order.phase]?.label ?? "its move"}"`);
        continue;
      }
      setUnit(id, { order: { kind: "hold" }, bound: undefined, ...(broken ? { fellBack: true } : {}) });
      emit(
        id,
        "arrived",
        order.kind === "withdraw"
          ? broken
            ? "reached its rally point"
            : "withdrawal complete"
          : order.dash
            ? "reached cover"
            : "reached its destination",
      );
      continue;
    }
    moving.push(id);
  }

  // ── 3. Running into the enemy ─────────────────────────────────────────────
  //
  // Now that everyone has moved: who has run into whom, and who has walked
  // into an identified enemy's reach.
  const contacts: { id: string; enemyId: string; range: number }[] = [];
  for (const id of moving) {
    const self = fe(id);
    const order = units[id].order;

    // Run into the enemy and you stop: nobody drives through a troop at
    // point-blank range. An assault closes further; a withdrawal and a dash
    // for cover keep going — that is what they are for.
    if (order.kind === "move" && !order.dash) {
      const halt = order.mode === "assault" ? ASSAULT_CONTACT_M : CLOSE_CONTACT_M;
      const close = forceElementsOf(game, opposing(self.side))
        .filter((enemy) => enemy.combatStrength > 0)
        .map((enemy) => ({ enemy, range: distanceM(enemy.position, self.position) }))
        .filter(({ range }) => range <= halt)
        .filter(({ enemy }) => sees(self.position, enemy.position))
        .sort((a, b) => a.range - b.range || a.enemy.id.localeCompare(b.enemy.id))[0];
      if (close) {
        contacts.push({ id, enemyId: close.enemy.id, range: close.range });
        continue;
      }
    }

    // Walking into an identified enemy's reach, once per enemy per order.
    for (const enemy of forceElementsOf(game, opposing(self.side))) {
      if (enemy.combatStrength <= 0) continue;
      if (units[id].exposedTo.includes(enemy.id)) continue;
      if (known(id, enemy.id) !== "full") continue;
      if (!canHit(enemy, self, self.position, config)) continue;
      setUnit(id, { exposedTo: [...units[id].exposedTo, enemy.id] });
      emit(
        id,
        "exposed",
        `moving into ${enemy.id}'s sight and range at ${Math.round(distanceM(enemy.position, self.position))} m`,
      );
      break;
    }
  }
  for (const { id, enemyId, range } of contacts) {
    const enemy = fe(enemyId);
    observe(id, enemyId, "full");
    const order = units[id].order;
    if (order.kind === "move" && order.mode === "assault") {
      setUnit(id, { order: { kind: "engage", targetId: enemyId } });
      emit(id, "contact", `closed with ${enemy.label} (${enemyId}) at ${Math.round(range)} m`, true);
    } else {
      const did = drill(id, enemyId);
      // A bounding unit halts to cover; anything else that is still on its
      // move after the drill (no cover, no knowledge) stops here.
      const after = units[id].order;
      if (after.kind === "move" && !after.dash && after.mode !== "bound") {
        setUnit(id, { order: { kind: "engage", targetId: enemyId } });
      }
      emit(id, "contact", `ran into ${enemy.label} (${enemyId}) at ${Math.round(range)} m${did}`, true);
    }
  }

  // ── 4. Sighting ───────────────────────────────────────────────────────────
  for (const side of ["blue", "red"] as const) {
    const observers = ids.filter((id) => fe(id).side === side && alive(id));
    const enemies = ids.filter((id) => fe(id).side !== side && alive(id));

    for (const enemyId of enemies) {
      const enemy = fe(enemyId);
      const their = units[enemyId];
      const enemyInCover = inCover(config.terrain, enemy.position);
      for (const observerId of observers) {
        const observer = fe(observerId);
        const range = distanceM(observer.position, enemy.position);
        if (range > LOS_CAP_M) continue;
        if (!sees(observer.position, enemy.position)) continue;

        // Already found (by this crew, or reported to it): tracked while in
        // sight, no roll — and identified once close enough to tell.
        const before = known(observerId, enemyId);
        if (before !== "none") {
          observe(observerId, enemyId, before === "full" || range <= IDENTIFY_M ? "full" : (before as ReportLevel));
          continue;
        }

        // Not yet found: a chance per second (detection.ts).
        const mine = units[observerId];
        const p = detectChance(
          range,
          {
            targetMoving: their.lastMovedAt === time,
            targetFired: time - their.lastShotAt <= 10,
            targetInCover: enemyInCover,
            targetHullDown: their.posture === "hullDown" && !enemyInCover,
            targetSettled: their.posture === "settled",
            targetOnFoot: enemy.targetClass === "foot",
            observerMoving: mine.lastMovedAt === time,
            observerSuppressed: mine.suppression >= SUPPRESSED_AT && mine.suppression < PINNED_AT,
            observerPinned: mine.suppression >= PINNED_AT,
            observerSearching:
              mine.order.kind === "search" &&
              bearingDeltaDeg(mine.order.bearingDeg, bearingDeg(observer.position, enemy.position)) <= SEARCH_ARC_DEG,
            observerWatching: mine.order.kind === "observe" || mine.order.kind === "wait",
          },
          timing.tickS,
        );
        if (p < 1 && rng.int(1_000_000) >= p * 1_000_000) continue;
        const level: ReportLevel = range <= IDENTIFY_M ? "full" : "partial";
        observe(observerId, enemyId, level);
        // A new enemy is worth a decision at once, not after the cooldown.
        emit(
          observerId,
          "sighted",
          `${level === "full" ? enemy.label : "an unidentified contact"} (${enemyId}) at ${Math.round(range)} m`,
          true,
          { about: enemyId },
        );
      }
    }

    // A unit forgets what it saw itself on the same clock as its side.
    for (const id of observers) {
      const own = units[id].ownSeen;
      const kept = Object.fromEntries(
        Object.entries(own).filter(
          ([enemyId, seen]) => isAlive(game, enemyId) && time - seen.time <= timing.contactMemoryS,
        ),
      );
      if (Object.keys(kept).length !== Object.keys(own).length) setUnit(id, { ownSeen: kept });
    }
    // Where each contact was last seen; and contact fades when nobody has
    // seen it for a while, leaving that last-known position on the map.
    for (const enemyId of enemies) {
      const level = sightingOf(game, side, enemyId);
      if (level === "none") continue;
      if (lastSeen[side][enemyId] === time || !lastKnown[side][enemyId]) {
        lastKnown[side][enemyId] = {
          at: fe(enemyId).position,
          time,
          ...(level === "full" ? { label: fe(enemyId).label } : {}),
        };
      }
      if (time - (lastSeen[side][enemyId] ?? -Infinity) <= timing.contactMemoryS) continue;
      game = applyEffects(game, [{ kind: "sighting", viewer: side, feId: enemyId, to: "none" }]);
    }
    for (const id of observers) {
      const order = units[id].order;
      if ((order.kind === "engage" || order.kind === "wait") && known(id, order.targetId) === "none") {
        if (order.kind === "engage" && order.then) {
          // "Finish this fight first" is over: the new order runs, no call.
          follow(id, order.then);
          emit(id, "targetGone", `lost sight of ${order.targetId}; on with its orders`, false, { info: true, about: order.targetId });
        } else {
          setUnit(id, { order: { kind: "hold" } });
          emit(id, "targetGone", `lost sight of ${order.targetId}`, false, { about: order.targetId });
        }
      }
    }
    // Bearing-only contacts are forgotten after a while.
    for (const id of observers) {
      const suspects = units[id].suspects;
      const kept = Object.entries(suspects).filter(
        ([enemyId, s]) => isAlive(game, enemyId) && time - s.time <= timing.contactMemoryS,
      );
      if (kept.length !== Object.keys(suspects).length) setUnit(id, { suspects: Object.fromEntries(kept) });
    }
  }

  // ── 4b. Cues: does the enemy know we are here? ─────────────────────────────
  //
  // A unit trying to stay hidden watches the enemies it can see for a sign it
  // has been seen: a halt, a turn towards it, a dash for cover, a start its
  // way. It sees only the behaviour, never the reason (knowledge.ts).
  for (const id of ids) {
    const unit = units[id];
    if (!unit || !alive(id) || unit.cohesion !== "steady" || !tryingToHide(unit, time)) continue;
    const self = fe(id);
    for (const [enemyId, seen] of Object.entries(unit.ownSeen)) {
      if (seen.time !== time || !alive(enemyId)) continue;
      const enemy = fe(enemyId);
      const range = distanceM(self.position, enemy.position);
      if (range > CUE_RANGE_M) continue;
      if (time - (units[id].cueAt[enemyId] ?? -Infinity) < CUE_INTERVAL_S) continue;
      const before = prev.units[enemyId];
      const was = prev.game.forceElements[enemyId];
      if (!before || !was) continue;
      const now = units[enemyId];
      const lastDrill = now.history[now.history.length - 1];
      const cue = cueOf({
        wasMoving: before.lastMovedAt === prev.time,
        isMoving: now.lastMovedAt === time,
        wasFacing: was.facing,
        isFacing: enemy.facing,
        bearingToMe: bearingDeg(enemy.position, self.position),
        wentToGround: lastDrill?.by === "crew" && time - lastDrill.time <= timing.tickS && /dashing|backing/.test(lastDrill.chose),
      });
      if (!cue) continue;
      setUnit(id, { cueAt: { ...units[id].cueAt, [enemyId]: time }, lastCue: { time, enemyId, cue } });
      emit(id, "cue", `${enemyId} ${cue} (${Math.round(range)} m ${compass(self.position, enemy.position)})`, true, {
        about: enemyId,
      });
    }
  }

  // ── 4c. Triggers ──────────────────────────────────────────────────────────
  //
  // A waiting unit's trigger, checked every second. Briefed to fire when it
  // is met, it fires at once — laid on already, as an ambush is. Otherwise it
  // asks (D4).
  for (const id of ids) {
    const unit = units[id];
    if (!unit || !alive(id) || unit.cohesion !== "steady") continue;
    const order = unit.order;
    if (order.kind !== "wait" || order.met) continue;
    const self = fe(id);
    const target = fe(order.targetId);
    if (!target || target.combatStrength <= 0 || known(id, target.id) === "none") continue;
    if (!triggerMet({ units, time }, self, target, order.trigger, config)) continue;
    const range = Math.round(distanceM(self.position, target.position));
    if (order.autoFire) {
      setUnit(id, {
        order: { kind: "engage", targetId: target.id },
        laying: { targetId: target.id, readyAt: time },
        weaponReadyAt: Math.min(unit.weaponReadyAt, time),
      });
      emit(id, "triggerMet", `${describeTrigger(order.trigger)}: firing on ${target.id} at ${range} m, as briefed`, false, {
        info: true,
        about: target.id,
      });
    } else {
      setUnit(id, { order: { ...order, met: true } });
      emit(id, "triggerMet", `${describeTrigger(order.trigger)}: ${target.id} at ${range} m`, true, { about: target.id });
    }
  }

  // ── 4d. Time-boxed orders run out ─────────────────────────────────────────
  for (const id of ids) {
    const unit = units[id];
    if (!unit || !alive(id)) continue;
    const order = unit.order;
    if (order.kind === "engage" && order.until != null && time >= order.until) {
      follow(id, order.then);
      emit(id, "phaseDone", `finished its fight with ${order.targetId} (time is up); on with its orders`, false, { info: true });
    } else if (order.kind === "search" && time >= order.until) {
      setUnit(id, { order: { kind: "hold" } });
      emit(id, "searchDone", `searched ${compassWord(order.bearingDeg)} and found nothing`, false, { bearingDeg: order.bearingDeg });
    }
  }

  // ── 5. Fire ───────────────────────────────────────────────────────────────
  //
  // ⚠ SIMULTANEOUS. Every shot this tick is chosen and rolled against the same
  // snapshot, and only then applied. Resolving them one unit at a time let
  // whoever came first in the list shoot first — which was blue, alphabetically
  // — and on open ground blue won 15 of 20 identical engagements. A unit that
  // is destroyed this tick still gets its own shot away, as it would.
  const snapshot = game;
  const planned: {
    firerId: string;
    targetId: string;
    rangeM: number;
    /** Rounds fired: one per fit vehicle. */
    rounds: number;
    /** Chance each hits (fire.ts). */
    pHit: number;
  }[] = [];

  for (const id of ids) {
    const self = snapshot.forceElements[id];
    const unit = units[id];
    if (!unit || self.combatStrength <= 0 || unit.cohesion === "broken") continue;
    if (unit.weaponReadyAt > time) continue;
    const order = unit.order;
    // A withdrawal does not stop to fight, and a road march does not fire on
    // the move. Tactical movement fires within its ROE, an assault at
    // anything, a bounding unit covering its partner like overwatch.
    if (order.kind === "withdraw") continue;

    const reachable = forceElementsOf(snapshot, opposing(self.side))
      .filter((enemy) => enemy.combatStrength > 0)
      .filter((enemy) => knownTo(snapshot, units, id, enemy.id) !== "none")
      .filter((enemy) => canHit(self, enemy, enemy.position, config));
    const firedOnMe = (enemyId: string) => time - (unit.attackers[enemyId] ?? -Infinity) <= FIRED_UPON_MEMORY_S;
    // Self-defence: whoever is shooting at this unit may always be shot back,
    // unless it has been told to stay silent (Command's weapons-hold).
    const selfDefence = unit.roe === "never" ? [] : reachable.filter((enemy) => firedOnMe(enemy.id));

    let candidates: ForceElement[] = [];
    let target: ForceElement | undefined;
    if (order.kind === "move" && order.mode === "march" && !order.dash) {
      candidates = [];
    } else if (
      unit.cohesion === "shaken" ||
      (order.kind === "move" && order.dash) ||
      order.kind === "wait" ||
      order.kind === "observe" ||
      order.kind === "search"
    ) {
      // Hiding, watching or searching: holds its fire, but shoots back at a
      // shooter it has located.
      candidates = selfDefence;
    } else {
      if (order.kind === "engage") {
        target = reachable.find((enemy) => enemy.id === order.targetId);
        if (!target && !isAlive(snapshot, order.targetId)) {
          if (order.then) {
            follow(id, order.then);
            emit(id, "targetGone", `${order.targetId} is destroyed; on with its orders`, false, { info: true, about: order.targetId });
          } else {
            setUnit(id, { order: { kind: "hold" } });
            emit(id, "targetGone", `${order.targetId} is destroyed`, false, { about: order.targetId });
          }
        }
      }
      if (!target) {
        const covering = order.kind === "move" && order.mode === "bound" && unit.bound?.moving === false;
        const assault = order.kind === "move" && order.mode === "assault";
        const roe = order.kind === "overwatch" || covering || assault ? "always" : unit.roe;
        candidates =
          unit.roe === "never"
            ? []
            : reachable.filter((enemy) => {
                if (firedOnMe(enemy.id) || roe === "always") return true;
                if (roe === "ifFiredUpon") {
                  return time - (lastFiredOn[self.side][enemy.id] ?? -Infinity) <= FIRED_UPON_MEMORY_S;
                }
                const range = distanceM(self.position, enemy.position);
                const weapon = weaponFor(self, enemy, range);
                return weapon != null && range <= weapon.shortRangeM;
              });
      }
    }
    if (!target && candidates.length) {
      // By threat, not by nearest: who is shooting at me, who can hurt me,
      // whose flank I have.
      const threat = (enemy: ForceElement) =>
        (firedOnMe(enemy.id) ? 3 : 0) +
        (canHit(enemy, self, self.position, config) ? 2 : 0) +
        (isFlankShot([self], enemy, config.ruleset) ? 1 : 0) -
        distanceM(self.position, enemy.position) / 3000;
      target = [...candidates].sort((a, b) => threat(b) - threat(a) || a.id.localeCompare(b.id))[0];
    }
    if (!target) {
      setUnit(id, { weaponReadyAt: time + RETARGET_S });
      continue;
    }

    const rangeM = distanceM(self.position, target.position);
    const weapon = weaponFor(self, target, rangeM)!;
    // A new target has to be found in the sight and laid on before the first
    // round goes (fire.ts, acquisitionS); a target already being engaged does not.
    const continuing =
      unit.engagement != null &&
      unit.engagement.targetId === target.id &&
      time - unit.engagement.lastShotAt <= FIRED_UPON_MEMORY_S;
    if (!continuing) {
      const laying = unit.laying;
      if (!laying || laying.targetId !== target.id) {
        const readyAt =
          time +
          acquisitionS(rangeM, {
            onlyReported: units[id].ownSeen[target.id] == null,
            suppressed: unit.suppression >= SUPPRESSED_AT && unit.suppression < PINNED_AT,
            pinned: unit.suppression >= PINNED_AT,
          });
        setUnit(id, { laying: { targetId: target.id, readyAt }, weaponReadyAt: readyAt });
        continue;
      }
      if (laying.readyAt > time) continue;
    }
    setUnit(id, { laying: undefined });
    planned.push({
      firerId: id,
      targetId: target.id,
      rangeM,
      rounds: Math.max(1, unit.vehicles.fit),
      pHit: hitChance(weapon, target, rangeM, hitConditions({ units, time }, self, target, config), timing),
    });
    // Its fire on this target, for "is this working?". A new target, or a
    // pause of a minute, starts a new record.
    const was = unit.engagement;
    const same = was != null && was.targetId === target.id && time - was.lastShotAt <= FIRED_UPON_MEMORY_S;
    const engagement = same
      ? { ...was, lastShotAt: time, shots: was.shots + 1, window: { ...was.window, shots: was.window.shots + 1 } }
      : {
          targetId: target.id,
          since: time,
          lastShotAt: time,
          shots: 1,
          hits: 0,
          damage: 0,
          window: { since: time, shots: 1, hits: 0, damage: 0 },
        };
    setUnit(id, {
      // From the weapon's sustained rate of fire (L7), not one figure for all.
      weaponReadyAt: time + aimedIntervalS(weapon, timing.shotIntervalS),
      lastShotAt: time,
      engagement,
      ...(same ? {} : { lastReviewAt: time }),
    });
    // "Fire N volleys, then…": fire and move, fire and move back.
    if (order.kind === "engage" && order.targetId === target.id && order.volleys != null) {
      if (order.volleys <= 1) follow(id, order.then);
      else setUnit(id, { order: { ...order, volleys: order.volleys - 1 } });
    }
  }

  // ── 6. What the fire did ──────────────────────────────────────────────────
  //
  // The table says how many rounds hit; fire.ts how many of those are on
  // target at this range; lethality.ts what each does to the face it strikes.
  // Every shot, misses included, suppresses.
  const newAttacker = new Map<string, string>();
  // FIRE DISTRIBUTION. Every round that hits was aimed at one vehicle of the
  // target, picked by its own gunner: two crews pick the same tank, or a
  // wreck that has not yet been seen to burn. Rounds on a vehicle already
  // knocked out this volley, or on a fresh wreck, are wasted. Slots are the
  // vehicles fit at the start of the volley, then the fresh wrecks.
  const slotsAtStart = new Map<string, { fit: number; wrecks: number }>();
  const killedSlots = new Map<string, Set<number>>();
  for (const shot of planned) {
    if (slotsAtStart.has(shot.targetId)) continue;
    const t = units[shot.targetId];
    slotsAtStart.set(shot.targetId, {
      fit: t.vehicles.fit,
      wrecks: t.losses.filter((at) => time - at <= WRECK_DRAWS_FIRE_S).length,
    });
    killedSlots.set(shot.targetId, new Set());
  }
  const labels = new Map<(typeof planned)[number], string>();
  for (const shot of planned) {
    const firer = snapshot.forceElements[shot.firerId];
    const target = fe(shot.targetId);
    const targetUnit = units[shot.targetId];
    if (target.combatStrength <= 0) continue;
    // Every round: does it hit (fire.ts)? Every hit: intercepted, stopped by
    // the armour on the face it strikes, or through — and does it knock the
    // vehicle out (lethality.ts)?
    const weapon = weaponFor(firer, target, shot.rangeM);
    const odds = weapon
      ? strikeOdds(weapon, target, firer.position, shot.rangeM, config.ruleset, hullDownFrom({ units }, target, firer.position, config))
      : null;
    const outcomes: StrikeResult[] = [];
    let hits = 0;
    let wasted = 0;
    const slots = slotsAtStart.get(target.id)!;
    const killed = killedSlots.get(target.id)!;
    for (let k = 0; k < shot.rounds; k += 1) {
      if (rng.int(1_000_000) >= shot.pHit * 1_000_000 || !odds) continue;
      hits += 1;
      const slot = rng.int(slots.fit + slots.wrecks);
      if (slot >= slots.fit || killed.has(slot)) {
        wasted += 1;
        continue;
      }
      const outcome = rollStrike(odds, rng);
      outcomes.push(outcome);
      if (outcome === "knockedOut") killed.add(slot);
    }
    const vehicles = units[target.id].vehicles;
    const knocked = Math.min(vehicles.fit, outcomes.filter((one) => one === "knockedOut").length);
    const damaged = knocked > 0;
    let lostNow = 0;
    if (damaged) {
      const next = { ...vehicles, fit: vehicles.fit - knocked };
      const strength = strengthFor(target, next);
      lostNow = target.combatStrength - strength;
      setUnit(target.id, {
        vehicles: next,
        losses: [...units[target.id].losses.filter((at) => time - at <= WRECK_DRAWS_FIRE_S), ...Array(knocked).fill(time)],
      });
      game = applyEffects(game, [
        { kind: "combatStrength", feId: target.id, delta: -lostNow },
        ...(strength <= 0 ? [{ kind: "eliminated" as const, feId: target.id }] : []),
      ]);
    }
    const struck = hits > 0;
    const face = odds ? describeStrike(odds) : "";
    const fired = `${shot.rounds} round${shot.rounds === 1 ? "" : "s"}`;
    const what =
      knocked > 0
        ? `knocked out ${knocked} (${face}); ${vehicles.fit - knocked}/${vehicles.total} left`
        : outcomes.includes("survived")
          ? `penetrated (${face}), crew fighting on`
          : outcomes.includes("noPenetration")
            ? `did not penetrate (${face})`
            : outcomes.includes("intercepted")
              ? "intercepted by active protection"
              : "";
    const onWrecks = wasted > 0 ? `${what ? "; " : ""}${wasted} on a tank already knocked out` : "";
    const label = struck ? `${fired}, ${hits} hit: ${what}${onWrecks}` : `${fired}, all missed`;
    labels.set(shot, label);

    const base =
      (struck ? SUPPRESSION_FOR.hit : shot.rounds > 1 ? SUPPRESSION_FOR.suppress : SUPPRESSION_FOR.miss) +
      (damaged ? SUPPRESSION_FOR.damaged : 0);
    const firerUnit = units[firer.id];
    if (firerUnit?.engagement && firerUnit.engagement.targetId === target.id) {
      const e = firerUnit.engagement;
      const struckCount = hits;
      setUnit(firer.id, {
        dealt: firerUnit.dealt + lostNow,
        engagement: {
          ...e,
          hits: e.hits + struckCount,
          damage: e.damage + lostNow,
          window: { ...e.window, hits: e.window.hits + struckCount, damage: e.window.damage + lostNow },
        },
      });
    }
    const had = targetUnit.incoming[firer.id];
    const incoming = {
      ...targetUnit.incoming,
      [firer.id]:
        had && time - had.last <= FIRED_UPON_MEMORY_S * 2
          ? { ...had, shots: had.shots + 1, damage: had.damage + lostNow, last: time }
          : { shots: 1, damage: lostNow, since: time, last: time },
    };
    const cover = isCovered({ units }, target, config, firer.position) ? 0.6 : 1;
    const quality = Math.max(0.5, 1.2 - target.troopQuality * 0.05);
    const recent = time - (targetUnit.attackers[firer.id] ?? -Infinity) <= FIRED_UPON_MEMORY_S;
    if (!recent && !newAttacker.has(target.id)) newAttacker.set(target.id, firer.id);
    setUnit(target.id, {
      suppression: Math.min(100, targetUnit.suppression + base * cover * quality),
      lastIncomingAt: time,
      attackers: { ...targetUnit.attackers, [firer.id]: time },
      incoming,
    });
    lastFiredOn[target.side][firer.id] = time;
    shots.push({
      time,
      firerId: firer.id,
      targetId: target.id,
      result: label,
      narrative:
        `${firer.label} fired ${shot.rounds} at ${target.label} at ${Math.round(shot.rangeM)} m, ` +
        `${Math.round(shot.pHit * 100)}% each to hit: ${label}.`,
    });

    // THE LOCATE ROLL (knowledge.ts). The target always learns a bearing;
    // whether it finds the shooter is a roll — easier close, against a
    // shooter in the open, and with every volley from the same place.
    if (known(target.id, firer.id) === "none") {
      const tu = units[target.id];
      const bearing = bearingDeg(target.position, firer.position);
      const tries = tu.locating[firer.id];
      const same =
        tries != null &&
        distanceM(tries.at, firer.position) <= SHOOTER_MOVED_M &&
        time - tries.time <= FIRED_UPON_MEMORY_S;
      const volleys = same ? tries.volleys + 1 : 1;
      const searching =
        tu.order.kind === "search" && bearingDeltaDeg(tu.order.bearingDeg, bearing) <= SEARCH_ARC_DEG;
      const p = sees(target.position, firer.position)
        ? locateChance(shot.rangeM, {
            volleys,
            shooterInCover: inCover(config.terrain, firer.position),
            shooterHullDown: hullDownFrom({ units }, firer, target.position, config),
            targetSearching: searching,
            targetSuppressed: tu.suppression >= SUPPRESSED_AT && tu.suppression < PINNED_AT,
            targetPinned: tu.suppression >= PINNED_AT,
          })
        : 0;
      setUnit(target.id, {
        locating: { ...tu.locating, [firer.id]: { volleys, at: firer.position, time } },
        suspects: {
          ...tu.suspects,
          [firer.id]: { bearingDeg: bearing, time, from: target.position, why: `fired on from the ${compassWord(bearing)}` },
        },
      });
      if (p > 0 && (p >= 1 || rng.int(1_000_000) < p * 1_000_000)) observe(target.id, firer.id, "partial");
    }
    // Firing is heard: anyone near who does not know the shooter gets a bearing.
    for (const other of ids) {
      if (other === target.id || !alive(other) || fe(other).side === firer.side) continue;
      if (distanceM(fe(other).position, firer.position) > HEARD_M || known(other, firer.id) !== "none") continue;
      const bearing = bearingDeg(fe(other).position, firer.position);
      setUnit(other, {
        suspects: {
          ...units[other].suspects,
          [firer.id]: { bearingDeg: bearing, time, from: fe(other).position, why: `heard firing to the ${compassWord(bearing)}` },
        },
      });
    }
    // After its first volley at a target, or one that knocks a vehicle out,
    // the firer weighs what to do next (D6): has it been found yet?
    const record = units[firer.id]?.engagement;
    if (record && record.targetId === target.id && (record.shots === 1 || knocked > 0)) {
      emit(
        firer.id,
        "volley",
        knocked > 0 ? `knocked out ${knocked} of ${target.id}'s vehicles` : `first volley at ${target.id}: ${label}`,
        knocked > 0,
        { about: target.id },
      );
    }
  }

  // What the fire did to each target, compared with where it started.
  for (const targetId of [...new Set(planned.map((shot) => shot.targetId))].sort()) {
    const before = snapshot.forceElements[targetId];
    const after = fe(targetId);
    const firers = planned.filter((shot) => shot.targetId === targetId);
    const hurt = after.combatStrength < before.combatStrength;

    if (after.combatStrength <= 0) {
      if (before.combatStrength > 0) {
        for (const friend of friendsNear(after)) {
          setUnit(friend, { lastFriendLostAt: time });
          emit(friend, "friendLost", `${after.id} destroyed ${Math.round(distanceM(fe(friend).position, after.position))} m away`, true);
        }
      }
      continue;
    }
    const shooter = newAttacker.get(targetId);
    const did = shooter ? drill(targetId, shooter) : "";
    // Located, or only a bearing: decision point D2 or D3.
    const locatedFirer = firers.find((shot) => known(targetId, shot.firerId) !== "none");
    const unlocated = firers.find((shot) => known(targetId, shot.firerId) === "none");
    const about = locatedFirer?.firerId ?? unlocated?.firerId;
    const where = locatedFirer
      ? {}
      : { bearingDeg: bearingDeg(after.position, fe(unlocated!.firerId).position) };
    const unseen = unlocated && !locatedFirer ? ` (shooter not located: fire from the ${compassWord(where.bearingDeg!)})` : "";
    const by = locatedFirer
      ? firers
          .filter((shot) => known(targetId, shot.firerId) !== "none")
          .map((shot) => `${shot.firerId} (${Math.round(shot.rangeM)} m, ${labels.get(shot) ?? "fired"})`)
          .join(", ")
      : firers.map((shot) => labels.get(shot) ?? "fired").join(", ");
    emit(targetId, "underFire", `fired on by ${locatedFirer ? by : "an unseen enemy"}${unseen}${locatedFirer ? "" : `: ${by}`}${did}`, hurt || shooter != null, {
      about,
      located: locatedFirer != null,
      ...where,
    });
    if (hurt) {
      const v = units[targetId].vehicles;
      emit(targetId, "hit", `vehicle knocked out: ${v.fit} of ${v.total} still fighting`, true, {
        about,
        located: locatedFirer != null,
        ...where,
      });
    }
    // A friend in trouble: those close by who know its attacker may help (D9).
    if ((hurt || shooter != null) && about) {
      for (const friend of ids) {
        if (friend === targetId || !alive(friend) || fe(friend).side !== after.side) continue;
        const f = units[friend];
        if (f.cohesion !== "steady" || time - f.lastShotAt <= 30) continue;
        if (distanceM(fe(friend).position, after.position) > HELP_RADIUS_M) continue;
        if (time - (f.helpAt[targetId] ?? -Infinity) < HELP_INTERVAL_S) continue;
        const attacker = firers.map((shot) => shot.firerId).find((id) => known(friend, id) !== "none");
        if (!attacker) continue;
        setUnit(friend, { helpAt: { ...f.helpAt, [targetId]: time } });
        emit(friend, "friendNeedsHelp", `${targetId} is ${hurt ? "losing vehicles" : "under fire"} from ${attacker}`, hurt, {
          about: attacker,
        });
      }
    }
  }

  // ── 7. Suppression fades; posture ─────────────────────────────────────────
  for (const id of ids) {
    const unit = units[id];
    const self = fe(id);
    if (!unit || self.combatStrength <= 0) continue;
    const suppression =
      time - unit.lastIncomingAt > SUPPRESSION_GRACE_S
        ? Math.max(0, unit.suppression - SUPPRESSION_DECAY_PER_S * timing.tickS)
        : unit.suppression;
    const pinnedSince = suppression >= PINNED_AT ? (unit.pinnedSince ?? time) : null;

    let posture: Posture =
      unit.lastMovedAt === time ? "moving" : time - unit.lastMovedAt < SETTLE_S ? "halted" : "settled";
    if (posture === "settled") {
      const nearest = knownEnemies({ game, units }, id).sort(
        (a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position),
      )[0];
      const defilade = nearest != null && hullDownAgainst(self.position, nearest.position, config);
      if (inCover(config.terrain, self.position) || defilade) posture = "hullDown";
    }
    if (suppression !== unit.suppression || pinnedSince !== unit.pinnedSince || posture !== unit.posture) {
      setUnit(id, { suppression, pinnedSince, posture });
    }
  }

  // ── 8. Break tests ────────────────────────────────────────────────────────
  //
  // A unit tests its nerve when its losses cross a threshold — about 20% for
  // an attacker, 40% for a defender (Dupuy), then every 20% more — and when
  // it has been pinned for a minute. It does not lose nerve a step per hit.
  const brokeNow: string[] = [];
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0 || unit.cohesion === "broken") continue;
    const losses = 1 - unit.vehicles.fit / Math.max(1, unit.vehicles.total);
    const attacking =
      (unit.order.kind === "move" && !unit.order.dash) ||
      (unit.mission.task === "take" && time - unit.lastMovedAt <= ATTACKING_S);
    const threshold = (attacking ? ATTACKER_BREAK_AT : DEFENDER_BREAK_AT) + BREAK_STEP * unit.breakTests;
    const lossTest = losses >= threshold - 1e-9;
    const pinnedTest = unit.pinnedSince != null && time - unit.pinnedSince >= PINNED_TEST_S;
    if (!lossTest && !pinnedTest) continue;

    const score =
      rng.d6().total +
      Math.floor(self.troopQuality / 2) +
      (isCovered({ units }, self, config) ? 1 : 0) +
      (hqNear(self) ? 1 : 0) -
      (unit.suppression >= PINNED_AT ? 1 : 0) -
      (time - unit.lastFriendLostAt <= FRIEND_LOST_S ? 1 : 0);
    setUnit(id, {
      ...(lossTest ? { breakTests: unit.breakTests + 1 } : {}),
      ...(pinnedTest ? { pinnedSince: time } : {}),
    });
    // Losses can break a unit; being pinned can only shake it (it cowers, it
    // does not run) — otherwise long-range misses alone ended games.
    let cohesion: Cohesion = unit.cohesion;
    if (lossTest && (score < BREAK_SHAKEN || (score < BREAK_PASS && unit.cohesion === "shaken"))) cohesion = "broken";
    else if (score < BREAK_PASS && unit.cohesion === "steady") cohesion = "shaken";
    if (cohesion === unit.cohesion) continue;

    const why = lossTest ? `${Math.round(losses * 100)}% losses` : "pinned down";
    setUnit(id, { cohesion, lastRallyCheckAt: time, bound: undefined });
    if (cohesion === "shaken") {
      // Shaken: stops where it is, and only shoots back.
      if (unit.order.kind !== "withdraw") setUnit(id, { order: { kind: "hold" } });
      emit(id, "moraleDrop", `shaken (${why}): holding, firing only in self-defence`, true);
    } else {
      brokeNow.push(id);
      emit(id, "moraleDrop", `broken (${why}): falling back to rally`, true);
    }
  }

  // Broken units fall back ONCE, to a rally point: towards friends or an HQ,
  // away from the enemy they know about, out of its sight if possible. There
  // they stop and try to rally — they are not sent back again on arrival.
  for (const id of brokeNow) {
    const self = fe(id);
    const threats = knownEnemies({ game, units }, id);
    const danger = (at: LatLng) =>
      threats.length ? Math.min(...threats.map((enemy) => distanceM(enemy.position, at))) : Infinity;
    const hidden = (at: LatLng) => threats.every((enemy) => !sees(enemy.position, at));
    const here = danger(self.position);
    const friends = ids
      .filter((other) => other !== id && alive(other) && fe(other).side === self.side)
      .filter((other) => units[other].cohesion !== "broken")
      .map((other) => ({ at: fe(other).position, hq: (fe(other).commandRating ?? 0) > 0 }))
      .filter((one) => distanceM(one.at, self.position) <= RALLY_SEARCH_M)
      .filter((one) => !threats.length || danger(one.at) > here + 100);
    const pick = friends.sort(
      (a, b) =>
        Number(b.hq) - Number(a.hq) ||
        Number(hidden(b.at)) - Number(hidden(a.at)) ||
        distanceM(a.at, self.position) - distanceM(b.at, self.position),
    )[0];
    const nearestThreat = [...threats].sort(
      (a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position),
    )[0];
    const away = nearestThreat
      ? (bearingDeg(nearestThreat.position, self.position) + 360) % 360
      : game.objectives
        ? (bearingDeg(game.objectives[self.side], self.position) + 360) % 360
        : 180;
    const to = pick ? pick.at : offsetBy(self.position, away, FALL_BACK_M);
    setUnit(id, { order: routed({ kind: "withdraw", to }, self, config), fellBack: false });

    for (const friend of friendsNear(self)) {
      setUnit(friend, { lastFriendLostAt: time });
      emit(friend, "friendLost", `${id} broken ${Math.round(distanceM(fe(friend).position, self.position))} m away`, true);
    }
    for (const enemy of forceElementsOf(game, opposing(self.side))) {
      if (!alive(enemy.id) || units[enemy.id].cohesion !== "steady") continue;
      if (known(enemy.id, id) === "none" || !sees(enemy.position, self.position)) continue;
      emit(enemy.id, "enemyBroke", `${id} has broken and is falling back`);
    }
  }

  // ── 9. Rally ──────────────────────────────────────────────────────────────
  //
  // Out of fire, a shaken unit steadies and a broken one that has fallen back
  // pulls itself together, one step per successful check: faster with better
  // troops, an HQ close by, and no enemy in sight.
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0) continue;
    if (unit.cohesion === "steady" || (unit.cohesion === "broken" && !unit.fellBack)) continue;
    if (time - unit.lastRallyCheckAt < timing.rallyCheckS) continue;
    setUnit(id, { lastRallyCheckAt: time });
    if (unit.suppression >= SUPPRESSED_AT || time - unit.lastIncomingAt < QUIET_S) continue;
    const inContact = Object.values(unit.ownSeen).some((seen) => time - seen.time <= FIRED_UPON_MEMORY_S);
    const score =
      rng.d6().total + Math.floor(self.troopQuality / 2) + (hqNear(self) ? 1 : 0) + (inContact ? 0 : 1);
    if (score < RALLY_PASS) continue;
    if (unit.cohesion === "broken") {
      setUnit(id, { cohesion: "shaken", order: { kind: "hold" } });
    } else {
      setUnit(id, { cohesion: "steady", fellBack: false });
      emit(id, "rallied", "rallied: steady again and taking orders", true);
    }
  }

  // ── 10. Idle ──────────────────────────────────────────────────────────────
  //
  // Nothing happens to a unit holding with no enemy in view, so without this
  // it is never asked again — the winner "holding for ever". A steady unit
  // that is off its mission and has been quiet for `idleS` is asked again.
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0 || unit.cohesion !== "steady") continue;
    if (unit.order.kind === "move" || unit.order.kind === "withdraw") continue;
    if (onMission(unit, self)) continue;
    // Its orders are done, or cannot be carried out: the player has been told.
    // Asking it again every minute changes nothing.
    const o = unit.orders;
    if (o && (o.done || o.blocked?.phase === o.phase)) continue;
    const quiet = Math.min(time - unit.lastEventAt, time - unit.lastIncomingAt, time - unit.lastShotAt);
    if (quiet < timing.idleS) continue;
    emit(id, "idle", `quiet for ${Math.round(quiet)} s and off its mission (${unit.mission.purpose})`);
  }

  // Is its fire working? A unit that has been engaging for `REVIEW_S` is
  // asked again: "ineffective" if it has done no damage in that time, which
  // is what an endless long-range exchange of misses looks like, and
  // "review" if it has. Nothing else would ever ask the unit doing the firing.
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0 || unit.cohesion !== "steady") continue;
    const e = unit.engagement;
    if (!e || time - e.lastShotAt > FIRED_UPON_MEMORY_S) continue;
    if (time - Math.max(e.window.since, unit.lastReviewAt) < REVIEW_S) continue;
    const w = e.window;
    const span = clockOf(time - w.since);
    const taken = Object.values(unit.incoming)
      .filter((one) => one.last >= w.since)
      .reduce((sum, one) => sum + one.damage, 0);
    const summary =
      `${span} on ${e.targetId}: ${w.shots} shots, ${w.hits} struck, ${w.damage} damage done` +
      `; ${taken} damage taken meanwhile`;
    emit(id, w.damage === 0 ? "ineffective" : "review", w.damage === 0 ? `fire not working — ${summary}` : summary);
    setUnit(id, {
      lastReviewAt: time,
      engagement: { ...e, window: { since: time, shots: 0, hits: 0, damage: 0 } },
    });
  }

  // Morale as the shared rules see it.
  for (const id of ids) {
    const unit = units[id];
    if (!unit || fe(id).combatStrength <= 0) continue;
    const morale = derivedMorale(unit);
    if (fe(id).morale !== morale) setFe(id, { morale });
  }
  for (const side of ["blue", "red"] as const) {
    for (const enemyId of Object.keys(lastKnown[side])) {
      if (!isAlive(game, enemyId)) delete lastKnown[side][enemyId];
    }
  }

  let state: RtState = { ...prev, time, game, units, lastSeen, lastFiredOn, lastKnown, reports };

  // ── 11. Is it over? ───────────────────────────────────────────────────────
  //
  // A side is beaten when half its strength is destroyed or broken — a
  // breakpoint, not the last tank.
  const lost = (side: Side) => {
    const start = Math.max(1, prev.startStrength?.[side] ?? 0);
    const fighting = forceElementsOf(game, side)
      .filter((one) => one.combatStrength > 0 && units[one.id]?.cohesion !== "broken")
      .reduce((sum, one) => sum + one.combatStrength, 0);
    return 1 - fighting / start;
  };
  const blueLost = lost("blue");
  const redLost = lost("red");
  const blueBeaten = blueLost >= SIDE_BREAKPOINT;
  const redBeaten = redLost >= SIDE_BREAKPOINT;
  const pct = (x: number) => `${Math.round(x * 100)}%`;
  if (blueBeaten || redBeaten) {
    const winner: Side | null =
      blueBeaten && redBeaten
        ? blueLost < redLost
          ? "blue"
          : redLost < blueLost
            ? "red"
            : null
        : blueBeaten
          ? "red"
          : "blue";
    state = {
      ...state,
      over: {
        winner,
        reason:
          blueBeaten && redBeaten
            ? `both sides past their breakpoint (blue ${pct(blueLost)}, red ${pct(redLost)} destroyed or broken)`
            : `${blueBeaten ? "blue" : "red"} is past its breakpoint: ${pct(
                blueBeaten ? blueLost : redLost,
              )} of its strength destroyed or broken`,
      },
    };
  } else if (time >= timing.maxDurationS) {
    const verdict = judgeVictory(game, config.ruleset);
    state = { ...state, over: { winner: verdict.winner, reason: "time limit" } };
  }

  return { state, events, shots };
}
