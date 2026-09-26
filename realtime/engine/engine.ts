// ── bgws/realtime/engine/engine.ts ─────────────────────────────────────────
// One simulated second at a time: the autopilot.
//
// `tick` is pure. Given the board and the rules it returns the next board,
// the events that happened to units, and the shots that were fired. It never
// decides anything — every unit does what its current order says — so a
// decider can be swapped, faked or left out without the engine noticing.
//
// Each tick, in order:
//   1. broken units turn and withdraw (they are no longer fighting)
//   2. movement, at the ground's speed; arriving, getting stuck and walking
//      into an identified enemy's reach are events
//   3. sighting, staggered so each observer tries each enemy every
//      `sightingIntervalS`; contact fades after `contactMemoryS` unseen
//   4. fire, one shot per `engagementCycleS` per unit, by its order and ROE
//   5. morale recovers after a quiet spell (never from broken)
//   6. is anyone left fighting? has time run out?
//
// Everything random comes from `config.rng`, in a fixed order, so the same
// seed and the same decisions give the same game.

import { bearingDeg, distanceM, LOS_CAP_M, type LatLng } from "../../lib/board";
import { lineOfSight } from "../../lib/lineOfSight";
import { inCover } from "../../lib/proceduralTerrain";
import {
  forceElementsOf,
  improveMorale,
  opposing,
  sightingOf,
  type ForceElement,
  type GameState,
  type Morale,
  type Side,
  type SightingLevel,
} from "../../lib/state";
import { applyEffects } from "../../rules/apply";
import { canAdvance, canEngage, resolveDirectFire, resolveSighting } from "../../rules/resolvers";
import { isFlankShot, weaponFor } from "../../rules/turnLoop";
import { judgeVictory } from "../../rules/victory";
import { bearingRoutePlanner } from "../../lib/routePlan";
import { allowanceAt, offsetBy, towards } from "./geometry";
import { AUTO_SIGHT_M, CLOSE_CONTACT_M, metresPerTick } from "./timing";
import type { RtConfig, RtEvent, RtOrder, RtShot, RtState, RtUnit } from "./types";

const SIGHT_RANK: Record<SightingLevel, number> = { none: 0, veryPartial: 1, partial: 2, full: 3 };
/** How long "fired upon" lasts, for the ifFiredUpon rule. */
const FIRED_UPON_MEMORY_S = 60;
/** Who counts as "close by" when a friend is lost. */
const FRIEND_RADIUS_M = 1000;
/** Look again this often when a unit's weapon is ready and there is nothing to shoot. */
const RETARGET_S = 5;

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

export function isAlive(game: GameState, id: string): boolean {
  return (game.forceElements[id]?.combatStrength ?? 0) > 0;
}

/** Still in the fight: alive and not broken. */
export function isFighting(fe: ForceElement): boolean {
  return fe.combatStrength > 0 && fe.morale !== "broken";
}

/** Can this enemy hit that element, from where each of them is? */
export function canHit(enemy: ForceElement, target: ForceElement, at: LatLng, config: RtConfig): boolean {
  const range = distanceM(enemy.position, at);
  if (range > LOS_CAP_M) return false;
  if (!weaponFor(enemy, target, range)) return false;
  return lineOfSight(config.terrain, { from: enemy.position, to: at }).visible;
}

/** Identified enemies that can already reach this element where it stands. */
export function exposureOf(state: RtState, id: string, config: RtConfig): string[] {
  const fe = state.game.forceElements[id];
  if (!fe) return [];
  return forceElementsOf(state.game, opposing(fe.side))
    .filter((enemy) => enemy.combatStrength > 0)
    .filter((enemy) => sightingOf(state.game, fe.side, enemy.id) === "full")
    .filter((enemy) => canHit(enemy, fe, fe.position, config))
    .map((enemy) => enemy.id);
}

/** A fresh real-time game from a placed board. Everyone holds until ordered. */
export function createRealtimeState(game: GameState): RtState {
  const units: Record<string, RtUnit> = {};
  for (const fe of Object.values(game.forceElements)) {
    units[fe.id] = {
      order: { kind: "hold" },
      roe: "withinShortRange",
      weaponReadyAt: 0,
      lastMovedAt: -Infinity,
      lastHurtAt: -Infinity,
      exposedTo: [],
    };
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
    game: { ...game, phase: "arcAction" },
    units,
    lastSeen,
    lastFiredOn: { blue: {}, red: {} },
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

/** Give a unit a new order. Resets what it is already exposed to. */
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
  const next = { ...state, units: { ...state.units, [id]: { ...unit, ...extra, order: planned } } };
  return {
    ...next,
    units: { ...next.units, [id]: { ...next.units[id], exposedTo: exposureOf(next, id, config) } },
  };
}

function describeOrder(order: RtOrder): string {
  switch (order.kind) {
    case "move":
      return "moving";
    case "withdraw":
      return "withdrawing";
    case "engage":
      return `engaging ${order.targetId}`;
    default:
      return order.kind;
  }
}

export { describeOrder };

export function tick(prev: RtState, config: RtConfig): TickResult {
  if (prev.over) return { state: prev, events: [], shots: [] };

  const timing = config.timing;
  const time = prev.time + timing.tickS;
  const turn = Math.floor(time / timing.turnS) + 1;
  let game: GameState = { ...prev.game, turn };
  const units: Record<string, RtUnit> = { ...prev.units };
  const lastSeen = { blue: { ...prev.lastSeen.blue }, red: { ...prev.lastSeen.red } };
  const lastFiredOn = { blue: { ...prev.lastFiredOn.blue }, red: { ...prev.lastFiredOn.red } };
  const events: RtEvent[] = [];
  const shots: RtShot[] = [];

  const ids = Object.keys(game.forceElements).sort();
  const fe = (id: string) => game.forceElements[id];
  const setFe = (id: string, patch: Partial<ForceElement>) => {
    game = { ...game, forceElements: { ...game.forceElements, [id]: { ...fe(id), ...patch } } };
  };
  const setUnit = (id: string, patch: Partial<RtUnit>) => {
    units[id] = { ...units[id], ...patch };
  };
  const emit = (unitId: string, kind: RtEvent["kind"], detail: string, severe = false) => {
    if (units[unitId] && isAlive(game, unitId)) events.push({ time, unitId, kind, detail, severe });
  };
  const friendsNear = (of: ForceElement) =>
    ids.filter(
      (id) =>
        id !== of.id &&
        fe(id).side === of.side &&
        isAlive(game, id) &&
        distanceM(fe(id).position, of.position) <= FRIEND_RADIUS_M,
    );

  // ── 1. The broken withdraw ────────────────────────────────────────────────
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0 || self.morale !== "broken") continue;
    if (unit.order.kind === "withdraw") continue;
    const enemies = forceElementsOf(game, opposing(self.side)).filter(
      (enemy) => enemy.combatStrength > 0 && sightingOf(game, self.side, enemy.id) !== "none",
    );
    const nearest = enemies.sort(
      (a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position),
    )[0];
    const away = nearest
      ? (bearingDeg(nearest.position, self.position) + 360) % 360
      : game.objectives
        ? (bearingDeg(game.objectives[self.side], self.position) + 360) % 360
        : 180;
    setUnit(id, {
      order: routed({ kind: "withdraw", to: offsetBy(self.position, away, 1000) }, self, config),
      roe: "never",
    });
  }

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
      if (self.markers.includes("moved") && time - unit.lastMovedAt > 60) {
        setFe(id, { markers: self.markers.filter((m) => m !== "moved") });
      }
      continue;
    }
    // An advance needs a steady unit; a withdrawal only needs to be alive.
    if (order.kind === "move" && !canAdvance(self.morale)) continue;

    // Head for the next waypoint of the route, or straight for the goal.
    const route = order.route ?? [];
    const aim = route[0] ?? order.to;
    const step = metresPerTick(allowanceAt(self, self.position, config), timing);
    const next = step > 0 ? towards(self.position, aim, step) : self.position;
    if (step <= 0 || allowanceAt(self, next, config) <= 0) {
      setUnit(id, { order: { kind: "hold" } });
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
      setUnit(id, { order: { kind: "hold" } });
      emit(id, "arrived", order.kind === "withdraw" ? "withdrawal complete" : "reached its destination");
      continue;
    }
    moving.push(id);
  }

  // Now that everyone has moved: who has run into whom, and who has walked
  // into an identified enemy's reach.
  const contacts: { id: string; enemyId: string; range: number }[] = [];
  for (const id of moving) {
    const self = fe(id);
    const order = units[id].order;

    // Run into the enemy and you stop: nobody drives through a troop at
    // point-blank range. A withdrawal keeps going — that is what it is for.
    if (order.kind === "move") {
      const close = forceElementsOf(game, opposing(self.side))
        .filter((enemy) => enemy.combatStrength > 0)
        .map((enemy) => ({ enemy, range: distanceM(enemy.position, self.position) }))
        .filter(({ range }) => range <= CLOSE_CONTACT_M)
        .filter(({ enemy }) => lineOfSight(config.terrain, { from: self.position, to: enemy.position }).visible)
        .sort((a, b) => a.range - b.range)[0];
      if (close) {
        contacts.push({ id, enemyId: close.enemy.id, range: close.range });
        continue;
      }
    }

    // Walking into an identified enemy's reach, once per enemy per order.
    for (const enemy of forceElementsOf(game, opposing(self.side))) {
      if (enemy.combatStrength <= 0) continue;
      if (units[id].exposedTo.includes(enemy.id)) continue;
      if (sightingOf(game, self.side, enemy.id) !== "full") continue;
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
    game = applyEffects(game, [{ kind: "sighting", viewer: fe(id).side, feId: enemyId, to: "full" }]);
    lastSeen[fe(id).side][enemyId] = time;
    setUnit(id, { order: { kind: "engage", targetId: enemyId } });
    emit(id, "contact", `ran into ${enemy.label} (${enemyId}) at ${Math.round(range)} m and halted`, true);
  }

  // ── 3. Sighting ───────────────────────────────────────────────────────────
  for (const side of ["blue", "red"] as const) {
    const observers = ids.filter((id) => fe(id).side === side && isAlive(game, id));
    const enemies = ids.filter((id) => fe(id).side !== side && isAlive(game, id));

    for (const enemyId of enemies) {
      for (const observerId of observers) {
        const observer = fe(observerId);
        const enemy = fe(enemyId);

        // Close enough that nobody fails to see it: no roll, every tick.
        if (distanceM(observer.position, enemy.position) <= AUTO_SIGHT_M) {
          if (lineOfSight(config.terrain, { from: observer.position, to: enemy.position }).visible) {
            lastSeen[side][enemyId] = time;
            const was = sightingOf(game, side, enemyId);
            if (was !== "full") {
              game = applyEffects(game, [{ kind: "sighting", viewer: side, feId: enemyId, to: "full" }]);
              if (was === "none") {
                emit(observerId, "sighted", `${enemy.label} (${enemyId}) at ${Math.round(distanceM(observer.position, enemy.position))} m, close`, true);
              }
            }
          }
          continue;
        }

        // Seeded per game: an unseeded stagger gave the same side the earlier
        // look in every game, whatever the seed — enough to tip identical
        // engagements 44 to 15.
        if (
          (time + stagger(`${config.rng.seed}:${observerId}>${enemyId}`, timing.sightingIntervalS)) %
            timing.sightingIntervalS !==
          0
        ) {
          continue;
        }
        if (distanceM(observer.position, enemy.position) > LOS_CAP_M) continue;
        if (!lineOfSight(config.terrain, { from: observer.position, to: enemy.position }).visible) continue;

        lastSeen[side][enemyId] = time;
        const before = sightingOf(game, side, enemyId);
        if (before === "full") continue;

        const outcome = resolveSighting(
          observer,
          enemy,
          { targetInCover: inCover(config.terrain, enemy.position) },
          config.ruleset,
          config.rng,
          turn,
          "arcAction",
          side,
        );
        const found = outcome.effects.find((effect) => effect.kind === "sighting");
        const level = (found && found.kind === "sighting" ? found.to : "none") as SightingLevel;
        if (SIGHT_RANK[level] <= SIGHT_RANK[before]) continue;

        game = applyEffects(game, [{ kind: "sighting", viewer: side, feId: enemyId, to: level }]);
        if (before === "none") {
          // A new enemy is worth a decision at once, not after the cooldown.
          emit(
            observerId,
            "sighted",
            `${level === "full" ? enemy.label : "an unidentified contact"} (${enemyId}) at ${Math.round(
              distanceM(observer.position, enemy.position),
            )} m`,
            true,
          );
        }
      }
    }

    // Contact fades when nobody has seen it for a while.
    for (const enemyId of enemies) {
      if (sightingOf(game, side, enemyId) === "none") continue;
      if (time - (lastSeen[side][enemyId] ?? -Infinity) <= timing.contactMemoryS) continue;
      game = applyEffects(game, [{ kind: "sighting", viewer: side, feId: enemyId, to: "none" }]);
      for (const id of observers) {
        const order = units[id]?.order;
        if (order?.kind === "engage" && order.targetId === enemyId) {
          setUnit(id, { order: { kind: "hold" } });
          emit(id, "targetGone", `lost sight of ${enemyId}`);
        }
      }
    }
  }

  // ── 4. Fire ───────────────────────────────────────────────────────────────
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
    if (!unit || self.combatStrength <= 0 || !canEngage(self.morale)) continue;
    if (unit.weaponReadyAt > time) continue;
    const order = unit.order;
    // A withdrawal does not stop to fight. A move fires ON the move, within
    // its rules of engagement and at the fire table's moving penalty — two
    // columns that meet exchange fire rather than drive past each other.
    if (order.kind === "withdraw") continue;

    const reachable = forceElementsOf(snapshot, opposing(self.side))
      .filter((enemy) => enemy.combatStrength > 0)
      .filter((enemy) => sightingOf(snapshot, self.side, enemy.id) !== "none")
      .filter((enemy) => canHit(self, enemy, enemy.position, config));

    let target: ForceElement | undefined;
    if (order.kind === "engage") {
      target = reachable.find((enemy) => enemy.id === order.targetId);
      if (!target) {
        const gone = !isAlive(snapshot, order.targetId);
        if (gone || sightingOf(snapshot, self.side, order.targetId) === "none") {
          setUnit(id, { order: { kind: "hold" } });
          emit(id, "targetGone", gone ? `${order.targetId} is destroyed` : `lost sight of ${order.targetId}`);
        }
      }
    }
    if (!target && unit.roe !== "never") {
      const roe = order.kind === "overwatch" ? "always" : unit.roe;
      const allowed = reachable.filter((enemy) => {
        const range = distanceM(self.position, enemy.position);
        if (roe === "always") return true;
        if (roe === "ifFiredUpon") {
          return time - (lastFiredOn[self.side][enemy.id] ?? -Infinity) <= FIRED_UPON_MEMORY_S;
        }
        const weapon = weaponFor(self, enemy, range);
        return weapon != null && range <= weapon.shortRangeM;
      });
      target = allowed.sort(
        (a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position),
      )[0];
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
          targetInCover: inCover(config.terrain, target.position),
          flank: isFlankShot([self], target, config.ruleset),
        },
        config.ruleset,
        config.rng,
        turn,
        "arcAction",
      ),
    });
    setUnit(id, { weaponReadyAt: time + timing.engagementCycleS });
  }

  // Apply them all. Effects compose: two hits on one target this tick are
  // both taken, from the same starting strength.
  for (const shot of planned) {
    const firer = snapshot.forceElements[shot.firerId];
    const target = fe(shot.targetId);
    if (target.combatStrength <= 0) continue;
    game = applyEffects(game, shot.outcome.effects);
    lastFiredOn[target.side][firer.id] = time;
    shots.push({
      time,
      firerId: firer.id,
      targetId: target.id,
      result: shot.outcome.event.result,
      narrative: shot.outcome.event.narrative,
    });

    // Firing gives a position away to whoever it was aimed at, if they can see back.
    if (
      sightingOf(game, target.side, firer.id) === "none" &&
      lineOfSight(config.terrain, { from: target.position, to: firer.position }).visible
    ) {
      game = applyEffects(game, [{ kind: "sighting", viewer: target.side, feId: firer.id, to: "partial" }]);
      lastSeen[target.side][firer.id] = time;
      emit(target.id, "sighted", `muzzle flash: fired on from ${Math.round(shot.rangeM)} m (${firer.id})`, true);
    }
  }

  // What the fire did to each target, compared with where it started.
  for (const targetId of [...new Set(planned.map((shot) => shot.targetId))].sort()) {
    const before = snapshot.forceElements[targetId];
    const after = fe(targetId);
    const firers = planned.filter((shot) => shot.targetId === targetId);
    const hurt = after.combatStrength < before.combatStrength;
    const shaken = after.morale !== before.morale;
    if (hurt || shaken) setUnit(targetId, { lastHurtAt: time });

    if (after.combatStrength <= 0) {
      for (const friend of friendsNear(after)) {
        emit(friend, "friendLost", `${after.id} destroyed ${Math.round(distanceM(fe(friend).position, after.position))} m away`, true);
      }
      continue;
    }
    const by = firers
      .map((shot) => `${shot.firerId} (${Math.round(shot.rangeM)} m, ${shot.outcome.event.result})`)
      .join(", ");
    emit(targetId, "underFire", `fired on by ${by}`, hurt);
    if (hurt) emit(targetId, "hit", `lost strength: ${after.combatStrength}/${after.combatStrengthStart}`, true);
    if (shaken) {
      const bad = after.morale === "disrupted" || after.morale === "broken";
      emit(targetId, "moraleDrop", `now ${after.morale}`, bad);
      if (after.morale === "broken") {
        for (const friend of friendsNear(after)) {
          emit(friend, "friendLost", `${after.id} broken ${Math.round(distanceM(fe(friend).position, after.position))} m away`, true);
        }
      }
    }
  }

  // ── 5. Morale recovers after a quiet spell ────────────────────────────────
  for (const id of ids) {
    const self = fe(id);
    const unit = units[id];
    if (!unit || self.combatStrength <= 0) continue;
    if (self.morale === "good" || self.morale === "broken") continue;
    if (time - unit.lastHurtAt < timing.recoveryS) continue;
    setFe(id, { morale: improveMorale(self.morale as Morale) });
    setUnit(id, { lastHurtAt: time });
  }

  let state: RtState = { ...prev, time, game, units, lastSeen, lastFiredOn };

  // ── 6. Is it over? ────────────────────────────────────────────────────────
  const fighting = (side: Side) => forceElementsOf(game, side).some(isFighting);
  const blue = fighting("blue");
  const red = fighting("red");
  if (!blue || !red) {
    state = {
      ...state,
      over: {
        winner: blue ? "blue" : red ? "red" : null,
        reason: !blue && !red ? "both sides broken" : `${blue ? "red" : "blue"} is no longer fighting`,
      },
    };
  } else if (time >= timing.maxDurationS) {
    const verdict = judgeVictory(game, config.ruleset);
    state = { ...state, over: { winner: verdict.winner, reason: "time limit" } };
  }

  return { state, events, shots };
}
