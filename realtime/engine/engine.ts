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
//   4. sighting: certain at close range (how close depends on the target's
//      posture), otherwise staggered rolls; a unit knows what it saw at once,
//      its side after `reportDelayS`; faded contacts leave a last-known position
//   5. fire, simultaneous: one shot per `shotIntervalS`, targets by threat,
//      self-defence always allowed unless the ROE is "never"
//   6. what the fire did: strength (scaled by `lethalityPerTurn`), suppression,
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

import { bearingDeg, distanceM, LOS_CAP_M, type LatLng } from "../../lib/board";
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
import { canAdvance, resolveDirectFire, resolveSighting } from "../../rules/resolvers";
import { hitsFor, type FireResult } from "../../rules/ruleset";
import { isFlankShot, weaponFor } from "../../rules/turnLoop";
import { judgeVictory } from "../../rules/victory";
import { bearingRoutePlanner } from "../../lib/routePlan";
import { rangeModifiers, strikeChance } from "./fire";
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
import { describeStrike, rollStrike, strikeOdds, type StrikeResult } from "./lethality";
import {
  ASSAULT_CONTACT_M,
  ATTACKER_BREAK_AT,
  AUTO_SIGHT_HIDDEN_M,
  AUTO_SIGHT_M,
  AUTO_SIGHT_MOVING_M,
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
import type {
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

/** Suppression one shot adds, before cover and quality. */
const SUPPRESSION_FOR = { miss: 8, suppress: 18, hit: 30, damaged: 15 };

export interface TickResult {
  state: RtState;
  events: RtEvent[];
  shots: RtShot[];
}

/** A stable small number from a string, for staggering checks across ticks. */
function stagger(text: string, modulo: number): number {
  let h = 0;
  for (let i = 0; i < text.length; i += 1) h = (Math.imul(h, 31) + text.charCodeAt(i)) | 0;
  return Math.abs(h) % Math.max(1, modulo);
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
  const waypoints = planner.plan(fe.position, order.to, fe.moveType);
  if (!waypoints || waypoints.length === 0) return order;
  // A goal on impassable ground is snapped by the planner to the nearest
  // ground it can reach; that is where the unit is really going.
  const last = waypoints[waypoints.length - 1];
  return { ...order, to: last, route: waypoints };
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
      return `engaging ${order.targetId}`;
    default:
      return order.kind;
  }
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
  const emit = (unitId: string, kind: RtEvent["kind"], detail: string, severe = false) => {
    if (!alive(unitId)) return;
    events.push({ time, unitId, kind, detail, severe });
    setUnit(unitId, { lastEventAt: time });
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
    setUnit(observerId, {
      ownSeen: {
        ...units[observerId].ownSeen,
        [enemyId]: { time, level: own ? higher(own.level, level) : level },
      },
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
    setUnit(id, { weaponReadyAt: Math.min(unit.weaponReadyAt, time + RETURN_FIRE_S) });
    if (order.kind !== "move" || order.dash) return "; returning fire";
    if (order.mode === "bound") {
      setUnit(id, { bound: { moving: false, until: time + BOUND_COVER_S } });
      return "; returning fire, halted to cover";
    }
    const threat = fe(threatId);
    if (!inCover(config.terrain, self.position)) {
      const cover = nearestCover(self, config, DRILL_COVER_M, threat?.position);
      if (cover) {
        setUnit(id, {
          order: routed({ kind: "move", to: cover, mode: "march", dash: true }, self, config),
          bound: undefined,
        });
        return `; returning fire and dashing ${Math.round(distanceM(self.position, cover))} m ${compass(
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
        return `; returning fire and backing ${Math.round(distanceM(self.position, spot))} m ${compass(
          self.position,
          spot,
        )} into a hull-down position`;
      }
    }
    setUnit(id, {
      order: known(id, threatId) !== "none" ? { kind: "engage", targetId: threatId } : { kind: "hold" },
      bound: undefined,
    });
    return "; returning fire and halted";
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
    const perSecond =
      (allowanceAt(self, self.position, config) / timing.turnS) *
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
    if (step <= 0 || allowanceAt(self, next, config) <= 0) {
      setUnit(id, { order: { kind: "hold" }, bound: undefined, ...(broken ? { fellBack: true } : {}) });
      emit(id, "blocked", `cannot move on through ${config.terrain.classify(next)}`);
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
      // How close is certain depends on the target: a moving vehicle is seen
      // from further, a settled one in cover or hull-down only much closer.
      const posture: Posture = units[enemyId].posture;
      const hidden = posture === "hullDown" || inCover(config.terrain, fe(enemyId).position);
      const certain =
        posture === "moving"
          ? AUTO_SIGHT_MOVING_M
          : posture === "settled" || posture === "hullDown"
            ? hidden
              ? AUTO_SIGHT_HIDDEN_M
              : AUTO_SIGHT_M
            : AUTO_SIGHT_M;

      for (const observerId of observers) {
        const observer = fe(observerId);
        const enemy = fe(enemyId);
        const range = distanceM(observer.position, enemy.position);

        // Close enough that nobody fails to see it: no roll, every tick.
        if (range <= certain) {
          if (sees(observer.position, enemy.position)) {
            const was = known(observerId, enemyId);
            observe(observerId, enemyId, "full");
            if (was === "none") {
              emit(observerId, "sighted", `${enemy.label} (${enemyId}) at ${Math.round(range)} m, close`, true);
            }
          }
          continue;
        }

        // Seeded per game: an unseeded stagger gave the same side the earlier
        // look in every game, whatever the seed — enough to tip identical
        // engagements 44 to 15.
        if (
          (time + stagger(`${rng.seed}:${observerId}>${enemyId}`, timing.sightingIntervalS)) %
            timing.sightingIntervalS !==
          0
        ) {
          continue;
        }
        if (range > LOS_CAP_M) continue;
        if (!sees(observer.position, enemy.position)) continue;

        const before = known(observerId, enemyId);
        if (before === "full") {
          observe(observerId, enemyId, "full");
          continue;
        }
        if (before !== "none") observe(observerId, enemyId, before);
        else lastSeen[side][enemyId] = time;

        const outcome = resolveSighting(
          observer,
          enemy,
          { targetInCover: hidden },
          config.ruleset,
          rng,
          turn,
          "arcAction",
          side,
        );
        const found = outcome.effects.find((effect) => effect.kind === "sighting");
        const level = (found && found.kind === "sighting" ? found.to : "none") as SightingLevel;
        if (level === "none" || SIGHT_RANK[level] <= SIGHT_RANK[before]) continue;

        observe(observerId, enemyId, level);
        if (before === "none") {
          // A new enemy is worth a decision at once, not after the cooldown.
          emit(
            observerId,
            "sighted",
            `${level === "full" ? enemy.label : "an unidentified contact"} (${enemyId}) at ${Math.round(range)} m`,
            true,
          );
        }
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
      if (order.kind === "engage" && known(id, order.targetId) === "none") {
        setUnit(id, { order: { kind: "hold" } });
        emit(id, "targetGone", `lost sight of ${order.targetId}`);
      }
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
    outcome: ReturnType<typeof resolveDirectFire>;
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
    } else if (unit.cohesion === "shaken" || (order.kind === "move" && order.dash)) {
      candidates = selfDefence;
    } else {
      if (order.kind === "engage") {
        target = reachable.find((enemy) => enemy.id === order.targetId);
        if (!target && !isAlive(snapshot, order.targetId)) {
          setUnit(id, { order: { kind: "hold" } });
          emit(id, "targetGone", `${order.targetId} is destroyed`);
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
    planned.push({
      firerId: id,
      targetId: target.id,
      rangeM,
      outcome: resolveDirectFire(
        [self],
        target,
        {
          rangeM,
          maxRangeM: weapon.maxRangeM,
          penetrationMm: weapon.penetrationMm,
          munition: weapon.munition,
          topAttack: weapon.topAttack,
          targetInCover: isCovered({ units }, target, config, self.position),
          flank: isFlankShot([self], target, config.ruleset),
          // Real time only: closer is easier (see fire.ts).
          extraModifiers: rangeModifiers(rangeM),
        },
        config.ruleset,
        rng,
        turn,
        "arcAction",
      ),
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
      weaponReadyAt: time + timing.shotIntervalS,
      lastShotAt: time,
      engagement,
      ...(same ? {} : { lastReviewAt: time }),
    });
  }

  // ── 6. What the fire did ──────────────────────────────────────────────────
  //
  // The table was written for one result per 15-minute turn. Here a hit costs
  // strength only with probability hits × lethalityPerTurn × shotIntervalS /
  // turnS × the range's lethality factor (fire.ts), so a quarter-hour of
  // steady fire does about what `lethalityPerTurn` turn-game results would —
  // much more at point-blank range, less at the limit of the gun's reach.
  // Every shot, misses included, suppresses.
  const newAttacker = new Map<string, string>();
  const labels = new Map<(typeof planned)[number], string>();
  for (const shot of planned) {
    const firer = snapshot.forceElements[shot.firerId];
    const target = fe(shot.targetId);
    const targetUnit = units[shot.targetId];
    if (target.combatStrength <= 0) continue;
    const result = shot.outcome.event.result as FireResult;
    const hits = hitsFor(result);
    // Each fire-table hit is a round on target with a range-scaled chance
    // (fire.ts); each round on target is then intercepted, stopped by the
    // armour on the face it strikes, or penetrates and may knock a vehicle out
    // (lethality.ts).
    const weapon = weaponFor(firer, target, shot.rangeM);
    const odds = weapon
      ? strikeOdds(weapon, target, firer.position, shot.rangeM, config.ruleset, hullDownFrom({ units }, target, firer.position, config))
      : null;
    const outcomes: StrikeResult[] = [];
    for (let k = 0; k < hits; k += 1) {
      if (rng.int(1_000_000) >= strikeChance(shot.rangeM, timing) * 1_000_000 || !odds) continue;
      outcomes.push(rollStrike(odds, rng));
    }
    const vehicles = units[target.id].vehicles;
    const knocked = Math.min(vehicles.fit, outcomes.filter((one) => one === "knockedOut").length);
    const damaged = knocked > 0;
    let lostNow = 0;
    if (damaged) {
      const next = { ...vehicles, fit: vehicles.fit - knocked };
      const strength = strengthFor(target, next);
      lostNow = target.combatStrength - strength;
      setUnit(target.id, { vehicles: next });
      game = applyEffects(game, [
        { kind: "combatStrength", feId: target.id, delta: -lostNow },
        ...(strength <= 0 ? [{ kind: "eliminated" as const, feId: target.id }] : []),
      ]);
    }
    const struck = outcomes.length > 0;
    const face = odds ? describeStrike(odds) : "";
    const label =
      knocked > 0
        ? `knocked out ${knocked} (${face}); ${vehicles.fit - knocked}/${vehicles.total} left`
        : outcomes.includes("survived")
          ? `penetrated (${face}), crew fighting on`
          : outcomes.includes("noPenetration")
            ? `struck, did not penetrate (${face})`
            : outcomes.includes("intercepted")
              ? "intercepted by active protection"
              : hits > 0 || result === "suppress"
                ? "near miss, suppressed"
                : "missed";
    labels.set(shot, label);

    const base =
      (struck ? SUPPRESSION_FOR.hit : hits > 0 || result === "suppress" ? SUPPRESSION_FOR.suppress : SUPPRESSION_FOR.miss) +
      (damaged ? SUPPRESSION_FOR.damaged : 0);
    const firerUnit = units[firer.id];
    if (firerUnit?.engagement && firerUnit.engagement.targetId === target.id) {
      const e = firerUnit.engagement;
      const struckCount = struck ? 1 : 0;
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
      narrative: shot.outcome.event.narrative,
    });

    // Firing gives a position away to whoever it was aimed at, if they can see back.
    if (known(target.id, firer.id) === "none" && sees(target.position, firer.position)) {
      observe(target.id, firer.id, "partial");
      emit(target.id, "sighted", `muzzle flash: fired on from ${Math.round(shot.rangeM)} m (${firer.id})`, true);
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
    const by = firers
      .map((shot) => `${shot.firerId} (${Math.round(shot.rangeM)} m, ${labels.get(shot) ?? shot.outcome.event.result})`)
      .join(", ");
    const shooter = newAttacker.get(targetId);
    const did = shooter ? drill(targetId, shooter) : "";
    emit(targetId, "underFire", `fired on by ${by}${did}`, hurt || shooter != null);
    if (hurt) {
      const v = units[targetId].vehicles;
      emit(targetId, "hit", `vehicle knocked out: ${v.fit} of ${v.total} still fighting`, true);
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
