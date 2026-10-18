// ── bgws/realtime/engine/deciders.ts ───────────────────────────────────────
// Who makes the call at a decision point.
//
// The runner collects events, decides WHEN a unit is asked (coalescing,
// cooldown, severity, one decision in flight per unit), works out WHICH
// decision point it is and its options (decisions.ts), and applies the
// answer after the unit's reaction time. A decider only answers: for each
// unit asked, which of its options.
//
// `ruleDecider` is the default and the fallback: it takes the rules' choice,
// which follows the unit's actions on contact. Jev (jevDecider.ts) is the one
// that matters.

import type { Side } from "../../lib/state";
import type { TacticalTrace } from "../../rules/tactical";
import type { DecisionPoint } from "./decisions";
import type { RtConfig, RtEvent, RtOption, RtState } from "./types";

/** One unit's question. */
export interface RtDecisionRequest {
  unitId: string;
  /** Which decision point, and the event that raised it. */
  point: DecisionPoint;
  event: RtEvent;
  /** Everything that happened to it since it was last asked. */
  events: RtEvent[];
  options: RtOption[];
  /** The rules' choice here, following its actions on contact. */
  fallback: string;
}

/** One unit's answer: an option id, and how it was reached. */
export interface RtDecision {
  unitId: string;
  optionId: string;
  trace: TacticalTrace;
}

export interface RtDecider {
  readonly name: string;
  /** Answer every request. Must resolve, never reject: fall back instead. */
  decide(
    state: RtState,
    side: Side,
    requests: RtDecisionRequest[],
    config: RtConfig,
  ): Promise<RtDecision[]>;
}

export function ruleTrace(
  request: RtDecisionRequest,
  optionId: string,
  why: string,
  fallback?: TacticalTrace["fallback"],
): TacticalTrace {
  return {
    actorId: request.unitId,
    question: `${request.point}: ${request.events.map((event) => event.kind).join(" + ")}`,
    options: request.options.map((option) => ({ id: option.id, summary: option.summary })),
    chosenId: optionId,
    chosenBy: "heuristic",
    rationale: why,
    fallback,
    mark: request.options.find((option) => option.id === optionId)?.summary,
  };
}

/** The default: the rules' choice, answered at once. */
export const ruleDecider: RtDecider = {
  name: "rules",
  async decide(_state, _side, requests) {
    return requests.map((request) => ({
      unitId: request.unitId,
      optionId: request.fallback,
      trace: ruleTrace(request, request.fallback, "rules"),
    }));
  },
};
