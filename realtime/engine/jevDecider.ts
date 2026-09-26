// ── bgws/realtime/engine/jevDecider.ts ─────────────────────────────────────
// Jev in command, in real time.
//
// When units on one side need a decision at the same moment, they are asked
// together: ONE request, one `choice` question per unit, answered in parallel.
// Jev chooses from the options the rules generated, so it cannot give an
// order that is not possible.
//
// "Carry on" is always an option and is what an unsure Jev gets: a unit that
// is not told something new keeps doing what it was doing, which is the
// right default for a crew and stops units dithering between two orders.
//
// Every decision is printed to the browser console, like the turn game's.

import { distanceM } from "../../lib/board";
import { projectForSide } from "../../lib/fogOfWar";
import { inCover } from "../../lib/proceduralTerrain";
import { forceElementsOf, opposing, sightingOf, type Side } from "../../lib/state";
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
import { canHit, describeOrder } from "./engine";
import { compass } from "./geometry";
import { oddsAgainst } from "./options";
import { ruleChoice, ruleTrace, type RtDecider, type RtDecision } from "./deciders";
import { clock } from "./timing";
import type { RtConfig, RtState } from "./types";

export interface JevRealtimeOptions {
  side: Side;
  call: JevCall;
  /** Doctrine or temperament, put in front of every question. */
  directive?: string;
  /** How long to wait before the rules decide. Sim time waits for this, not the other way round. */
  timeoutMs?: number;
  /** Below this, Jev's answer is not taken on its own and the unit carries on. */
  minConfidence?: number;
  /** Print decisions to the console. On unless turned off. */
  log?: boolean;
}

/**
 * What this side knows, right now, for Jev.
 *
 * Fog of war applied: enemies only as sighted, unidentified ones unnamed,
 * and their odds against us only when identified (their weapons are what
 * nobody knows yet).
 */
export function realtimeSideState(
  state: RtState,
  side: Side,
  config: RtConfig,
  asked: readonly string[],
  recent: readonly { time: number; text: string }[] = [],
) {
  const view = projectForSide(state.game, side);
  const objective = state.game.objectives?.[side];

  const units = view.own.map((self) => {
    const unit = state.units[self.id];
    const threats = forceElementsOf(state.game, opposing(side))
      .filter((enemy) => enemy.combatStrength > 0 && sightingOf(state.game, side, enemy.id) === "full")
      .filter((enemy) => canHit(enemy, self, self.position, config))
      .map((enemy) => {
        const odds = oddsAgainst(enemy, self, state, config);
        return {
          from: enemy.id,
          rangeM: Math.round(distanceM(enemy.position, self.position)),
          ...(odds ? { pHitOnYou: odds.pHit } : {}),
        };
      });
    return {
      id: self.id,
      unit: self.label,
      strength: `${self.combatStrength}/${self.combatStrengthStart}`,
      troopQuality: self.troopQuality,
      morale: self.morale,
      doing: unit ? describeOrder(unit.order) : "unknown",
      rulesOfEngagement: unit?.roe,
      ...(unit?.purpose ? { purpose: unit.purpose } : {}),
      terrain: config.terrain.classify(self.position),
      inCover: inCover(config.terrain, self.position),
      ...(objective
        ? {
            objective: `${Math.round(distanceM(self.position, objective))} m ${compass(self.position, objective)}`,
          }
        : {}),
      ...(threats.length ? { threatsToYou: threats } : {}),
      beingAskedNow: asked.includes(self.id),
    };
  });

  const contacts = view.contacts.map((contact) => ({
    id: contact.id,
    identified: contact.sighting === "full",
    unit: contact.label ?? "unidentified",
    seen: contact.observedMarkers,
    nearestOfYoursM: view.own.length
      ? Math.round(Math.min(...view.own.map((fe) => distanceM(fe.position, contact.position))))
      : null,
  }));

  return {
    clock: clock(state.time),
    you: side,
    ...(state.plan[side] ? { commandersPlan: state.plan[side] } : {}),
    yourUnits: units,
    knownEnemies: contacts,
    recent: recent.slice(-8),
    note:
      "Real time: every unit acts at once. Enemies you have not sighted are not " +
      "listed; their absence is not evidence that they are not there.",
  };
}

function describe(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

export function jevRealtimeDecider(
  options: JevRealtimeOptions,
  /** The last few things that happened, as this side saw them. Supplied by the runner. */
  recent: () => { time: number; text: string }[] = () => [],
): RtDecider {
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
      const stateSent = realtimeSideState(state, side, config, requests.map((r) => r.unitId), recent());
      const keysFor = new Map<string, { key: string; id: string; summary: string }[]>();
      const questions: Record<string, JevQuestion> = {};

      for (const { key, item } of asked) {
        const choices = item.options.slice(0, JEV_MAX_CHOICES).map((option, index) => ({
          key: option.id === "keep" ? "keep" : `o${index}`,
          id: option.id,
          summary: option.summary,
        }));
        keysFor.set(key, choices);
        const label = state.game.forceElements[item.unitId]?.label ?? item.unitId;
        questions[key] = {
          type: "choice",
          instructions:
            `${directive}Your unit ${item.unitId} (${label}) is ${describeOrder(state.units[item.unitId].order)}. ` +
            `What just happened: ${item.events.map((event) => event.detail).join("; ")}. ` +
            "Choose its order from now on. Weigh its purpose and your commander's plan " +
            "against the threats to it, its odds, the cover around it and its strength " +
            "and morale. Carry on unless the situation calls for a change.",
          criteria: Object.fromEntries(choices.map((choice) => [choice.key, choice.summary])),
        };
      }

      let response: JevResponse;
      try {
        response = await withDeadline(options.call({ state: stateSent, questions }), timeoutMs);
      } catch (err) {
        return requests.map((request) => {
          const optionId = ruleChoice(state, request);
          const trace = ruleTrace(
            request,
            optionId,
            `Jev unavailable (${describe(err)}) — the rules decided`,
            err instanceof Error && err.name === "JevTimeoutError" ? "timeout" : "error",
          );
          print(trace, stateSent);
          return { unitId: request.unitId, optionId, trace };
        });
      }

      return asked.map(({ key, item }, index) => {
        const choices = keysFor.get(key) ?? [];
        const answer = choiceOf(response.answers, key);
        const picked = choices.find((choice) => choice.key === answer?.choice);
        const confident = answer != null && picked != null && answer.confidence >= minConfidence;
        const optionId = confident ? picked.id : "keep";
        const trace: TacticalTrace = {
          actorId: item.unitId,
          question: item.events.map((event) => event.kind).join(" + "),
          options: choices.map((choice) => ({ id: choice.id, summary: choice.summary })),
          chosenId: optionId,
          chosenBy: confident ? "jev" : "heuristic",
          rationale: confident
            ? item.events.map((event) => event.detail).join("; ")
            : `Jev ${answer ? "was unsure" : "gave no answer"} — carried on`,
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
