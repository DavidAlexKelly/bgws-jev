// ── bgws/realtime/engine/deciders.ts ───────────────────────────────────────
// Who chooses a unit's next order when something happens to it.
//
// The runner collects events, decides WHEN a unit is asked (coalescing,
// cooldown, severity) and applies the answer after the unit's reaction time.
// A decider only answers: for each unit asked, which of its options.
//
// `ruleDecider` is the default and the fallback: simple, legible, instant,
// and deterministic. Jev (jevDecider.ts) is the one that matters.

import { distanceM } from "../../lib/board";
import type { Side } from "../../lib/state";
import type { TacticalTrace } from "../../rules/tactical";
import type { RtConfig, RtEvent, RtOption, RtState } from "./types";

/** One unit's question. */
export interface RtDecisionRequest {
  unitId: string;
  events: RtEvent[];
  options: RtOption[];
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

/** The option the rule would take. */
export function ruleChoice(state: RtState, request: RtDecisionRequest): string {
  const kinds = new Set(request.events.map((event) => event.kind));
  const has = (id: string) => request.options.some((option) => option.id === id);
  const self = state.game.forceElements[request.unitId];

  // The best shot on offer, by the odds in its summary; otherwise the nearest.
  const engage = request.options
    .filter((option) => option.id.startsWith("engage:"))
    .map((option) => ({
      option,
      p: Number(/(\d+)% to hit/.exec(option.summary)?.[1] ?? 0),
      range:
        option.order.kind === "engage" && self
          ? distanceM(self.position, state.game.forceElements[option.order.targetId]?.position ?? self.position)
          : Infinity,
    }))
    .sort((a, b) => b.p - a.p || a.range - b.range)[0]?.option.id;

  const unit = state.units[request.unitId];
  const attackers = new Set(
    Object.entries(unit?.attackers ?? {})
      .filter(([, at]) => state.time - at <= 60)
      .map(([id]) => id),
  );
  // Shooting back at whoever is shooting at it comes first.
  const answer =
    request.options.find(
      (option) => option.order.kind === "engage" && attackers.has(option.order.targetId),
    )?.id ?? engage;

  // Back from being shaken or broken, or quiet and off its mission: get on
  // with what it is for. This is what stops a unit holding for ever.
  if (kinds.has("rallied") || kinds.has("idle")) {
    if (has("resume")) return "resume";
    return engage ?? "keep";
  }
  // The enemy has broken: an attacker follows up, anyone else stays put.
  if (kinds.has("enemyBroke")) {
    const pursue = request.options.find((option) => option.id.startsWith("pursue:"))?.id;
    if (unit?.mission.task === "take" && pursue) return pursue;
    if (has("consolidate")) return "consolidate";
    return "keep";
  }
  // Under fire in the open: get into cover if there is any, else shoot back.
  if (kinds.has("hit") || kinds.has("underFire") || kinds.has("moraleDrop")) {
    if (has("pos:cover")) return "pos:cover";
    if (answer) return answer;
    return "keep";
  }
  // Run into the enemy, sighted one, or walked into its reach: stop and
  // fight if there is a shot, unless this unit was told to stay silent.
  // "Carry on" here is how two columns used to drive straight past each
  // other.
  if (kinds.has("contact") || kinds.has("sighted") || kinds.has("exposed")) {
    const silent = unit?.roe === "never";
    if (answer && !silent) return answer;
    return "keep";
  }
  // Nothing left to do where it is: fight if it can, else get on with the mission.
  if (kinds.has("arrived") || kinds.has("targetGone") || kinds.has("blocked") || kinds.has("friendLost")) {
    if (answer) return answer;
    if (has("resume") && !kinds.has("blocked")) return "resume";
    return has("overwatch") ? "overwatch" : "keep";
  }
  return "keep";
}

export function ruleTrace(
  request: RtDecisionRequest,
  optionId: string,
  why: string,
  fallback?: TacticalTrace["fallback"],
): TacticalTrace {
  return {
    actorId: request.unitId,
    question: request.events.map((event) => event.kind).join(" + "),
    options: request.options.map((option) => ({ id: option.id, summary: option.summary })),
    chosenId: optionId,
    chosenBy: "heuristic",
    rationale: why,
    fallback,
    mark: request.options.find((option) => option.id === optionId)?.summary,
  };
}

/** The default: simple rules, answered at once. */
export const ruleDecider: RtDecider = {
  name: "rules",
  async decide(state, _side, requests) {
    return requests.map((request) => {
      const optionId = ruleChoice(state, request);
      return { unitId: request.unitId, optionId, trace: ruleTrace(request, optionId, "rules") };
    });
  },
};
