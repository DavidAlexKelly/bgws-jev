// ── realtime/world/questions.ts ──────────────────────────────────────────
// World decision points: when a world system needs a choice made — a mortar
// receives a fire request, a drone's link degrades, a destroyer sees missiles
// inbound — it raises a question here.
//
// The question is built exactly as a land decision is (DECISION_FLOW_GRAPHS.md):
// the agent's template is walked from the trigger, with the world facts;
// options are masked by what the agent can actually do now; the rules'
// choice is worked out. Then:
//   - a drill (`do` node) or a single option is taken without asking (§3.3);
//   - otherwise the runner hands it to the side's world decider — Jev, or the
//     rules — and applies the answer at `dueAt`: the agent's reaction time
//     after it was raised, in SIMULATED seconds, waiting if the answer is late.
// So a world decision replays as exactly as a land one.
//
// Which template: the agent's assignment if it covers this trigger, else its
// archetype's core template (catalogue `template`), else core/world.

import { WORLD_ACTION_INFO } from "../behaviour/actions/world";
import { behaviourRegistry, resolvedTemplate } from "../behaviour/actions/land";
import type { ResolvedTemplate } from "../behaviour/types";
import { walk } from "../behaviour/walk";
import { hasRecord, recordFor } from "../catalogue";
import type { RtConfig, RtEvent, RtState } from "../engine/types";
import { handlerFor, ruleFor, type QuestionDraft } from "./actions";
import type { WorldCtx } from "./context";
import { WORLD_FACT_TABLE } from "./facts";
import { withWorldCtx } from "./standalone";
import type { WorldLogItem, WorldOption, WorldQuestion } from "./types";

export const WORLD_DEFAULT_TEMPLATE = "core/world";

export interface RaiseSpec {
  agentId: string;
  side: "blue" | "red";
  trigger: string;
  about?: WorldQuestion["about"];
  /** Troop quality, for the reaction time; default regular (4). */
  quality?: number;
}

function archetypeOf(c: Pick<WorldCtx, "world">, agentId: string): string | undefined {
  return c.world.entities[agentId]?.archetype ?? c.world.land[agentId]?.archetype;
}

/** The template that answers this trigger for this agent, resolved. */
export function templateForTrigger(c: Pick<WorldCtx, "world" | "config">, agentId: string, trigger: string): ResolvedTemplate | null {
  const reg = behaviourRegistry(c.config.behaviourTemplates);
  const archetype = archetypeOf(c, agentId);
  const candidates = [
    c.config.behaviours?.[agentId],
    c.world.entities[agentId]?.template,
    archetype && hasRecord(archetype) ? recordFor(archetype).template : undefined,
    WORLD_DEFAULT_TEMPLATE,
  ];
  for (const id of candidates) {
    if (!id || !reg.has(id)) continue;
    const t = resolvedTemplate(id, reg);
    if (t.triggers[trigger]) return t;
  }
  return null;
}

/**
 * Raise a question for an agent. Returns it, or null when the agent's
 * template has nothing for this trigger, nothing it could do is left, or it
 * already has a question on this trigger waiting.
 */
export function raiseQuestion(c: WorldCtx, spec: RaiseSpec): WorldQuestion | null {
  if (c.world.questions.some((q) => q.agentId === spec.agentId && q.trigger === spec.trigger && !q.answer)) return null;
  const template = templateForTrigger(c, spec.agentId, spec.trigger);
  if (!template) return null;
  const draft: QuestionDraft = { agentId: spec.agentId, side: spec.side, trigger: spec.trigger, about: spec.about ?? {} };
  const result = walk(template, spec.trigger, WORLD_FACT_TABLE, {
    state: { game: c.game, units: c.land.units, time: c.time },
    world: c.world,
    config: c.config,
    agentId: spec.agentId,
    about: draft.about,
  });
  if (result.kind === "none") return null;
  const word = (action: string, params: WorldOption["params"]) =>
    handlerFor(action)?.describe?.(c, draft, params) ?? (WORLD_ACTION_INFO as Record<string, { label: string }>)[action]?.label ?? action;
  const usable = (action: string, params: WorldOption["params"]) => {
    const h = handlerFor(action);
    return h != null && (h.available?.(c, draft, params) ?? true);
  };
  let options: WorldOption[];
  let title: string;
  let question: string;
  let fallback: string | null;
  if (result.kind === "do") {
    if (!usable(result.action, result.params)) return null;
    options = [{ id: result.nodeId, action: result.action, summary: word(result.action, result.params), bias: 0, params: result.params }];
    title = `${spec.trigger} (drill)`;
    question = "";
    fallback = result.nodeId;
  } else {
    options = result.options
      .filter((o) => usable(o.action, o.params))
      .map((o) => ({ id: o.id, action: o.action, summary: word(o.action, o.params), bias: o.bias, ...(o.params ? { params: o.params } : {}) }));
    title = result.title;
    question = result.question;
    fallback = options.some((o) => o.id === result.fallback) ? result.fallback : (options[0]?.id ?? null);
  }
  if (!options.length || !fallback) return null;
  const ruled = ruleFor(spec.trigger)?.(c, draft, options);
  if (ruled && options.some((o) => o.id === ruled)) fallback = ruled;
  const time = c.time;
  const q: WorldQuestion = {
    id: c.newNumber(),
    agentId: spec.agentId,
    side: spec.side,
    trigger: spec.trigger,
    raisedAt: time,
    dueAt: time + c.config.timing.reactionS(spec.quality ?? 4),
    title,
    question,
    doctrine: template.doctrine,
    template: template.id,
    path: result.path,
    options,
    fallback,
    about: draft.about,
    // Nothing to choose: taken as a drill, without asking anyone.
    ...(options.length === 1 ? { asked: true, answer: { optionId: options[0].id, by: "rules" as const, at: time, rationale: result.kind === "do" ? "drill" : "only option" } } : {}),
  };
  c.world.questions.push(q);
  c.log({ side: spec.side, domain: "decision", kind: "question", text: `${spec.agentId}: ${title}${options.length > 1 ? ` (${options.map((o) => o.id).join(" / ")})` : ` → ${options[0].summary}`}`, ids: [spec.agentId] });
  return q;
}

/**
 * Carry out an answer: the chosen option's handler runs against the state,
 * the question is closed, and the agent remembers what it chose.
 */
export function answerQuestion(
  state: RtState,
  config: RtConfig,
  questionId: number,
  optionId: string,
  by: "jev" | "rules",
  rationale?: string,
): { state: RtState; events: RtEvent[]; log: WorldLogItem[]; question?: WorldQuestion; option?: WorldOption } {
  const q = state.world?.questions.find((one) => one.id === questionId);
  if (!q) return { state, events: [], log: [] };
  const option = q.options.find((o) => o.id === optionId) ?? q.options.find((o) => o.id === q.fallback)!;
  const result = withWorldCtx(state, config, (c) => {
    c.world.questions.splice(c.world.questions.findIndex((one) => one.id === questionId), 1);
    const alive = c.landAlive(q.agentId) || (c.entity(q.agentId)?.status ?? "destroyed") !== "destroyed";
    if (!alive) return;
    handlerFor(option.action)?.apply(c, q, option);
    const entity = c.entity(q.agentId);
    if (entity) {
      c.setEntity(q.agentId, { history: [...entity.history, { time: c.time, chose: option.summary, by: (by === "jev" ? "jev" : q.options.length === 1 ? "drill" : "rules") as "jev" | "rules" | "drill" }].slice(-8) });
    }
    c.log({ side: q.side, domain: "decision", kind: "answer", text: `${q.agentId} (${q.trigger}): ${option.summary}${by === "jev" ? " — Jev" : ""}${rationale ? ` (${rationale})` : ""}`, ids: [q.agentId] });
  });
  return { ...result, question: q, option };
}

/** Questions raised and not yet handed to a decider. */
export function unaskedQuestions(state: RtState): WorldQuestion[] {
  return state.world?.questions.filter((q) => !q.asked) ?? [];
}

/** Mark questions as handed to a decider. */
export function markAsked(state: RtState, ids: ReadonlySet<number>): RtState {
  if (!state.world || ids.size === 0) return state;
  return { ...state, world: { ...state.world, questions: state.world.questions.map((q) => (ids.has(q.id) ? { ...q, asked: true } : q)) } };
}
