// ── bgws/rules/jevAssess.ts ────────────────────────────────────────────────
// Jev as a second pair of eyes for the planner.
//
// Before a language model writes the turn's orders, Jev is asked two cheap
// questions about every element — how much danger is it in, and how good are
// its chances — as `score` questions, all in one request. The answers go into
// the planner's prompt as advice.
//
// Why bother, when the planner can read the same board? Because the planner
// reads it in prose and in one pass, and a table of "B3: danger 4.6" is the
// kind of thing it reliably acts on and reliably misses when it has to work it
// out. It costs a fraction of a cent and one round trip; if Jev is unreachable
// the planner simply plans without it.

import type { GameState, Side } from "../lib/state";
import { keyed, withDeadline, type JevCall, type JevQuestion } from "./jev";
import { sideState } from "./jevState";
import type { ElementAssessment, OrdersCommander, OrdersRequest } from "./orders";
import type { PhaseConfig } from "./turnLoop";

const DANGER: Record<string, string> = {
  "1": "Safe: no known enemy can see or reach it.",
  "2": "Watched: known enemies could see it, but at long range or with poor odds.",
  "3": "Exposed: in the open to at least one enemy that can hurt it.",
  "4": "Threatened: several enemies can hit it with good odds.",
  "5": "In grave danger: likely to be hit hard or broken this turn where it stands.",
};

const OPPORTUNITY: Record<string, string> = {
  "1": "None: nothing it can usefully engage.",
  "2": "Poor: long-range or low-odds shots only.",
  "3": "Fair: a reasonable shot or a useful move to make.",
  "4": "Good: a good shot, or a chance to take ground or a flank.",
  "5": "Decisive: can do serious damage or take the objective this turn.",
};

export interface JevAssessOptions {
  side: Side;
  call: JevCall;
  /**
   * The live game, for terrain, objectives, threats and recent events. A
   * getter because the log and the board change every turn; fog of war is
   * still applied from the request's own view.
   */
  context?: () => { config: PhaseConfig; state: GameState } | undefined;
  timeoutMs?: number;
  log?: boolean;
}

/** Ask Jev for a danger and opportunity score for every element in view. */
export async function assessElements(
  request: OrdersRequest,
  options: JevAssessOptions,
): Promise<Record<string, ElementAssessment> | undefined> {
  const elements = keyed(request.view.own, "e");
  if (elements.length === 0) return undefined;

  const questions: Record<string, JevQuestion> = {};
  for (const { key, item } of elements) {
    questions[`${key}_danger`] = {
      type: "score",
      instructions: `How much danger is your element ${item.id} in where it stands, this turn?`,
      criteria: DANGER,
    };
    questions[`${key}_opportunity`] = {
      type: "score",
      instructions: `How good are your element ${item.id}'s chances of hurting the enemy or taking ground this turn?`,
      criteria: OPPORTUNITY,
    };
  }

  try {
    const context = options.context?.();
    const response = await withDeadline(
      options.call({
        state: sideState(request.view, context?.config, context?.state),
        questions,
      }),
      options.timeoutMs ?? 6000,
    );
    const out: Record<string, ElementAssessment> = {};
    for (const { key, item } of elements) {
      const danger = response.answers[`${key}_danger`];
      const opportunity = response.answers[`${key}_opportunity`];
      if (danger?.type === "score" && opportunity?.type === "score") {
        out[item.id] = { danger: danger.score, opportunity: opportunity.score };
      }
    }
    if (options.log !== false && typeof console !== "undefined") {
      console.info?.(`[Jev ${options.side}] assessed ${Object.keys(out).length} element(s) for the planner`, out);
    }
    return Object.keys(out).length > 0 ? out : undefined;
  } catch (err) {
    if (options.log !== false && typeof console !== "undefined") {
      console.warn?.(`[Jev ${options.side}] assessment skipped: ${err instanceof Error ? err.message : String(err)}`);
    }
    return undefined;
  }
}

/**
 * The same commander, planning with Jev's assessment in front of it.
 *
 * A wrapper rather than a change to the commander, so any OrdersCommander —
 * a model, the heuristic, a person — can be given the advice, and none of
 * them has to know where it came from.
 */
export function withJevAssessments(
  inner: OrdersCommander,
  options: JevAssessOptions,
): OrdersCommander {
  return {
    kind: inner.kind,
    name: `${inner.name}+jev-assess`,
    async planTurn(request) {
      const assessments = await assessElements(request, options);
      return inner.planTurn(assessments ? { ...request, assessments } : request);
    },
    ...(inner.planCounteraction
      ? { planCounteraction: (request) => inner.planCounteraction!(request) }
      : {}),
  };
}
