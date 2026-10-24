// ── realtime/world/actions.ts ────────────────────────────────────────────
// What each world verb DOES (the names are in behaviour/actions/world.ts).
//
// Each domain module registers its handlers when it loads (world/index.ts
// loads them all). A handler can:
//   available  — capability masking at decision time (DECISION_FLOW_GRAPHS.md
//                §3): a team with no fibre drones is never offered "switch to
//                fibre"; out of ammunition, "fire now" disappears.
//   describe   — the option in words, for Jev and the feed.
//   apply      — carry the choice out, through a world context.
//
// A domain also registers a RULE per trigger: what the rules choose when
// there is no Jev, Jev is unsure, or cannot be reached — the world's
// counterpart to decisions.ts's ruleFallback.

import type { WorldAction } from "../behaviour/actions/world";
import type { WorldCtx } from "./context";
import type { WorldOption, WorldQuestion } from "./types";

/** A question being built: everything but its options and answer. */
export type QuestionDraft = Pick<WorldQuestion, "agentId" | "side" | "trigger" | "about">;

export interface ActionHandler {
  available?(c: WorldCtx, q: QuestionDraft, params: WorldOption["params"]): boolean;
  describe?(c: WorldCtx, q: QuestionDraft, params: WorldOption["params"]): string;
  apply(c: WorldCtx, q: WorldQuestion, option: WorldOption): void;
}

/** What the rules take for a trigger, given the options on offer. Undefined: the template's fallback. */
export type TriggerRule = (c: WorldCtx, q: QuestionDraft, options: readonly WorldOption[]) => string | undefined;

const HANDLERS = new Map<string, ActionHandler>();
const RULES = new Map<string, TriggerRule>();

export function registerHandlers(handlers: Partial<Record<WorldAction, ActionHandler>>): void {
  for (const [name, handler] of Object.entries(handlers)) if (handler) HANDLERS.set(name, handler);
}

export function registerRules(rules: Record<string, TriggerRule>): void {
  for (const [trigger, rule] of Object.entries(rules)) RULES.set(trigger, rule);
}

export function handlerFor(action: string): ActionHandler | undefined {
  return HANDLERS.get(action);
}

export function ruleFor(trigger: string): TriggerRule | undefined {
  return RULES.get(trigger);
}

/** Every action with a handler, for the vocabulary test. */
export function handledActions(): string[] {
  return [...HANDLERS.keys()].sort();
}

/** The first option whose action is one of these, in the order given. */
export function prefer(options: readonly WorldOption[], ...actions: string[]): string | undefined {
  for (const action of actions) {
    const hit = options.find((o) => o.action === action);
    if (hit) return hit.id;
  }
  return undefined;
}
