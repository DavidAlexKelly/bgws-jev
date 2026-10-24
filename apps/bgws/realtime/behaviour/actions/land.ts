// ── behaviour/actions/land.ts ────────────────────────────────────────────
// The land action library: everything a land unit can be offered at a
// decision point, and the order it is offered in.
//
// Moved here UNCHANGED from engine/decisions.ts (decision-graph step 2,
// docs/plan/DECISION_FLOW_GRAPHS.md §7). engine/decisions.ts's optionsAt now
// just calls landOptions, and the decision golden test
// (engine/decisions.golden.test.ts) proves nothing a unit is offered changed.
// The next step splits this switch into named actions that behaviour
// templates (../templates) refer to.

import { bearingDeg, distanceM, type LatLng } from "../../../lib/board";
import { inCover } from "../../../lib/proceduralTerrain";
import type { ForceElement } from "../../../lib/state";
import {
  canHit,
  crosses,
  describeTrigger,
  hullDownFrom,
  knownEnemies,
  knownTo,
  nearestCover,
} from "../../engine/engine";
import { allowanceAt, hullDownSpot, offsetBy } from "../../engine/geometry";
import { chanceBand, chanceLocatedAfter, compassWord, rangeBand } from "../../engine/knowledge";
import { damageEffect, describeEffect, engagedBy } from "../../engine/options";
import type {
  RtConfig,
  RtEvent,
  RtOption,
  RtOrder,
  RtState,
  Trigger,
  UnitOrders,
} from "../../engine/types";
import {
  allowedTarget,
  askFor,
  betterShot,
  currentEnemy,
  type DecisionContext,
  type DecisionPoint,
  describeActivity,
  effectWords,
  helperFor,
  hiddenSpot,
  nameOf,
  pullBackSpot,
  resumeOrder,
  triggersFor,
  whereWords,
} from "../../engine/decisions";
import { DEFAULT_TEMPLATE_ID, defaultTemplateForClass } from "../archetypes";
import { LAND_FACTS, type LandFact } from "../facts";
import { buildRegistry, bundledTemplateFiles, type TemplateFiles } from "../load";
import type { Bias, FactTable, FactValue, ResolvedTemplate, Template, WalkResult } from "../types";
import { resolveTemplate, walk } from "../walk";

/**
 * The land actions a template may name: one per kind of option a land unit
 * can be offered. core/default.json lists them per decision point, in order.
 */
export const LAND_ACTIONS: readonly string[] = [
  "land.complyWithOrders", "land.breakOffCovered", "land.fireAndBack", "land.finishFightFirst",
  "land.breakContactFirst", "land.carryOn", "land.engageNow", "land.waitFor",
  "land.betterPosition", "land.observe", "land.pullBackToFriends", "land.returnFire",
  "land.hullDownThenFire", "land.outOfSight", "land.askFriendForHelp", "land.assault",
  "land.goQuietInCover", "land.searchBearing", "land.coverFromBearing", "land.pullBackFromBearing",
  "land.fireNow", "land.waitCloser", "land.letPass", "land.fireFirst",
  "land.keepHolding", "land.relocateQuietly", "land.slipAway", "land.keepFiring",
  "land.fireAndMove", "land.shiftFire", "land.goQuiet", "land.resumeOrCarryOn",
  "land.watchLastSeen", "land.regainContact", "land.shiftFireFromLost", "land.betterShotOnTarget",
  "land.shiftFireFromTarget", "land.stopFiring", "land.outOfTargetSight", "land.keepAtIt",
  "land.helpFriend", "land.moveToSupport", "land.complyWithRequest", "land.partlyOverwatch",
  "land.cannotComply", "land.resumeOrders", "land.skipToNextPhase", "land.holdHere",
  "land.overwatchHere", "land.keepHoldingHere", "land.rejoin", "land.holdAndRecover",
  "land.joinNearestFriends",
];

type Registry = Map<string, Template>;
const NO_EXTRA: readonly Template[] = [];
const registries = new WeakMap<object, Registry>();
/**
 * The bundled templates plus a game's user templates (RtConfig.behaviourTemplates),
 * validated once per set. An invalid user set is ignored (bundled only): a bad
 * template can cost units their custom behaviour, never break a game.
 */
function registryFor(extra: readonly Template[] = NO_EXTRA): Registry {
  let reg = registries.get(extra);
  if (!reg) {
    const files: TemplateFiles = { ...bundledTemplateFiles() };
    for (const t of extra) files[`./templates/${t.id}.json`] = t;
    try {
      reg = buildRegistry(files, { actions: new Set(LAND_ACTIONS), facts: new Set<string>(LAND_FACTS) });
    } catch (e) {
      if (extra === NO_EXTRA) throw e;
      console.warn("[behaviour] user templates invalid; using the built-in ones", e);
      reg = registryFor(NO_EXTRA);
    }
    registries.set(extra, reg);
  }
  return reg;
}
const resolvedCache = new WeakMap<Registry, Map<string, ResolvedTemplate>>();
/** A resolved template; an id the registry does not know falls back to core/default. */
function resolvedTemplate(id: string, reg: Registry): ResolvedTemplate {
  let byId = resolvedCache.get(reg);
  if (!byId) resolvedCache.set(reg, (byId = new Map()));
  let r = byId.get(id);
  if (!r) byId.set(id, (r = resolveTemplate(reg.has(id) ? id : DEFAULT_TEMPLATE_ID, reg)));
  return r;
}

export interface PlannedAction {
  action: string;
  /** The template's preference for this option; travels to Jev with what the action offers. */
  bias: Bias;
}

/**
 * Which actions run at this decision point, in order, under this template
 * (core/default if unknown), with each one's bias. `facts` answers the
 * template's conditions; a template naming a fact it is not given throws
 * rather than quietly offering nothing.
 */
export function landPlan(
  point: DecisionPoint,
  templateId: string = DEFAULT_TEMPLATE_ID,
  extra?: readonly Template[],
  facts: FactTable<null> = {},
): PlannedAction[] {
  return planOf(landWalk(point, templateId, extra, facts));
}

/** The whole walk at a decision point under this template: its path, and what it kept and dropped. */
export function landWalk(
  point: DecisionPoint,
  templateId: string = DEFAULT_TEMPLATE_ID,
  extra?: readonly Template[],
  facts: FactTable<null> = {},
): WalkResult {
  return walk(resolvedTemplate(templateId, registryFor(extra)), point, facts, null);
}

const planOf = (result: WalkResult): PlannedAction[] =>
  result.kind === "choose" ? result.options.map((o) => ({ action: o.action, bias: o.bias })) : [];

/** The ids of a template and its ancestors, itself first (what it inherits from). */
export function templateChain(templateId: string, extra?: readonly Template[]): string[] {
  return resolvedTemplate(templateId, registryFor(extra)).lineage;
}

/**
 * How a decision's options came about, for the test bench (filled in by
 * `landOptions` when given one; the game never asks).
 */
export interface LandTrace {
  templateId?: string;
  walk?: WalkResult;
  /** Each action the template kept, in order, and the options it actually produced here (none: it has nothing to offer). */
  steps: { action: string; bias: Bias; produced: RtOption[] }[];
}

/** The template a unit uses in this game: its assignment, else its type's default. */
export function templateIdFor(id: string, state: RtState, config: RtConfig): string {
  return config.behaviours?.[id] ?? defaultTemplateForClass(state.game.forceElements[id]?.targetClass);
}

/** The doctrine Jev is told for this unit, root template first. Empty for core/default. */
export function behaviourDoctrine(id: string, state: RtState, config: RtConfig): string[] {
  return resolvedTemplate(templateIdFor(id, state, config), registryFor(config.behaviourTemplates)).doctrine;
}

/**
 * The options at a decision point, in their fixed order. The first is always the
 * safe default. Moved verbatim from engine/decisions.ts (optionsAt), which now
 * delegates here.
 */
export function landOptions(
  state: RtState,
  id: string,
  point: DecisionPoint,
  event: RtEvent,
  config: RtConfig,
  context: DecisionContext = {},
  trace?: LandTrace,
): RtOption[] {
  const self = state.game.forceElements[id];
  const unit = state.units[id];
  if (!self || !unit || self.combatStrength <= 0) return [];
  const about = event.about ? state.game.forceElements[event.about] : undefined;
  const aboutAlive = about && about.combatStrength > 0 && about.side !== self.side ? about : undefined;
  const aboutKnown = aboutAlive && knownTo(state.game, state, id, aboutAlive.id) !== "none" ? aboutAlive : undefined;
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

  /** D3: the bearing of an unseen shooter, from the event or what the unit suspects. */
  const shooterBearing = () => event.bearingDeg ?? unit.suspects[event.about ?? ""]?.bearingDeg;
  /** D7: the enemy just lost, where it was last seen, and whether it broke. */
  const lostTarget = () => {
    const enemyId = event.about;
    return {
      enemyId,
      lkp: enemyId ? state.lastKnown[self.side]?.[enemyId] : undefined,
      broken: enemyId ? state.units[enemyId]?.cohesion === "broken" : false,
      target: enemyId ? state.game.forceElements[enemyId] : undefined,
    };
  };
  /** D0: the orders waiting to take over, their first order, and the fight it is in. */
  const newOrders = () => {
    const orders = context.orders;
    if (!orders || !orders.phases.length) return null;
    const first: RtOrder = { ...orders.phases[0].order, phase: 0 };
    const enemyId = currentEnemy(state, id);
    const enemy = enemyId ? state.game.forceElements[enemyId] : undefined;
    const hide = enemy ? hiddenSpot(self, enemy.position, config) : null;
    return { orders, first, enemy, hide, task: `"${orders.task}"` };
  };
  /** D12: the request being answered, its enemy if this unit knows it, and how to reply. */
  const theRequest = () => {
    const request = unit.requests.find((r) => r.id === event.requestId);
    if (!request) return null;
    const enemy = request.enemyId ? state.game.forceElements[request.enemyId] : undefined;
    const known = enemy && enemy.combatStrength > 0 && knownTo(state.game, state, id, enemy.id) !== "none" ? enemy : undefined;
    const reply = (answer: "comply" | "cannot" | "partly", text: string) => ({
      kind: "reply" as const,
      to: request.from,
      text,
      reply: { requestId: request.id, answer },
    });
    const what = request.kind === "cover" ? `cover ${request.from}` : `fire on ${request.enemyId ?? "its attacker"}`;
    return { request, known, reply, what };
  };
  /** D8: the target the unit has been firing on. */
  const engagedTarget = () => {
    const targetId = unit.engagement?.targetId;
    return { targetId, target: targetId ? state.game.forceElements[targetId] : undefined };
  };

  // Each case of what used to be a `switch (point)` is now a named action,
  // and core/default.json says which action runs at which decision point.
  const actions: Record<string, () => void> = {
    // ── D0 (new orders arrive mid-fight). Each needs orders with at least one
    // phase. "now" orders offer comply / covered / fireAndBack; others offer
    // comply / finish / breakContact — the template lists all five, and each
    // action runs only for its urgency, so either way the order is as before.
    "land.complyWithOrders": () => {
      const n = newOrders();
      if (!n) return;
      push(
        n.orders.urgency === "now"
          ? { id: "comply", summary: `break off at once and start ${n.task}`, order: n.first, orders: n.orders }
          : { id: "comply", summary: `comply now: start ${n.task}`, order: n.first, orders: n.orders },
      );
    },
    /** Urgent: ask the nearest steady friend that can hit the enemy to cover the break-off. */
    "land.breakOffCovered": () => {
      const n = newOrders();
      if (!n || n.orders.urgency !== "now" || !n.enemy) return;
      const enemy = n.enemy;
      const cover = Object.values(state.game.forceElements)
        .filter((f) => f.side === self.side && f.id !== id && f.combatStrength > 0 && state.units[f.id]?.cohesion === "steady")
        .filter((f) => distanceM(f.position, self.position) <= 1500 && canHit(f, enemy, enemy.position, config))
        .filter((f) => knownTo(state.game, state, f.id, enemy.id) !== "none")
        .sort((a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position))[0];
      if (cover) {
        // Asked over the radio: whether and how it covers is its own call (D12).
        push({
          id: "covered",
          summary: `break off under covering fire: ask ${cover.id} to cover it while it pulls out to start ${n.task}`,
          order: n.first,
          orders: n.orders,
          message: askFor(cover.id, "cover", enemy.id, `${id} asks ${cover.id} to cover it off against ${enemy.id}`),
        });
      }
    },
    "land.fireAndBack": () => {
      const n = newOrders();
      if (!n || n.orders.urgency !== "now") return;
      if (n.enemy && n.hide && canHit(self, n.enemy, n.enemy.position, config)) {
        push({
          id: "fireAndBack",
          summary: `fire one more volley at ${n.enemy.id}, move back out of its sight, then start ${n.task}`,
          order: { kind: "engage", targetId: n.enemy.id, volleys: 1, then: { kind: "withdraw", to: n.hide, then: n.first } },
          orders: n.orders,
        });
      }
    },
    "land.finishFightFirst": () => {
      const n = newOrders();
      if (!n || n.orders.urgency === "now") return;
      if (n.enemy && canHit(self, n.enemy, n.enemy.position, config)) {
        push({
          id: "finish",
          summary: `finish this fight with ${n.enemy.id} first (until it is out of the fight or lost, at most two minutes), then start ${n.task}`,
          order: { kind: "engage", targetId: n.enemy.id, until: state.time + 120, then: n.first },
          orders: n.orders,
        });
      }
    },
    "land.breakContactFirst": () => {
      const n = newOrders();
      if (!n || n.orders.urgency === "now") return;
      if (n.hide) {
        push({
          id: "breakContact",
          summary: `break contact (back out of sight, ${whereWords(self.position, n.hide)}), then start ${n.task}`,
          order: { kind: "withdraw", to: n.hide, then: n.first },
          orders: n.orders,
        });
      }
    },
    // ── D1 (new contact), split into one action per option family. The
    // order they run in — and so the order offered — is core/default.json's.
    "land.carryOn": () => {
      push(keep(`carry on (${describeActivity(state, id)})`));
    },
    "land.engageNow": () => {
      if (aboutKnown) push(engage(aboutKnown, "engage", "engage now:"));
    },
    /** One pair per trigger the unit could wait for: wait then decide, or wait then fire at once. */
    "land.waitFor": () => {
      if (!aboutKnown || !allowedTarget(state, id, aboutKnown.id)) return;
      for (const [index, trigger] of triggersFor(state, self, aboutKnown, config).entries()) {
        push(
          { id: `wait${index}`, summary: `hold still and wait until ${describeTrigger(trigger)}, then decide`, order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: false } },
          { id: `wait${index}:fire`, summary: `hold still and wait until ${describeTrigger(trigger)}, then fire at once`, order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: true } },
        );
      }
    },
    "land.betterPosition": () => {
      if (aboutKnown) push(better(aboutKnown));
    },
    "land.observe": () => {
      push({ id: "observe", summary: "observe and report, without firing", order: { kind: "observe" } });
    },
    "land.pullBackToFriends": () => {
      push(pullBack(aboutKnown?.position ?? null, "pullBack", "pull back to friends"));
    },
    // ── D2 (fired on, shooter located). Each needs the shooter known.
    "land.returnFire": () => {
      if (!aboutKnown) return;
      push(engage(aboutKnown, "returnFire", "return fire from here on"));
    },
    "land.hullDownThenFire": () => {
      if (!aboutKnown) return;
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
    },
    "land.outOfSight": () => {
      if (aboutKnown) push(outOfSight(aboutKnown));
    },
    /**
     * With a friend to ask: fire on it together, or pull back under its cover.
     * Builds on the returnFire / pullBack options already offered, so it must
     * run after them — and offers nothing for one a template removed.
     */
    "land.askFriendForHelp": () => {
      if (!aboutKnown) return;
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
    },
    "land.assault": () => {
      if (!aboutKnown) return;
      if (distanceM(self.position, aboutKnown.position) <= 1500 && allowedTarget(state, id, aboutKnown.id)) {
        push({
          id: "assault",
          summary: `assault ${nameOf(state, id, aboutKnown.id)}: close in firing, to point-blank`,
          order: { kind: "move", to: aboutKnown.position, mode: "assault" },
        });
      }
    },
    "land.goQuietInCover": () => {
      if (aboutKnown) push(quiet("hold still in cover and do not fire"));
    },
    // ── D3 (fired on, shooter not located). Each needs the shooter's bearing;
    // without one, only carrying on (land.carryOn) is offered.
    "land.searchBearing": () => {
      const bearing = shooterBearing();
      if (bearing == null) return;
      const words = compassWord(bearing);
      push({
        id: "search",
        summary: `search the ${words}: hold still and look for the shooter (spotting doubled that way for half a minute)`,
        order: { kind: "search", bearingDeg: bearing, until: state.time + 30 },
      });
    },
    "land.coverFromBearing": () => {
      const bearing = shooterBearing();
      if (bearing == null) return;
      const words = compassWord(bearing);
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
    },
    "land.pullBackFromBearing": () => {
      const bearing = shooterBearing();
      if (bearing == null) return;
      const words = compassWord(bearing);
      const back = offsetBy(self.position, (bearing + 180) % 360, 500);
      if (allowanceAt(self, back, config) > 0) {
        push({ id: "pullBack", summary: `pull back, away from the ${words}`, order: { kind: "withdraw", to: back } });
      }
    },
    // ── D4 (trigger met). Each needs the enemy known and the unit waiting on it.
    "land.fireNow": () => {
      if (!aboutKnown || unit.order.kind !== "wait") return;
      push(engage(aboutKnown, "fire", "fire now on"));
    },
    "land.waitCloser": () => {
      if (!aboutKnown || unit.order.kind !== "wait") return;
      const range = distanceM(self.position, aboutKnown.position);
      if (range > 500) {
        const trigger: Trigger = { kind: "range", withinM: Math.max(300, Math.round((range * 0.6) / 100) * 100) };
        push({
          id: "closer",
          summary: `wait for a closer shot: until ${describeTrigger(trigger)}`,
          order: { kind: "wait", targetId: aboutKnown.id, trigger, autoFire: false },
        });
      }
    },
    "land.letPass": () => {
      if (!aboutKnown || unit.order.kind !== "wait") return;
      push({ id: "letPass", summary: "let it pass and stay hidden", order: { kind: "observe" } });
    },
    // ── D5 (a watched enemy's behaviour changed). Each needs it still alive.
    "land.fireFirst": () => {
      if (!aboutAlive) return;
      push(aboutKnown ? engage(aboutKnown, "fireFirst", "fire first, while it still can, on") : null);
    },
    "land.keepHolding": () => {
      if (aboutAlive) push(keep("keep holding: it may be a coincidence"));
    },
    "land.relocateQuietly": () => {
      if (!aboutAlive) return;
      const spot = hiddenSpot(self, aboutAlive.position, config, 400) ?? nearestCover(self, config, 300, aboutAlive.position);
      if (spot) {
        push({
          id: "relocate",
          summary: `relocate quietly (${whereWords(self.position, spot)}) and keep watching`,
          order: move(spot, { kind: "observe" }),
        });
      }
    },
    "land.slipAway": () => {
      if (aboutAlive) push(outOfSight(aboutAlive));
    },
    // ── D6 (after a volley). Each needs the target known.
    /** Keep firing, with what it has achieved, how the target reacted, and how exposed the unit now is. */
    "land.keepFiring": () => {
      if (!aboutKnown) return;
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
      // The exact figures used to be patched onto out[0] after every D6
      // option was built; keep is always first here, so set them on it directly.
      push({
        ...keep(
          `keep firing on ${aboutKnown.id}: ${effectWords(odds)}; ${reaction}` +
            (firedBack ? "" : `; ${chanceBand(located)} that it has located you`),
        ),
        exact: `P(located) ≈ ${Math.round(located * 100)}%; ${describeEffect(odds)}`,
      });
    },
    "land.fireAndMove": () => {
      if (!aboutKnown) return;
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
    },
    "land.shiftFire": () => {
      if (aboutKnown) push(shift(aboutKnown.id));
    },
    "land.goQuiet": () => {
      if (aboutKnown) push(quiet());
    },
    // ── D7 (target lost: out of sight, or out of the fight).
    "land.resumeOrCarryOn": () => {
      push(resume ? { id: "resume", summary: resume.summary, order: resume.order } : keep(`carry on (${describeActivity(state, id)})`));
    },
    "land.watchLastSeen": () => {
      const { enemyId, lkp, broken } = lostTarget();
      if (lkp || broken) push({ id: "watch", summary: `watch where ${enemyId} was last seen`, order: { kind: "observe" } });
    },
    /** Follow up a broken enemy, or move halfway to where it was last seen. */
    "land.regainContact": () => {
      const { enemyId, lkp, broken, target } = lostTarget();
      if (broken && target && target.combatStrength > 0 && allowedTarget(state, id, target.id)) {
        push({ id: "regain", summary: `follow ${enemyId} up: it has broken (assault after it)`, order: { kind: "move", to: target.position, mode: "assault" } });
      } else if (lkp && distanceM(self.position, lkp.at) > 300) {
        const to = offsetBy(self.position, bearingDeg(self.position, lkp.at), distanceM(self.position, lkp.at) * 0.5);
        push({ id: "regain", summary: `move to regain sight of ${enemyId} (${whereWords(self.position, to)})`, order: move(to, { kind: "observe" }) });
      }
    },
    "land.shiftFireFromLost": () => {
      push(shift(lostTarget().enemyId ?? ""));
    },
    // ── D8 (fire not working: minutes of firing, nothing knocked out).
    "land.betterShotOnTarget": () => {
      const { target } = engagedTarget();
      if (target && target.combatStrength > 0) push(better(target, "better", true));
    },
    "land.shiftFireFromTarget": () => {
      push(shift(engagedTarget().targetId ?? ""));
    },
    /** As land.goQuiet, but not conditional on a known enemy (D6's is). */
    "land.stopFiring": () => {
      push(quiet());
    },
    "land.outOfTargetSight": () => {
      const { target } = engagedTarget();
      if (target) push(outOfSight(target));
    },
    "land.keepAtIt": () => {
      const { target } = engagedTarget();
      const odds = target ? damageEffect(self, target, state, config) : null;
      push(keep(`keep at it: ${effectWords(odds)}`));
    },
    // ── D9 (a friend close by needs help). Each needs its attacker known;
    // carrying on is land.carryOn.
    "land.helpFriend": () => {
      if (aboutKnown) push(engage(aboutKnown, "help", "fire on its attacker,"));
    },
    "land.moveToSupport": () => {
      if (!aboutKnown) return;
      const spot = betterShot(state, self, aboutKnown, config);
      if (spot && allowedTarget(state, id, aboutKnown.id)) {
        push({
          id: "support",
          summary: `move to support (${spot.why}, ${whereWords(self.position, spot.at)}), then engage ${aboutKnown.id}`,
          order: move(spot.at, { kind: "engage", targetId: aboutKnown.id }),
        });
      }
    },
    // ── D12 (a friend asked for help over the radio). Each needs the request;
    // every option answers it.
    /** Engage from here if it can, else move to get a shot. */
    "land.complyWithRequest": () => {
      const r = theRequest();
      if (!r) return;
      const { request, known, reply, what } = r;
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
    },
    "land.partlyOverwatch": () => {
      const r = theRequest();
      if (!r || unit.order.kind === "overwatch") return;
      push({
        id: "partly",
        summary: `partly: stay here on overwatch and fire at anything in reach, without moving`,
        order: { kind: "overwatch" },
        answers: r.request.id,
        message: r.reply("partly", `${id} to ${r.request.from}: can't move, covering from where it is`),
      });
    },
    "land.cannotComply": () => {
      const r = theRequest();
      if (!r) return;
      const busy = currentEnemy(state, id);
      push({
        ...keep(`can't: carry on (${describeActivity(state, id)})${busy ? `, it is fighting ${busy}` : ""}`),
        answers: r.request.id,
        message: r.reply("cannot", `${id} to ${r.request.from}: can't${busy ? `, engaged with ${busy}` : ""}`),
      });
    },
    // ── D10 (the current step of its orders is done, or blocked).
    "land.resumeOrders": () => {
      if (resume && event.kind !== "outOfOrders") push({ id: "resume", summary: resume.summary, order: resume.order });
    },
    /** Its current phase cannot be done: skip to the next one, if there is one. */
    "land.skipToNextPhase": () => {
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
    },
    "land.holdHere": () => {
      if (unit.order.kind !== "hold") push({ id: "hold", summary: "hold here", order: { kind: "hold" } });
    },
    "land.overwatchHere": () => {
      push({ id: "overwatch", summary: "overwatch from here: fire at anything in reach", order: { kind: "overwatch" } });
    },
    /** Already holding: carrying on is to keep holding. */
    "land.keepHoldingHere": () => {
      if (unit.order.kind === "hold") push(keep("hold here"));
    },
    // ── D11 (rallied: it takes orders again).
    "land.rejoin": () => {
      if (resume) push({ id: "resume", summary: `rejoin: ${resume.summary}`, order: resume.order });
    },
    "land.holdAndRecover": () => {
      push({ id: "hold", summary: "hold and recover", order: { kind: "hold" } });
    },
    "land.joinNearestFriends": () => {
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
    },
  };
  // The facts a template's conditions may test (behaviour/facts.ts).
  const facts = {
    "self.class": () => self.targetClass,
    "self.strength": () => self.combatStrength,
    "self.cohesion": () => unit.cohesion,
    "self.inCover": () => inCover(config.terrain, self.position),
    "enemy.known": () => aboutKnown != null,
    "enemy.rangeM": () => (aboutKnown ? Math.round(distanceM(self.position, aboutKnown.position)) : null),
    "orders.urgent": () => context.orders?.urgency === "now",
  } satisfies Record<LandFact, () => FactValue>;
  const templateId = templateIdFor(id, state, config);
  const walked = landWalk(point, templateId, config.behaviourTemplates, facts);
  if (trace) Object.assign(trace, { templateId, walk: walked });
  for (const { action, bias } of planOf(walked)) {
    const run = actions[action];
    if (!run) throw new Error(`Unknown land action "${action}"`);
    const before = out.length;
    run();
    if (bias) for (let i = before; i < out.length; i++) out[i] = { ...out[i], preference: bias };
    trace?.steps.push({ action, bias, produced: out.slice(before) });
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
