// ── bgws/realtime/engine/initialOrders.ts ──────────────────────────────────
// Orders before the clock starts.
//
// Two ways, both giving every unit an order, a rule of engagement and a
// purpose (which Jev reads later when it has to decide what the unit does):
//
//   heuristic   everyone advances on the objective, answering what closes in
//   Jev         one request: for every unit, an opening order and its rules
//               of engagement, chosen from options the rules generate

import { distanceM, type LatLng } from "../../lib/board";
import type { ForceElement, Side } from "../../lib/state";
import {
  JEV_MODEL,
  choiceOf,
  keyed,
  withDeadline,
  type JevCall,
  type JevQuestion,
} from "../../rules/jev";
import { printDecision } from "../../rules/jevConsole";
import { setOrder } from "./engine";
import { realtimeSideState } from "./jevDecider";
import { optionsFor } from "./options";
import type { Mission, Roe, RtConfig, RtOption, RtState } from "./types";

const ROE_CHOICES: Record<Roe, string> = {
  never: "Hold fire and stay hidden until ordered otherwise.",
  ifFiredUpon: "Stay quiet until the enemy engages your side, then answer.",
  withinShortRange: "Answer anything that closes inside short range.",
  always: "Fire at anything that can be reached.",
};

/**
 * What a unit may be told at the start: no "carry on" — there is nothing to
 * carry on with — and no "resume", since there is no mission yet.
 */
export function openingOptions(state: RtState, id: string, config: RtConfig): RtOption[] {
  return optionsFor(state, id, config).filter((option) => option.id !== "keep" && option.id !== "resume");
}

/**
 * What the unit is FOR, from its opening order. It outlasts the order: a unit
 * that has been knocked off it — into cover, back to rally — returns to it.
 */
export function missionOf(option: RtOption, fe: ForceElement, objective?: LatLng): Mission {
  const order = option.order;
  if (option.id.startsWith("objective") && objective) {
    return { task: "take", at: objective, purpose: "take the objective" };
  }
  if (order.kind === "move" && order.mode === "assault") {
    return { task: "take", at: order.to, purpose: `take the ground ${option.summary.replace(/^assault /, "held by ")}` };
  }
  if (option.id === "overwatch" || option.id === "pos:overwatch") {
    return { task: "support", at: order.kind === "move" ? order.to : fe.position, purpose: "cover the advance" };
  }
  if (order.kind === "move" || order.kind === "withdraw") {
    return { task: "hold", at: order.to, purpose: `hold the ground at the end of: ${option.summary}` };
  }
  return { task: "hold", at: fe.position, purpose: "hold this ground" };
}

/** Everyone advances on the objective; a unit already on it watches over it. */
export function heuristicInitialOrders(state: RtState, side: Side, config: RtConfig): RtState {
  let next = state;
  const objective = state.game.objectives?.[side];
  for (const fe of Object.values(state.game.forceElements)) {
    if (fe.side !== side || fe.combatStrength <= 0) continue;
    const far = objective && distanceM(fe.position, objective) > 300;
    next = setOrder(
      next,
      fe.id,
      far ? { kind: "move", to: objective, mode: "tactical" } : { kind: "overwatch" },
      config,
      {
        roe: "withinShortRange",
        mission: far
          ? { task: "take", at: objective, purpose: "take the objective" }
          : { task: "hold", at: objective ?? fe.position, purpose: "hold the objective" },
      },
    );
  }
  return { ...next, plan: { ...next.plan, [side]: "advance on the objective and take it" } };
}

/**
 * Jev chooses every unit's opening order and rules of engagement, in one request.
 *
 * Falls back to the heuristic's orders for any unit Jev does not answer
 * confidently, and entirely if Jev cannot be reached — the game must be able
 * to start either way.
 */
export async function jevInitialOrders(
  state: RtState,
  side: Side,
  config: RtConfig,
  call: JevCall,
  options: { directive?: string; timeoutMs?: number; log?: boolean } = {},
): Promise<RtState> {
  const fallback = heuristicInitialOrders(state, side, config);
  const units = keyed(
    Object.values(state.game.forceElements).filter((fe) => fe.side === side && fe.combatStrength > 0),
    "u",
  );
  if (units.length === 0) return fallback;

  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";
  const questions: Record<string, JevQuestion> = {};
  const offered = new Map<string, { key: string; option: RtOption }[]>();

  for (const { key, item } of units) {
    const choices = openingOptions(state, item.id, config).map((option, index) => ({
      key: `o${index}`,
      option,
    }));
    offered.set(key, choices);
    questions[`${key}_order`] = {
      type: "choice",
      instructions:
        `${directive}The engagement is about to start in real time: every unit will act ` +
        `at once. Choose the opening order for your unit ${item.id} (${item.label}). ` +
        "Think about the objective, the ground and keeping units able to support each other. " +
        "How it moves matters: a road march is fastest but does not fire on the move; " +
        "tactical movement is slower and answers fire; bounding overwatch is slowest " +
        "and safest, pairs covering each other. The order you choose also sets the " +
        "unit's mission, which it returns to after being knocked off it.",
      criteria: Object.fromEntries(choices.map((choice) => [choice.key, choice.option.summary])),
    };
    questions[`${key}_roe`] = {
      type: "choice",
      instructions:
        `${directive}Rules of engagement for ${item.id}: when does it open fire on its own ` +
        "initiative? Firing gives a position away.",
      criteria: { ...ROE_CHOICES },
    };
  }

  const stateSent = realtimeSideState(state, side, config, units.map(({ item }) => item.id));
  let answers;
  try {
    answers = (await withDeadline(call({ state: stateSent, questions }), options.timeoutMs ?? 8000)).answers;
  } catch (err) {
    if (options.log !== false && typeof console !== "undefined") {
      console.warn?.(`[Jev ${side}] initial orders fell back to the heuristic: ${err instanceof Error ? err.message : String(err)}`);
    }
    return fallback;
  }

  let next = state;
  let chosenCount = 0;
  for (const { key, item } of units) {
    const order = choiceOf(answers, `${key}_order`);
    const roe = choiceOf(answers, `${key}_roe`);
    const picked = offered.get(key)?.find((choice) => choice.key === order?.choice)?.option;
    if (!picked) {
      next = setOrder(next, item.id, fallback.units[item.id].order, config, {
        roe: fallback.units[item.id].roe,
        mission: fallback.units[item.id].mission,
      });
      continue;
    }
    chosenCount += 1;
    const rules = roe && roe.choice in ROE_CHOICES ? (roe.choice as Roe) : "withinShortRange";
    next = setOrder(next, item.id, picked.order, config, {
      roe: rules,
      mission: missionOf(picked, item, state.game.objectives?.[side]),
    });
    if (options.log !== false) {
      printDecision(side, {
        actorId: item.id,
        question: "opening order",
        options: (offered.get(key) ?? []).map((choice) => ({ id: choice.option.id, summary: choice.option.summary })),
        chosenId: picked.id,
        chosenBy: "jev",
        rationale: `rules of engagement: ${rules}`,
        probabilities: order
          ? Object.fromEntries((offered.get(key) ?? []).map((c) => [c.option.id, order.probabilities[c.key] ?? 0]))
          : undefined,
        confidence: order?.confidence,
      });
    }
  }

  return {
    ...next,
    plan: {
      ...next.plan,
      [side]: `${JEV_MODEL} opening orders: ${chosenCount} of ${units.length} units chosen by Jev`,
    },
  };
}
