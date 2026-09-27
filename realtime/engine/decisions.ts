// ── bgws/realtime/engine/decisions.ts ──────────────────────────────────────
// The decision points: the only moments a unit's leader (Jev) is asked.
//
// docs/REALTIME_COMMAND_DESIGN.html §4. Each has a fixed trigger, a short
// fixed list of options in a stable order — generated here, so a decider can
// only pick something legal — and a rules fallback that follows the unit's
// actions on contact. An event that matches none of them asks nobody.
//
//   D0  new orders arrive while in a fight      D6  after a volley
//   D1  new contact                             D7  target lost
//   D2  fired on, shooter located               D8  fire not working
//   D3  fired on, shooter not located           D9  a friend needs help
//   D4  a waiting unit's trigger is met         D10 order step done or blocked
//   D5  an enemy's behaviour changed (a cue)    D11 rallied
//
// Options are worded for Jev in bands, not numbers ("likely to knock one out
// within a minute"); the exact figures ride along in `exact`, for the console.
// The standing orders' constraints are enforced here too: an option that
// would cross one of its lines, or fire against its rules of engagement, is
// never offered.

import { bearingDeg, distanceM, type LatLng } from "../../lib/board";
import { lineOfSight } from "../../lib/lineOfSight";
import { inCover } from "../../lib/proceduralTerrain";
import type { ForceElement } from "../../lib/state";
import { weaponFor } from "../../rules/turnLoop";
import {
  canHit,
  crosses,
  describeTrigger,
  hitConditions,
  hullDownFrom,
  isAlive,
  knownEnemies,
  knownTo,
  nearestCover,
} from "./engine";
import { hitChance } from "./fire";
import { allowanceAt, compass, hullDownAgainst, hullDownSpot, offsetBy, positionsFor } from "./geometry";
import { aspectOf } from "./lethality";
import { chanceBand, chanceLocatedAfter, compassWord, rangeBand, selfBeliefOf } from "./knowledge";
import { closePosition, damageEffect, describeEffect, engagedBy, type Effect } from "./options";
import type { RtConfig, RtEvent, RtOption, RtOrder, RtState, Trigger, UnitOrders } from "./types";

export type DecisionPoint = "D0" | "D1" | "D2" | "D3" | "D4" | "D5" | "D6" | "D7" | "D8" | "D9" | "D10" | "D11" | "D12";

export const DECISION_POINTS: Record<DecisionPoint, { title: string; question: string }> = {
  D0: { title: "New orders arrive", question: "New orders have come down while it is in a fight. How does it comply?" },
  D1: { title: "New contact", question: "It has found an enemy it did not know about. What now?" },
  D2: { title: "Fired on, shooter located", question: "It is under fire and can see who is firing. How does it answer?" },
  D3: { title: "Fired on, shooter not located", question: "It is under fire from a shooter it cannot see; it knows only the bearing. What now?" },
  D4: { title: "Trigger met", question: "The moment it was waiting for has come. Fire now?" },
  D5: { title: "Enemy behaviour changed", question: "An enemy it is watching from hiding did something that may mean it has been seen. What now?" },
  D6: { title: "After a volley", question: "It has fired on a target. Keep going, or change something before the enemy finds it?" },
  D7: { title: "Target lost", question: "Its target is out of sight or out of the fight. What next?" },
  D8: { title: "Fire not working", question: "It has been firing for minutes and knocked nothing out. Change something?" },
  D9: { title: "Friend needs help", question: "A friend close by is in trouble and this unit knows its attacker. Help?" },
  D10: { title: "Order step done or blocked", question: "Its current step is done or it cannot go on. What next?" },
  D11: { title: "Rallied", question: "It has pulled itself together and takes orders again. What now?" },
  D12: { title: "Request received", question: "A friend has asked it for help over the radio. How does it answer?" },
};

/**
 * The balance Jev is told at every decision, in plain words. The time-box
 * and the constraints make sure it cannot be argued away.
 */
export const BALANCE =
  "The intent comes first; keep the unit alive and able to carry it out; take a local opportunity " +
  "only when it doesn't cost the intent.";

/** Which decision point these events raise, and the event that raised it. Most pressing first. */
export function decisionPointOf(events: readonly RtEvent[]): { point: DecisionPoint; event: RtEvent } | null {
  const live = events.filter((event) => !event.info);
  const first = (test: (event: RtEvent) => boolean) => live.find(test);
  const fired = (event: RtEvent) => event.kind === "underFire" || event.kind === "hit";
  const table: [DecisionPoint, (event: RtEvent) => boolean][] = [
    ["D0", (e) => e.kind === "newOrders"],
    ["D2", (e) => fired(e) && e.located === true],
    ["D3", (e) => fired(e) && e.located === false],
    ["D4", (e) => e.kind === "triggerMet"],
    ["D12", (e) => e.kind === "request"],
    ["D5", (e) => e.kind === "cue"],
    ["D9", (e) => e.kind === "friendNeedsHelp"],
    ["D1", (e) => e.kind === "sighted" || e.kind === "contact"],
    ["D6", (e) => e.kind === "volley"],
    ["D8", (e) => e.kind === "ineffective"],
    ["D7", (e) => e.kind === "targetGone" || e.kind === "enemyBroke"],
    ["D10", (e) => e.kind === "arrived" || e.kind === "blocked" || e.kind === "outOfOrders" || e.kind === "idle"],
    ["D3", (e) => e.kind === "searchDone"],
    ["D11", (e) => e.kind === "rallied"],
  ];
  for (const [point, test] of table) {
    const event = first(test);
    if (event) return { point, event };
  }
  return null;
}

// ── Words ──────────────────────────────────────────────────────────────────

/** What fire would do, in words: the chance of knocking a vehicle out within a minute. */
export function effectWords(effect: Effect | null, it = "it"): string {
  const whose = it === "you" ? "one of yours" : "one of its vehicles";
  if (!effect || effect.perShot <= 0) {
    return effect?.strike && effect.strike.pPenetrate < 0.01
      ? `rounds cannot get through ${it === "you" ? "your" : "its"} armour from there`
      : `no chance of hurting ${it} from there`;
  }
  const bounce = effect.strike && effect.strike.pPenetrate < 0.3 ? " (most rounds will not get through)" : "";
  return `${chanceBand(effect.perMinute)} to knock out ${whose} within a minute${bounce}`;
}

function nameOf(state: RtState, unitId: string, enemyId: string): string {
  const enemy = state.game.forceElements[enemyId];
  return knownTo(state.game, state.units, unitId, enemyId) === "full" && enemy
    ? `${enemy.label} (${enemyId})`
    : `the contact ${enemyId}`;
}

/** How far a move goes, in words. */
export function distanceWords(m: number): string {
  if (m < 300) return "a short way";
  if (m < 800) return "a few hundred metres";
  if (m < 1500) return "about a kilometre";
  return "a long way";
}

/** Where a move goes, in words: "a short way to the south". */
function whereWords(from: LatLng, to: LatLng): string {
  return `${distanceWords(distanceM(from, to))} to the ${compassWord(bearingDeg(from, to))}`;
}

// ── Places code picks ──────────────────────────────────────────────────────

/** A point within `radius` the enemy at `from` cannot see, preferring short moves away from it. */
export function hiddenSpot(self: ForceElement, from: LatLng, config: RtConfig, radius = 700): LatLng | null {
  const away = bearingDeg(from, self.position);
  const points = [200, 400, radius]
    .flatMap((ring) => [-90, -45, 0, 45, 90].map((delta) => offsetBy(self.position, (away + delta + 360) % 360, ring)))
    .filter((point) => allowanceAt(self, point, config) > 0)
    .filter((point) => !lineOfSight(config.terrain, { from, to: point }).visible);
  return points.sort((a, b) => distanceM(self.position, a) - distanceM(self.position, b))[0] ?? null;
}

/**
 * A better place to shoot at `enemy` from, as code sees it: hull-down if it
 * is not already, else closer to effective range, else on its flank.
 */
export function betterShot(
  state: RtState,
  self: ForceElement,
  enemy: ForceElement,
  config: RtConfig,
): { at: LatLng; why: string } | null {
  if (self.targetClass === "armoured_vehicle" && !hullDownAgainst(self.position, enemy.position, config)) {
    const spot = hullDownSpot(self, enemy.position, config, 400);
    if (spot && distanceM(spot, self.position) > 5) return { at: spot, why: "hull-down" };
  }
  const known = knownEnemies(state, self.id);
  const close = closePosition(self, enemy, known, config);
  if (close) return { at: close.at, why: close.cover ? "closer, from cover" : "closer" };
  const flank = positionsFor(self, [enemy], config).find((p) => p.purpose === "flank");
  if (flank) return { at: flank.at, why: "on its flank" };
  return null;
}

/** Back towards friends: the nearest steady friend further from the threat, else a withdrawal out of its sight. */
function pullBackSpot(state: RtState, self: ForceElement, threat: LatLng | null, config: RtConfig): { at: LatLng; why: string } | null {
  const here = threat ? distanceM(self.position, threat) : 0;
  const friend = Object.values(state.game.forceElements)
    .filter((other) => other.side === self.side && other.id !== self.id && other.combatStrength > 0)
    .filter((other) => state.units[other.id]?.cohesion === "steady")
    .filter((other) => distanceM(other.position, self.position) <= 3000)
    .filter((other) => !threat || distanceM(other.position, threat) > here + 150)
    .sort((a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position))[0];
  if (friend) return { at: offsetBy(friend.position, bearingDeg(friend.position, self.position), 100), why: `back to ${friend.id}` };
  if (!threat) return null;
  const hidden = hiddenSpot(self, threat, config);
  return hidden ? { at: hidden, why: "out of its sight" } : null;
}

/** Concrete triggers to wait for against this enemy, most useful first (D1, D4). */
export function triggersFor(state: RtState, self: ForceElement, enemy: ForceElement, config: RtConfig): Trigger[] {
  const range = distanceM(self.position, enemy.position);
  const out: Trigger[] = [];
  const weapon = weaponFor(self, enemy, Math.min(range, 1000));
  if (weapon) {
    const conditions = hitConditions(state, self, enemy, config);
    const at = (r: number) => hitChance(weapon, enemy, r, conditions, config.timing);
    if (at(range) < 0.8) {
      for (let r = Math.floor(range / 100) * 100 - 100; r >= 200; r -= 100) {
        if (at(r) >= 0.8) {
          out.push({ kind: "hitChance", atLeast: 0.8, aboutM: r });
          break;
        }
      }
    }
  }
  if (range > 1100) out.push({ kind: "range", withinM: 800 });
  const aspect = aspectOf(enemy, self.position, config.ruleset);
  if (aspect === "front") out.push({ kind: "flank" });
  return out.slice(0, 2);
}

// ── What to resume ────────────────────────────────────────────────────────

/** Back to its orders: the phase it was on. Or, with no standing orders, its mission. */
export function resumeOrder(state: RtState, id: string): { order: RtOrder; summary: string } | null {
  const unit = state.units[id];
  const self = state.game.forceElements[id];
  if (!unit || !self) return null;
  if (unit.orders) {
    const phase = unit.orders.phases[unit.orders.phase];
    // A phase it found it cannot carry out is not offered again.
    if (!phase || unit.orders.blocked?.phase === unit.orders.phase) return null;
    // Already doing it: "back to its orders" would change nothing.
    if (unit.order.phase === unit.orders.phase) return null;
    return { order: { ...phase.order, phase: unit.orders.phase }, summary: `back to its orders: ${phase.label}` };
  }
  const { mission } = unit;
  if (mission.at && distanceM(self.position, mission.at) > (mission.task === "take" ? 300 : 150)) {
    return { order: { kind: "move", to: mission.at, mode: "tactical" }, summary: `back to its mission: ${mission.purpose}` };
  }
  return unit.order.kind === "overwatch" ? null : { order: { kind: "overwatch" }, summary: `back to its mission: ${mission.purpose}, on overwatch` };
}

/** The enemy it is fighting now: its target, or whoever is shooting at it. */
export function currentEnemy(state: RtState, id: string): string | null {
  const unit = state.units[id];
  if (!unit) return null;
  const e = unit.engagement;
  const known = (enemyId: string) =>
    isAlive(state.game, enemyId) && knownTo(state.game, state.units, id, enemyId) !== "none";
  if (e && state.time - e.lastShotAt <= 60 && known(e.targetId)) return e.targetId;
  const attacker = Object.entries(unit.attackers)
    .filter(([enemyId, at]) => state.time - at <= 60 && known(enemyId))
    .sort((a, b) => b[1] - a[1])[0];
  return attacker?.[0] ?? null;
}

/** In a fight: it fired, or was fired on, in the last minute, at or by an enemy it knows. */
export function inContact(state: RtState, id: string): boolean {
  return currentEnemy(state, id) != null;
}

// ── Asking friends ────────────────────────────────────────────────────────

/** Friends within this may be asked for help. */
const ASK_RADIUS_M = 3000;

/**
 * Who this unit would ask for help against `enemyId`: the friend its orders
 * say supports it, else the nearest steady friend in reach — preferring one
 * that knows the enemy.
 */
export function helperFor(state: RtState, id: string, enemyId?: string): string | undefined {
  const self = state.game.forceElements[id];
  if (!self) return undefined;
  const ready = Object.values(state.game.forceElements).filter(
    (f) => f.side === self.side && f.id !== id && f.combatStrength > 0 && state.units[f.id]?.cohesion === "steady",
  );
  const supporter = ready.find((f) => state.units[f.id]?.orders?.supports === id);
  if (supporter) return supporter.id;
  return ready
    .filter((f) => distanceM(f.position, self.position) <= ASK_RADIUS_M)
    .sort(
      (a, b) =>
        Number(enemyId != null && knownTo(state.game, state.units, b.id, enemyId) !== "none") -
          Number(enemyId != null && knownTo(state.game, state.units, a.id, enemyId) !== "none") ||
        distanceM(a.position, self.position) - distanceM(b.position, self.position),
    )[0]?.id;
}

/** A request, carried by an option: sent when the option is taken. */
function askFor(to: string, kind: "cover" | "fire", enemyId: string | undefined, text: string): NonNullable<RtOption["message"]> {
  return { kind: "request", to, text, request: { kind, ...(enemyId ? { enemyId } : {}) } };
}

// ── The options ───────────────────────────────────────────────────────────

export interface DecisionContext {
  /** D0: the orders waiting to take over. */
  orders?: UnitOrders;
}

/** The options at a decision point, in their fixed order. The first is always the safe default. */
export function optionsAt(
  state: RtState,
  id: string,
  point: DecisionPoint,
  event: RtEvent,
  config: RtConfig,
  context: DecisionContext = {},
): RtOption[] {
  const self = state.game.forceElements[id];
  const unit = state.units[id];
  if (!self || !unit || self.combatStrength <= 0) return [];
  const about = event.about ? state.game.forceElements[event.about] : undefined;
  const aboutAlive = about && about.combatStrength > 0 && about.side !== self.side ? about : undefined;
  const aboutKnown = aboutAlive && knownTo(state.game, state.units, id, aboutAlive.id) !== "none" ? aboutAlive : undefined;
  const move = (to: LatLng, then?: RtOrder, mode: "tactical" | "march" = "tactical"): RtOrder => ({
    kind: "move",
    to,
    mode,
    ...(then ? { then } : {}),
  });
  const keep = (summary: string): RtOption => ({ id: "keep", summary, order: unit.order });
  const resume = resumeOrder(state, id);
  const out: RtOption[] = [];

  /** Fire on an enemy now, worded with its odds. */
  const engage = (enemy: ForceElement, id_: string, verb: string): RtOption | null => {
    if (!canHit(self, enemy, enemy.position, config) || !allowedTarget(state, id, enemy.id)) return null;
    const effect = damageEffect(self, enemy, state, config);
    const others = engagedBy(state, self.side, enemy.id, id);
    return {
      id: id_,
      summary:
        `${verb} ${nameOf(state, id, enemy.id)} at ${rangeBand(distanceM(self.position, enemy.position))}: ` +
        `${effectWords(effect)}${others.length ? `; ${others.join(", ")} already firing on it` : ""}`,
      order: { kind: "engage", targetId: enemy.id },
      ...(effect ? { effect: effect.perMinute, penetrate: effect.strike?.pPenetrate, exact: describeEffect(effect) } : {}),
    };
  };
  /** A better place to shoot from. `always`: offered even if the odds say it is no better (D8: they have been wrong). */
  const better = (enemy: ForceElement, id_ = "better", always = false): RtOption | null => {
    if (!allowedTarget(state, id, enemy.id)) return null;
    const spot = betterShot(state, self, enemy, config);
    if (!spot) return null;
    const there = damageEffect(self, enemy, state, config, spot.at);
    // Only a better shot if it is better: safer (hull-down), clearly more
    // likely to tell, or with rounds that get through where they now bounce.
    const here = canHit(self, enemy, enemy.position, config) ? damageEffect(self, enemy, state, config) : null;
    const gain =
      (there?.perMinute ?? 0) > Math.max(0.05, (here?.perMinute ?? 0) * 1.2) ||
      (there?.strike?.pPenetrate ?? 0) > (here?.strike?.pPenetrate ?? 0) + 0.2;
    if (!always && spot.why !== "hull-down" && !gain) return null;
    return {
      id: id_,
      summary: `get a better shot (${spot.why}, ${whereWords(self.position, spot.at)}), then engage: from there ${effectWords(there)}`,
      order: move(spot.at, { kind: "engage", targetId: enemy.id }),
      ...(there ? { effect: there.perMinute, exact: describeEffect(there) } : {}),
    };
  };
  const pullBack = (threat: LatLng | null, id_ = "pullBack", summary = "pull back"): RtOption | null => {
    const spot = pullBackSpot(state, self, threat, config);
    if (!spot) return null;
    return { id: id_, summary: `${summary} (${spot.why}, ${whereWords(self.position, spot.at)})`, order: { kind: "withdraw", to: spot.at } };
  };
  const outOfSight = (enemy: ForceElement, id_ = "pullBack"): RtOption | null => {
    const spot = hiddenSpot(self, enemy.position, config);
    if (!spot) return null;
    return {
      id: id_,
      summary: `pull back out of ${enemy.id}'s sight (${whereWords(self.position, spot)})`,
      order: { kind: "withdraw", to: spot },
    };
  };
  const quiet = (summary = "stop firing and go quiet: stay still and watch"): RtOption => ({
    id: "quiet",
    summary,
    order: { kind: "observe" },
  });
  const shift = (except: string): RtOption | null => {
    const next = knownEnemies(state, id)
      .filter((enemy) => enemy.id !== except && canHit(self, enemy, enemy.position, config))
      .map((enemy) => ({ enemy, effect: damageEffect(self, enemy, state, config) }))
      .sort((a, b) => (b.effect?.perMinute ?? 0) - (a.effect?.perMinute ?? 0))[0];
    return next ? engage(next.enemy, "shift", "shift fire to") : null;
  };
  const push = (...options: (RtOption | null | undefined)[]) => {
    for (const option of options) if (option && !out.some((one) => one.id === option.id)) out.push(option);
  };

  switch (point) {
    case "D0": {
      const orders = context.orders;
      if (!orders || !orders.phases.length) break;
      const first: RtOrder = { ...orders.phases[0].order, phase: 0 };
      const enemyId = currentEnemy(state, id);
      const enemy = enemyId ? state.game.forceElements[enemyId] : undefined;
      const hide = enemy ? hiddenSpot(self, enemy.position, config) : null;
      const task = `"${orders.task}"`;
      if (orders.urgency === "now") {
        push({ id: "comply", summary: `break off at once and start ${task}`, order: first, orders });
        const cover = enemy
          ? Object.values(state.game.forceElements)
              .filter((f) => f.side === self.side && f.id !== id && f.combatStrength > 0 && state.units[f.id]?.cohesion === "steady")
              .filter((f) => distanceM(f.position, self.position) <= 1500 && canHit(f, enemy, enemy.position, config))
              .filter((f) => knownTo(state.game, state.units, f.id, enemy.id) !== "none")
              .sort((a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position))[0]
          : undefined;
        if (cover && enemy) {
          // Asked over the radio: whether and how it covers is its own call (D12).
          push({
            id: "covered",
            summary: `break off under covering fire: ask ${cover.id} to cover it while it pulls out to start ${task}`,
            order: first,
            orders,
            message: askFor(cover.id, "cover", enemy.id, `${id} asks ${cover.id} to cover it off against ${enemy.id}`),
          });
        }
        if (enemy && hide && canHit(self, enemy, enemy.position, config)) {
          push({
            id: "fireAndBack",
            summary: `fire one more volley at ${enemy.id}, move back out of its sight, then start ${task}`,
            order: { kind: "engage", targetId: enemy.id, volleys: 1, then: { kind: "withdraw", to: hide, then: first } },
            orders,
          });
        }
      } else {
        push({ id: "comply", summary: `comply now: start ${task}`, order: first, orders });
        if (enemy && canHit(self, enemy, enemy.position, config)) {
          push({
            id: "finish",
            summary: `finish this fight with ${enemy.id} first (until it is out of the fight or lost, at most two minutes), then start ${task}`,
            order: { kind: "engage", targetId: enemy.id, until: state.time + 120, then: first },
            orders,
          });
        }
        if (hide) {
          push({
            id: "breakContact",
            summary: `break contact (back out of sight, ${whereWords(self.position, hide)}), then start ${task}`,
            order: { kind: "withdraw", to: hide, then: first },
            orders,
          });
        }
      }
      break;
    }
    case "D1": {
      push(keep(`carry on (${describeActivity(state, id)})`));
      if (aboutKnown) {
        push(engage(aboutKnown, "engage", "engage now:"));
        if (allowedTarget(state, id, aboutKnown.id)) {
          for (const [index, trigger] of triggersFor(state, self, aboutKnown, config).entries()) {
            push(
              { id: `wait${index}`, summary: `hold still and wait until ${describeTrigger(trigger)}, then decide`, order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: false } },
              { id: `wait${index}:fire`, summary: `hold still and wait until ${describeTrigger(trigger)}, then fire at once`, order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: true } },
            );
          }
        }
        push(better(aboutKnown));
      }
      push({ id: "observe", summary: "observe and report, without firing", order: { kind: "observe" } });
      push(pullBack(aboutKnown?.position ?? null, "pullBack", "pull back to friends"));
      break;
    }
    case "D2": {
      if (!aboutKnown) break;
      push(engage(aboutKnown, "returnFire", "return fire from here on"));
      if (self.targetClass === "armoured_vehicle" && !hullDownFrom(state, self, aboutKnown.position, config)) {
        const spot = hullDownSpot(self, aboutKnown.position, config, 300);
        if (spot && distanceM(spot, self.position) > 5 && allowedTarget(state, id, aboutKnown.id)) {
          push({
            id: "hullDown",
            summary: `back into hull-down (${whereWords(self.position, spot)}), then return fire on ${aboutKnown.id}`,
            order: move(spot, { kind: "engage", targetId: aboutKnown.id }),
          });
        }
      }
      push(outOfSight(aboutKnown));
      // With a friend to ask: fire on it together, or pull back under its cover.
      const friend = helperFor(state, id, aboutKnown.id);
      if (friend) {
        const back = out.find((o) => o.id === "pullBack");
        if (back) {
          push({
            ...back,
            id: "pullBackCovered",
            summary: `${back.summary}, asking ${friend} to cover the move`,
            message: askFor(friend, "cover", aboutKnown.id, `${id} pulling back under fire from ${aboutKnown.id}: asks ${friend} to cover`),
          });
        }
        const answer = out.find((o) => o.id === "returnFire");
        if (answer) {
          push({
            ...answer,
            id: "callFire",
            summary: `${answer.summary}, and ask ${friend} to engage it too`,
            message: askFor(friend, "fire", aboutKnown.id, `${id} asks ${friend} to engage ${aboutKnown.id}`),
          });
        }
      }
      if (distanceM(self.position, aboutKnown.position) <= 1500 && allowedTarget(state, id, aboutKnown.id)) {
        push({
          id: "assault",
          summary: `assault ${nameOf(state, id, aboutKnown.id)}: close in firing, to point-blank`,
          order: { kind: "move", to: aboutKnown.position, mode: "assault" },
        });
      }
      push(quiet("hold still in cover and do not fire"));
      break;
    }
    case "D3": {
      const bearing = event.bearingDeg ?? unit.suspects[event.about ?? ""]?.bearingDeg;
      if (bearing == null) {
        push(keep(`carry on (${describeActivity(state, id)})`));
        break;
      }
      const words = compassWord(bearing);
      push({
        id: "search",
        summary: `search the ${words}: hold still and look for the shooter (spotting doubled that way for half a minute)`,
        order: { kind: "search", bearingDeg: bearing, until: state.time + 30 },
      });
      if (!inCover(config.terrain, self.position)) {
        const threat = offsetBy(self.position, bearing, 1500);
        const cover = nearestCover(self, config, 300, threat);
        if (cover) {
          push({
            id: "cover",
            summary: `move into cover away from the ${words} (${whereWords(self.position, cover)})`,
            order: move(cover, undefined, "march"),
          });
        }
      }
      const back = offsetBy(self.position, (bearing + 180) % 360, 500);
      if (allowanceAt(self, back, config) > 0) {
        push({ id: "pullBack", summary: `pull back, away from the ${words}`, order: { kind: "withdraw", to: back } });
      }
      push(keep(`carry on (${describeActivity(state, id)})`));
      break;
    }
    case "D4": {
      if (!aboutKnown || unit.order.kind !== "wait") break;
      push(engage(aboutKnown, "fire", "fire now on"));
      const range = distanceM(self.position, aboutKnown.position);
      if (range > 500) {
        const trigger: Trigger = { kind: "range", withinM: Math.max(300, Math.round((range * 0.6) / 100) * 100) };
        push({
          id: "closer",
          summary: `wait for a closer shot: until ${describeTrigger(trigger)}`,
          order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: false },
        });
      }
      push({ id: "letPass", summary: "let it pass and stay hidden", order: { kind: "observe" } });
      break;
    }
    case "D5": {
      if (!aboutAlive) break;
      push(aboutKnown ? engage(aboutKnown, "fireFirst", "fire first, while it still can, on") : null);
      push(keep("keep holding: it may be a coincidence"));
      const spot = hiddenSpot(self, aboutAlive.position, config, 400) ?? nearestCover(self, config, 300, aboutAlive.position);
      if (spot) {
        push({
          id: "relocate",
          summary: `relocate quietly (${whereWords(self.position, spot)}) and keep watching`,
          order: move(spot, { kind: "observe" }),
        });
      }
      push(outOfSight(aboutAlive));
      break;
    }
    case "D6": {
      if (!aboutKnown) break;
      const e = unit.engagement;
      const firedBack = e != null && (unit.incoming[aboutKnown.id]?.last ?? -Infinity) >= e.since;
      const their = state.units[aboutKnown.id];
      const dived = their?.history.some((m) => m.by === "crew" && e != null && m.time >= e.since && /dashing|backing/.test(m.chose));
      const moved = their != null && e != null && their.lastMovedAt >= e.since;
      const reaction = firedBack ? "it is firing back: it has found you" : dived ? "it went to ground" : moved ? "it has moved" : "no visible reaction yet";
      const located = firedBack
        ? 1
        : chanceLocatedAfter(distanceM(self.position, aboutKnown.position), e?.shots ?? 1, {
            shooterInCover: inCover(config.terrain, self.position),
            shooterHullDown: hullDownFrom(state, self, aboutKnown.position, config),
          });
      const odds = damageEffect(self, aboutKnown, state, config);
      push(
        keep(
          `keep firing on ${aboutKnown.id}: ${effectWords(odds)}; ${reaction}` +
            (firedBack ? "" : `; ${chanceBand(located)} that it has located you`),
        ),
      );
      const spot = hiddenSpot(self, aboutKnown.position, config, 400) ?? hullDownSpot(self, aboutKnown.position, config, 300);
      if (spot) {
        const friend = helperFor(state, id, aboutKnown.id);
        push({
          ...(friend
            ? { message: askFor(friend, "cover", aboutKnown.id, `${id} shifting position under ${aboutKnown.id}'s eye: asks ${friend} to cover`) }
            : {}),
          id: "fireAndMove",
          summary: `fire and move: one more volley, then shift position (${whereWords(self.position, spot)}) before it finds you, then engage again${
            friend ? `, asking ${friend} to cover the move` : ""
          }`,
          order: {
            kind: "engage",
            targetId: aboutKnown.id,
            volleys: 1,
            then: move(spot, { kind: "engage", targetId: aboutKnown.id }),
          },
        });
      }
      push(shift(aboutKnown.id));
      push(quiet());
      push(outOfSight(aboutKnown));
      out[0] = { ...out[0], exact: `P(located) ≈ ${Math.round(located * 100)}%; ${describeEffect(odds)}` };
      break;
    }
    case "D7": {
      const enemyId = event.about;
      const lkp = enemyId ? state.lastKnown[self.side]?.[enemyId] : undefined;
      const broken = enemyId ? state.units[enemyId]?.cohesion === "broken" : false;
      push(resume ? { id: "resume", summary: resume.summary, order: resume.order } : keep(`carry on (${describeActivity(state, id)})`));
      if (lkp || broken) push({ id: "watch", summary: `watch where ${enemyId} was last seen`, order: { kind: "observe" } });
      const target = enemyId ? state.game.forceElements[enemyId] : undefined;
      if (broken && target && target.combatStrength > 0 && allowedTarget(state, id, target.id)) {
        push({ id: "regain", summary: `follow ${enemyId} up: it has broken (assault after it)`, order: { kind: "move", to: target.position, mode: "assault" } });
      } else if (lkp && distanceM(self.position, lkp.at) > 300) {
        const to = offsetBy(self.position, bearingDeg(self.position, lkp.at), distanceM(self.position, lkp.at) * 0.5);
        push({ id: "regain", summary: `move to regain sight of ${enemyId} (${whereWords(self.position, to)})`, order: move(to, { kind: "observe" }) });
      }
      push(shift(enemyId ?? ""));
      break;
    }
    case "D8": {
      const targetId = unit.engagement?.targetId;
      const target = targetId ? state.game.forceElements[targetId] : undefined;
      if (target && target.combatStrength > 0) push(better(target, "better", true));
      push(shift(targetId ?? ""));
      push(quiet());
      if (target) push(outOfSight(target));
      const odds = target ? damageEffect(self, target, state, config) : null;
      push(keep(`keep at it: ${effectWords(odds)}`));
      break;
    }
    case "D9": {
      if (!aboutKnown) break;
      push(engage(aboutKnown, "help", "fire on its attacker,"));
      const spot = betterShot(state, self, aboutKnown, config);
      if (spot && allowedTarget(state, id, aboutKnown.id)) {
        push({
          id: "support",
          summary: `move to support (${spot.why}, ${whereWords(self.position, spot.at)}), then engage ${aboutKnown.id}`,
          order: move(spot.at, { kind: "engage", targetId: aboutKnown.id }),
        });
      }
      push(keep(`carry on (${describeActivity(state, id)})`));
      break;
    }
    case "D12": {
      const request = unit.requests.find((r) => r.id === event.requestId);
      if (!request) break;
      const enemy = request.enemyId ? state.game.forceElements[request.enemyId] : undefined;
      const known = enemy && enemy.combatStrength > 0 && knownTo(state.game, state.units, id, enemy.id) !== "none" ? enemy : undefined;
      const reply = (answer: "comply" | "cannot" | "partly", text: string) => ({
        kind: "reply" as const,
        to: request.from,
        text,
        reply: { requestId: request.id, answer },
      });
      const what = request.kind === "cover" ? `cover ${request.from}` : `fire on ${request.enemyId ?? "its attacker"}`;
      if (known && canHit(self, known, known.position, config) && allowedTarget(state, id, known.id)) {
        const effect = damageEffect(self, known, state, config);
        push({
          id: "comply",
          summary: `comply: ${what} — engage ${nameOf(state, id, known.id)} from here: ${effectWords(effect)}`,
          order: { kind: "engage", targetId: known.id },
          answers: request.id,
          message: reply("comply", `${id} to ${request.from}: complying, engaging ${known.id}`),
          ...(effect ? { effect: effect.perMinute, exact: describeEffect(effect) } : {}),
        });
      } else if (known && allowedTarget(state, id, known.id)) {
        const spot = betterShot(state, self, known, config);
        if (spot) {
          push({
            id: "comply",
            summary: `comply: ${what} — move to get a shot (${spot.why}, ${whereWords(self.position, spot.at)}), then engage ${known.id}`,
            order: move(spot.at, { kind: "engage", targetId: known.id }),
            answers: request.id,
            message: reply("comply", `${id} to ${request.from}: complying, moving to engage ${known.id}`),
          });
        }
      }
      if (unit.order.kind !== "overwatch") {
        push({
          id: "partly",
          summary: `partly: stay here on overwatch and fire at anything in reach, without moving`,
          order: { kind: "overwatch" },
          answers: request.id,
          message: reply("partly", `${id} to ${request.from}: can't move, covering from where it is`),
        });
      }
      const busy = currentEnemy(state, id);
      push({
        ...keep(`can't: carry on (${describeActivity(state, id)})${busy ? `, it is fighting ${busy}` : ""}`),
        answers: request.id,
        message: reply("cannot", `${id} to ${request.from}: can't${busy ? `, engaged with ${busy}` : ""}`),
      });
      break;
    }
    case "D10": {
      if (resume && event.kind !== "outOfOrders") push({ id: "resume", summary: resume.summary, order: resume.order });
      // Its current phase cannot be done: skip to the next one, if there is one.
      const o = unit.orders;
      if (o && o.blocked?.phase === o.phase && o.phase + 1 < o.phases.length) {
        const next = o.phases[o.phase + 1];
        push({
          id: "nextPhase",
          summary: `skip to the next step of its orders: ${next.label}`,
          order: { ...next.order, phase: o.phase + 1 },
          orders: { ...o, phase: o.phase + 1, blocked: undefined },
        });
      }
      if (unit.order.kind !== "hold") push({ id: "hold", summary: "hold here", order: { kind: "hold" } });
      push({ id: "overwatch", summary: "overwatch from here: fire at anything in reach", order: { kind: "overwatch" } });
      if (unit.order.kind === "hold") push(keep("hold here"));
      break;
    }
    case "D11": {
      if (resume) push({ id: "resume", summary: `rejoin: ${resume.summary}`, order: resume.order });
      push({ id: "hold", summary: "hold and recover", order: { kind: "hold" } });
      const friend = Object.values(state.game.forceElements)
        .filter((f) => f.side === self.side && f.id !== id && f.combatStrength > 0 && state.units[f.id]?.cohesion === "steady")
        .sort((a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position))[0];
      if (friend && distanceM(friend.position, self.position) > 200) {
        push({
          id: "join",
          summary: `join the nearest friends, ${friend.id} (${whereWords(self.position, friend.position)})`,
          order: move(offsetBy(friend.position, bearingDeg(friend.position, self.position), 100)),
        });
      }
      break;
    }
  }
  if (out.length === 0) push(keep(`carry on (${describeActivity(state, id)})`));
  return withinConstraints(out, unit.orders ?? context.orders);
}

/** Drop options that would cross one of the orders' lines. "keep" always stays. */
function withinConstraints(options: RtOption[], orders: UnitOrders | null | undefined): RtOption[] {
  const lines = orders?.boundaries ?? [];
  if (!lines.length) return options;
  const destinations = (order: RtOrder | undefined): LatLng[] =>
    !order
      ? []
      : order.kind === "move" || order.kind === "withdraw"
        ? [order.to, ...destinations(order.then)]
        : order.kind === "engage"
          ? destinations(order.then)
          : [];
  return options.filter((option) => option.id === "keep" || destinations(option.order).every((at) => !crosses(lines, at)));
}

/**
 * May it open fire on this enemy, by its rules of engagement? Self-defence
 * (the enemy fired on it) always may, unless its orders are "never".
 */
export function allowedTarget(state: RtState, id: string, enemyId: string): boolean {
  const unit = state.units[id];
  const self = state.game.forceElements[id];
  if (!unit || !self) return false;
  const roe = unit.roe;
  if (roe === "never") return false;
  if (state.time - (unit.attackers[enemyId] ?? -Infinity) <= 60) return true;
  if (roe === "ifFiredUpon") return state.time - (state.lastFiredOn[self.side][enemyId] ?? -Infinity) <= 60;
  return true;
}

function describeActivity(state: RtState, id: string): string {
  const unit = state.units[id];
  const order = unit.order;
  const self = state.game.forceElements[id];
  if (order.kind === "move" || order.kind === "withdraw") {
    return `${order.kind === "move" ? `moving ${order.mode === "march" ? "fast" : order.mode}` : "withdrawing"}, ${rangeBand(
      distanceM(self.position, order.to),
    )} to go ${compass(self.position, order.to)}`;
  }
  if (order.kind === "engage") return `engaging ${order.targetId}`;
  if (order.kind === "wait") return `waiting on ${order.targetId}`;
  return order.kind === "search" ? "searching" : order.kind === "observe" ? "observing" : order.kind;
}

// ── The rules' fallback ────────────────────────────────────────────────────

/**
 * What the rules take at a decision point, following the order's actions on
 * contact. Used when there is no Jev, when Jev is unsure, and when it cannot
 * be reached.
 */
export function ruleFallback(state: RtState, id: string, point: DecisionPoint, options: readonly RtOption[]): string {
  const unit = state.units[id];
  const onContact = unit?.orders?.onContact ?? (unit?.roe === "never" ? "observe" : "engage");
  const has = (optionId: string) => options.some((option) => option.id === optionId);
  const pick = (...ids: string[]) => ids.find(has) ?? options[0]?.id ?? "keep";
  const inOpen = (): boolean => unit?.posture !== "hullDown";

  switch (point) {
    case "D0":
      return unit?.orders && options.some((o) => o.orders?.urgency === "now") ? pick("comply") : pick("finish", "comply");
    case "D1":
      if (onContact === "engage") {
        const engage = options.find((o) => o.id === "engage");
        const p = engage?.effect ?? 0;
        // Still and unseen, with a poor shot or one that mostly bounces off:
        // lie in wait for a better one — its flank if the rounds bounce —
        // rather than give the position away. An advancing unit fights.
        const still = unit != null && !["move", "withdraw"].includes(unit.order.kind);
        const bounces = (engage?.penetrate ?? 1) < 0.3;
        if (still && selfBeliefOf(unit, state.time) === "unobserved" && (p < 0.5 || bounces)) {
          const waits = options.filter((o) => o.order.kind === "wait" && o.order.autoFire);
          const flank = waits.find((o) => o.order.kind === "wait" && o.order.trigger.kind === "flank");
          const wait = (bounces ? flank : undefined) ?? waits[0];
          if (wait) return wait.id;
        }
        if (p >= 0.1) return "engage";
        return pick("better", "engage", "keep");
      }
      if (onContact === "observe") return pick("observe", "keep");
      if (onContact === "avoid") return pick("pullBack", "observe");
      return pick("keep");
    case "D2":
      if (onContact === "avoid") return pick("pullBackCovered", "pullBack", "quiet");
      if (onContact === "observe") return pick("quiet", "pullBack");
      // Fight back, and bring a friend in on it when there is one to ask.
      return inOpen() ? pick("hullDown", "callFire", "returnFire", "pullBack") : pick("callFire", "returnFire", "pullBack");
    case "D3":
      if (onContact === "avoid") return pick("pullBack", "cover");
      if (onContact === "bypass") return pick("keep");
      return pick("cover", "search", "pullBack");
    case "D4":
      return onContact === "engage" || onContact === "bypass" ? pick("fire") : pick("letPass");
    case "D5":
      if (onContact === "avoid") return pick("pullBack", "relocate");
      if (onContact === "engage") {
        const first = options.find((o) => o.id === "fireFirst");
        return first && (first.effect ?? 0) >= 0.3 ? "fireFirst" : pick("keep");
      }
      return pick("keep");
    case "D6":
      if (onContact === "avoid") return pick("pullBack", "quiet");
      if (onContact === "observe") return pick("quiet");
      return pick("keep");
    case "D7":
      return onContact === "engage" ? pick("shift", "resume", "watch") : pick("resume", "watch");
    case "D8":
      return onContact === "avoid" ? pick("pullBack", "quiet") : pick("better", "shift", "keep");
    case "D9":
      return onContact === "engage" ? pick("help", "support", "keep") : pick("keep");
    case "D10":
      return pick("resume", "nextPhase", "overwatch");
    case "D12": {
      // Help a friend who asks, unless told to avoid a fight or already
      // fighting its own battle; supporting it is part of its orders if it
      // was told to support that unit.
      const request = unit?.requests.find((r) => options.some((o) => o.answers === r.id));
      const supporting = request != null && unit?.orders?.supports === request.from;
      if (onContact === "avoid" && !supporting) return pick("partly", "keep");
      const busy = currentEnemy(state, id);
      if (busy && !supporting && !options.some((o) => o.id === "comply" && o.order.kind === "engage")) return pick("partly", "keep");
      return pick("comply", "partly", "keep");
    }
    case "D11":
      return pick("resume", "hold");
  }
}
