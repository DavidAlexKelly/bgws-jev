// ── bgws/realtime/engine/jevDecider.ts ─────────────────────────────────────
// Jev as each unit's leader, at the decision points.
//
// docs/REALTIME_COMMAND_DESIGN.html. Jev makes one call at a well-defined
// moment (decisions.ts), always inside the unit's orders; game logic does
// everything else. When units on one side reach a decision point at the same
// moment they are asked together: ONE request, one `choice` question each,
// answered in parallel.
//
// WHAT JEV IS SHOWN, and only this:
//   the order's task, intent, urgency and constraints;
//   the unit's beliefs about each enemy and about itself — never the truth;
//   the odds, in words ("likely to knock one out within a minute");
//   its friends within reach;
//   its last few decisions and what came of them.
// Only the units being asked are described: irrelevant state makes a single
// pass judge worse, not better. Exact figures go to the console, not to Jev.
//
// When Jev is unsure, or cannot be reached, the rules decide — following the
// order's actions on contact, not "carry on".
//
// Every decision is printed to the browser console, like the turn game's.

import { bearingDeg, distanceM } from "../../lib/board";
import { inCover } from "../../lib/proceduralTerrain";
import type { Side } from "../../lib/state";
import {
  JEV_MAX_CHOICES,
  JEV_MODEL,
  choiceOf,
  keyed,
  withDeadline,
  type JevCall,
  type JevQuestion,
  type JevResponse,
} from "../../rules/jev";
import { printDecision } from "../../rules/jevConsole";
import type { TacticalTrace } from "../../rules/tactical";
import { BALANCE, DECISION_POINTS, effectWords } from "./decisions";
import { ruleTrace, type RtDecider, type RtDecision, type RtDecisionRequest } from "./deciders";
import { activityOf, canHit, describeTrigger } from "./engine";
import { agoBand, beliefsOf, compassWord, rangeBand, selfBeliefOf } from "./knowledge";
import { damageEffect } from "./options";
import { PINNED_AT, SUPPRESSED_AT } from "./timing";
import type { RtConfig, RtState, UnitOrders } from "./types";

export interface JevRealtimeOptions {
  side: Side;
  call: JevCall;
  /** Doctrine or temperament, put in front of every question. */
  directive?: string;
  /** How long to wait before the rules decide. Sim time waits for this, not the other way round. */
  timeoutMs?: number;
  /** Below this, Jev's answer is not taken and the rules decide. */
  minConfidence?: number;
  /** Print decisions to the console. On unless turned off. */
  log?: boolean;
}

/** Friends within this are "nearby". */
const FRIENDS_M = 1500;

const ROE_WORDS: Record<string, string> = {
  never: "weapons hold: do not fire at all",
  ifFiredUpon: "fire only once the enemy has fired on your side",
  withinShortRange: "fire at anything that closes to short range",
  always: "fire at anything in reach",
};
const ON_CONTACT_WORDS: Record<string, string> = {
  engage: "engage",
  observe: "observe and report",
  avoid: "avoid a fight",
  bypass: "carry on with the task",
};

/** A unit's standing orders, in words. */
export function ordersWords(orders: UnitOrders) {
  return {
    task: orders.task,
    nowOn: orders.phases[orders.phase]?.label ?? "done",
    intent: orders.intent,
    urgency: orders.urgency === "now" ? "now" : "when able",
    rulesOfEngagement: ROE_WORDS[orders.roe] ?? orders.roe,
    onContact: ON_CONTACT_WORDS[orders.onContact] ?? orders.onContact,
    ...(orders.boundaries.length ? { constraints: orders.boundaries.map((b) => `stay ${b.keep} of ${b.label}`) } : {}),
    ...(orders.supports ? { supports: `${orders.supports}: its requests for help come to you first` } : {}),
  };
}

/** What one unit knows and is, in words, for its question. */
export function unitPicture(state: RtState, id: string, config: RtConfig) {
  const self = state.game.forceElements[id];
  const unit = state.units[id];
  if (!self || !unit) return { id };
  const self_ = selfBeliefOf(unit, state.time);
  const enemies = beliefsOf(state, id).map((belief) => {
    const enemy = state.game.forceElements[belief.id];
    if (!enemy || belief.belief !== "identified") return belief;
    const their = state.units[enemy.id];
    return {
      ...belief,
      ...(their ? { vehicles: `${their.vehicles.fit} of ${their.vehicles.total} still fighting` } : {}),
      ...(their && their.cohesion !== "steady" ? { visibly: their.cohesion === "broken" ? "breaking, falling back" : "shaken" } : {}),
      itsFireOnYou: canHit(enemy, self, self.position, config) ? effectWords(damageEffect(enemy, self, state, config), "you") : "cannot reach you",
      yourFireOnIt: canHit(self, enemy, enemy.position, config) ? effectWords(damageEffect(self, enemy, state, config)) : "out of your reach",
    };
  });
  const friends = Object.values(state.game.forceElements)
    .filter((other) => other.side === self.side && other.id !== id && other.combatStrength > 0)
    .filter((other) => distanceM(other.position, self.position) <= FRIENDS_M)
    .map((other) => ({
      id: other.id,
      doing: state.units[other.id] ? activityOf(state.units[other.id]) : "unknown",
      where: `${rangeBand(distanceM(self.position, other.position))} to the ${compassWord(bearingDeg(self.position, other.position))}`,
    }));
  const decisions = unit.history.map((memory) => {
    const lost = memory.strength - self.combatStrength;
    const did = unit.dealt - memory.dealt;
    return {
      when: agoBand(state.time - memory.time),
      chose: memory.chose,
      by: memory.by,
      since: `${lost > 0 ? "took losses" : "no losses"}; ${did > 0 ? "did damage" : "did no damage"}`,
    };
  });
  return {
    id,
    unit: self.label,
    doing: activityOf(unit),
    ...(unit.orders ? { orders: ordersWords(unit.orders) } : { purpose: unit.mission.purpose }),
    vehicles: `${unit.vehicles.fit} of ${unit.vehicles.total} fighting`,
    nerve:
      unit.suppression >= PINNED_AT
        ? "pinned down: cannot advance"
        : unit.suppression >= SUPPRESSED_AT
          ? "suppressed: shooting worse"
          : "steady",
    ground: `${config.terrain.classify(self.position)}${inCover(config.terrain, self.position) ? ", in cover" : ""}${
      unit.posture === "hullDown" ? ", hull-down" : unit.posture === "moving" ? ", on the move" : ""
    }`,
    doesTheEnemyKnowYouAreHere:
      self_ === "knownSeen"
        ? "yes: you have been fired on"
        : self_ === "possiblySeen"
          ? `perhaps: ${unit.lastCue?.enemyId} ${unit.lastCue?.cue}`
          : "no sign of it",
    enemies,
    ...(unit.heard.some((h) => state.time - h.time <= 180)
      ? {
          heardOnTheRadio: unit.heard
            .filter((h) => state.time - h.time <= 180)
            .map((h) => ({ when: agoBand(state.time - h.time), from: h.from, said: h.text })),
        }
      : {}),
    ...(friends.length ? { friendsNearby: friends } : {}),
    ...(decisions.length ? { lastDecisions: decisions } : {}),
  };
}

/** What happened, in words: the event that raised the decision point. */
export function situationWords(state: RtState, request: RtDecisionRequest): string {
  const text = situationText(state, request);
  return text.charAt(0).toUpperCase() + text.slice(1);
}

function situationText(state: RtState, request: RtDecisionRequest): string {
  const { event, point } = request;
  const unit = state.units[request.unitId];
  const self = state.game.forceElements[request.unitId];
  const enemy = event.about ? state.game.forceElements[event.about] : undefined;
  const identified = enemy && unit?.ownSeen[enemy.id]?.level === "full";
  const name = enemy ? (identified ? `${enemy.label} (${enemy.id})` : `the contact ${enemy.id}`) : "an enemy";
  const where =
    enemy && self
      ? ` at ${rangeBand(distanceM(self.position, enemy.position))} to the ${compassWord(bearingDeg(self.position, enemy.position))}`
      : "";
  const hurt = request.events.some((e) => e.kind === "hit") ? ", and has knocked out one of its vehicles" : "";
  switch (point) {
    case "D0":
      return `New orders have come down while it is fighting. ${event.detail}.`;
    case "D1":
      return `It has ${event.kind === "contact" ? "run into" : "found"} ${name}${where}.`;
    case "D2":
      return `${name}${where} is firing on it${hurt}.`;
    case "D3":
      return event.kind === "searchDone"
        ? `Its search of the ${compassWord(event.bearingDeg ?? 0)} found nothing.`
        : `It is under fire from a shooter it cannot see, somewhere to the ${compassWord(event.bearingDeg ?? 0)}${hurt}.`;
    case "D4":
      return unit?.order.kind === "wait"
        ? `The moment it was waiting for has come: ${describeTrigger(unit.order.trigger)} (${name}${where}).`
        : `Its trigger on ${name} is met.`;
    case "D5":
      return `${name}${where}, which it is watching from hiding, ${unit?.lastCue?.cue ?? "changed what it was doing"}. It cannot tell whether that is because it has been seen.`;
    case "D6":
      return event.severe ? `It has knocked out one of ${name}'s vehicles.` : `It has fired its first volley at ${name}${where}.`;
    case "D7":
      return event.kind === "enemyBroke" ? `${name} has broken and is falling back.` : `It has lost ${name}: out of sight or out of the fight.`;
    case "D8":
      return `It has been firing on ${unit?.engagement?.targetId ?? "its target"} for minutes and has knocked nothing out.`;
    case "D9":
      return `A friend close by needs help: ${event.detail}, ${where.trim()}.`;
    case "D10":
      return event.kind === "arrived"
        ? "It has reached where it was going."
        : event.kind === "outOfOrders"
          ? "It has carried out all its orders."
          : event.kind === "idle"
            ? "It has been quiet for a while and is not doing its task."
            : `It cannot go on: ${event.detail}.`;
    case "D11":
      return "It has rallied and takes orders again.";
    case "D12": {
      const request = unit?.requests.find((r) => r.id === event.requestId);
      const supporting = request != null && unit?.orders?.supports === request.from;
      return request
        ? `${request.from} asks over the radio: "${request.text}".${supporting ? ` Supporting ${request.from} is part of its orders.` : ""}`
        : `A friend asks for help: ${event.detail}.`;
    }
  }
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function jevRealtimeDecider(options: JevRealtimeOptions): RtDecider {
  const minConfidence = options.minConfidence ?? 0.25;
  const timeoutMs = options.timeoutMs ?? 4000;
  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";
  const print = (trace: TacticalTrace, stateSent: unknown) => {
    if (options.log !== false) printDecision(options.side, trace, stateSent);
  };

  return {
    name: `${JEV_MODEL}-${options.side}-realtime`,

    async decide(state, side, requests, config): Promise<RtDecision[]> {
      const asked = keyed(requests, "u");
      const stateSent = {
        you: side,
        ...(state.plan[side] ? { commandersPlan: state.plan[side] } : {}),
        units: requests.map((request) => unitPicture(state, request.unitId, config)),
        note:
          "Only what each unit believes. Enemies it has not found are not listed, and their absence is not " +
          "evidence that they are not there. A lost contact is where an enemy was last seen, not where it is.",
      };
      const keysFor = new Map<string, { key: string; id: string; summary: string; exact?: string }[]>();
      const questions: Record<string, JevQuestion> = {};

      for (const { key, item } of asked) {
        const choices = item.options.slice(0, JEV_MAX_CHOICES).map((option, index) => ({
          key: option.id === "keep" ? "keep" : `o${index}`,
          id: option.id,
          summary: option.summary,
          exact: option.exact,
        }));
        keysFor.set(key, choices);
        const dp = DECISION_POINTS[item.point];
        const unit = state.units[item.unitId];
        const label = state.game.forceElements[item.unitId]?.label ?? item.unitId;
        const others = requests.filter((r) => r.unitId !== item.unitId).map((r) => r.unitId);
        questions[key] = {
          type: "choice",
          instructions:
            `${directive}You lead ${item.unitId} (${label}). ${BALANCE}\n\n` +
            `Decision point ${item.point}, ${dp.title.toLowerCase()}. ${situationWords(state, item)} ${dp.question} ` +
            (unit?.orders
              ? `Its orders: ${unit.orders.task}. Intent: ${unit.orders.intent}. Urgency: ${unit.orders.urgency === "now" ? "now" : "when able"}. `
              : `Its purpose: ${unit?.mission.purpose ?? "hold its ground"}. `) +
            (others.length ? `Also deciding now, so choose to work together: ${others.join(", ")}. ` : "") +
            "The crew has already done its drill (cover, and return fire if it can see the shooter).",
          criteria: Object.fromEntries(choices.map((choice) => [choice.key, choice.summary])),
        };
      }

      const rulesTrace = (request: RtDecisionRequest, why: string, fallback: TacticalTrace["fallback"]) => {
        const trace = ruleTrace(request, request.fallback, why, fallback);
        return { ...trace, rationale: `${why}. ${exactOf(request)}` };
      };

      let response: JevResponse;
      try {
        response = await withDeadline(options.call({ state: stateSent, questions }), timeoutMs);
      } catch (err) {
        // Out in the open, not folded away: this is the one thing to see when Jev "does nothing".
        if (options.log !== false && typeof console !== "undefined") {
          console.warn?.(`[Jev ${side}] call failed, the rules decided ${requests.length} unit(s): ${describe(err)}`);
        }
        return requests.map((request) => {
          const trace = rulesTrace(
            request,
            `Jev unavailable (${describe(err)}) — the rules decided`,
            err instanceof Error && err.name === "JevTimeoutError" ? "timeout" : "error",
          );
          print(trace, stateSent);
          return { unitId: request.unitId, optionId: request.fallback, trace };
        });
      }

      return asked.map(({ key, item }, index) => {
        const choices = keysFor.get(key) ?? [];
        const answer = choiceOf(response.answers, key);
        const picked = choices.find((choice) => choice.key === answer?.choice);
        const confident = answer != null && picked != null && answer.confidence >= minConfidence;
        const optionId = confident ? picked.id : item.fallback;
        const trace: TacticalTrace = {
          actorId: item.unitId,
          question: `${item.point} ${DECISION_POINTS[item.point].title}: ${item.events.map((event) => event.kind).join(" + ")}`,
          options: choices.map((choice) => ({ id: choice.id, summary: choice.summary })),
          chosenId: optionId,
          chosenBy: confident ? "jev" : "heuristic",
          rationale:
            (confident
              ? item.events.map((event) => event.detail).join("; ")
              : `Jev ${answer ? `was unsure (${picked?.id ?? answer.choice} at ${Math.round((answer.confidence ?? 0) * 100)}% confidence)` : "gave no answer for this unit in its reply"} — the rules chose`) +
            `. ${exactOf(item)}`,
          probabilities: answer
            ? Object.fromEntries(choices.map((choice) => [choice.id, answer.probabilities[choice.key] ?? 0]))
            : undefined,
          confidence: answer?.confidence,
          latencyMs: response.latencyMs,
          costUsd: index === 0 ? response.costUsd : undefined,
          fallback: confident ? undefined : answer ? "lowConfidence" : "error",
          mark: choices.find((choice) => choice.id === optionId)?.summary,
        };
        print(trace, stateSent);
        return { unitId: item.unitId, optionId, trace };
      });
    },
  };
}

/** The exact figures behind the words, for the console. */
function exactOf(request: RtDecisionRequest): string {
  const figures = request.options.filter((option) => option.exact).map((option) => `${option.id}: ${option.exact}`);
  return figures.length ? `Exact: ${figures.join(" | ")}` : "";
}
