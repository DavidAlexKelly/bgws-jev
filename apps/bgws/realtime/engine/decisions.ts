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
import type { ForceElement } from "../../lib/state";
import { weaponFor } from "../../rules/turnLoop";
import { hitConditions, isAlive, knownEnemies, knownTo } from "./engine";
import { hitChance } from "./fire";
import {
  allowanceAt,
  compass,
  hullDownAgainst,
  hullDownSpot,
  offsetBy,
  positionsFor,
} from "./geometry";
import { aspectOf } from "./lethality";
import { chanceBand, compassWord, rangeBand, selfBeliefOf } from "./knowledge";
import { closePosition, type Effect } from "./options";
import type { RtConfig, RtEvent, RtOption, RtOrder, RtState, Trigger, UnitOrders } from "./types";
import { landOptions } from "../behaviour/actions/land";

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

export function nameOf(state: RtState, unitId: string, enemyId: string): string {
  const enemy = state.game.forceElements[enemyId];
  return knownTo(state.game, state, unitId, enemyId) === "full" && enemy
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
export function whereWords(from: LatLng, to: LatLng): string {
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
export function pullBackSpot(state: RtState, self: ForceElement, threat: LatLng | null, config: RtConfig): { at: LatLng; why: string } | null {
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
    isAlive(state.game, enemyId) && knownTo(state.game, state, id, enemyId) !== "none";
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
        Number(enemyId != null && knownTo(state.game, state, b.id, enemyId) !== "none") -
          Number(enemyId != null && knownTo(state.game, state, a.id, enemyId) !== "none") ||
        distanceM(a.position, self.position) - distanceM(b.position, self.position),
    )[0]?.id;
}

/** A request, carried by an option: sent when the option is taken. */
export function askFor(to: string, kind: "cover" | "fire", enemyId: string | undefined, text: string): NonNullable<RtOption["message"]> {
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
  return landOptions(state, id, point, event, config, context);
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

export function describeActivity(state: RtState, id: string): string {
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
