// ── behaviour/bench.ts ───────────────────────────────────────────────────
// The test bench (docs/plan/DECISION_FLOW_GRAPHS.md §5.2, §7 step 8): put a
// template in a calibration scene (engine/situations.ts) and see what it does
// there — the path through the graph, every option it offers and why the
// others are missing, and what the rules would choose. Two templates side by
// side on the same scene show exactly what a change does.
//
// It runs the game's own code (`landOptions`, `ruleFallback`) on the scene's
// state; nothing here decides anything the game would not.

import { DECISION_POINTS, ruleFallback } from "../engine/decisions";
import { SITUATIONS, type Situation } from "../engine/situations";
import type { RtOption } from "../engine/types";
import { landOptions, templateChain, type LandTrace } from "./actions/land";
import type { Bias, Template } from "./types";

export type BenchStatus =
  /** On offer here. */
  | "offered"
  /** The template switched it off. */
  | "removed"
  /** Its condition is not met in this scene. */
  | "condition"
  /** Allowed, but there is nothing of that kind to do here (no target, no cover, out of range…). */
  | "nothing here";

export interface BenchRow {
  action: string;
  status: BenchStatus;
  bias: Bias;
  /** The options it produced, as the unit would be offered them. */
  options: RtOption[];
}

export interface BenchResult {
  situation: Pick<Situation, "id" | "point" | "story" | "sensible">;
  question: string;
  templateId: string;
  /** The template, then what it inherits from. */
  lineage: string[];
  doctrine: string[];
  /** Graph nodes walked, trigger first. */
  path: string[];
  rows: BenchRow[];
  /** The options the unit is offered, in order (after its orders' boundaries). */
  offered: RtOption[];
  /** What the rules would choose: the game's choice whenever Jev is not asked. */
  rulesChoice: string;
  /** Whether the rules' choice is one a sensible leader might take (situations.ts, judged by hand). */
  sensibleChoice: boolean;
}

/** Run a template in a scene. `templates` are the user templates it may use. */
export function benchScene(situationId: string, templateId: string, templates: readonly Template[] = []): BenchResult {
  const situation = SITUATIONS.find((s) => s.id === situationId);
  if (!situation) throw new Error(`No scene "${situationId}"`);
  const built = situation.build();
  const { state, request } = built;
  const config = {
    ...built.config,
    behaviours: { ...built.config.behaviours, [request.unitId]: templateId },
    behaviourTemplates: [...templates],
  };
  const trace: LandTrace = { steps: [] };
  const offered = landOptions(state, request.unitId, situation.point, request.event, config, built.context ?? {}, trace);
  const walk = trace.walk;
  const rows: BenchRow[] = trace.steps.map((step) => ({
    action: step.action,
    status: step.produced.length ? "offered" : "nothing here",
    bias: step.bias,
    options: step.produced,
  }));
  if (walk?.kind === "choose") {
    for (const d of walk.dropped) rows.push({ action: d.action, status: d.why, bias: 0, options: [] });
  }
  const rulesChoice = ruleFallback(state, request.unitId, situation.point, offered);
  return {
    situation: { id: situation.id, point: situation.point, story: situation.story, sensible: situation.sensible },
    question: walk?.kind === "choose" ? walk.question : DECISION_POINTS[situation.point].question,
    templateId: trace.templateId ?? templateId,
    lineage: templateChain(trace.templateId ?? templateId, config.behaviourTemplates),
    doctrine: walk?.kind === "choose" ? walk.doctrine : [],
    path: walk?.path ?? [],
    rows,
    offered,
    rulesChoice,
    sensibleChoice: situation.sensible.includes(rulesChoice),
  };
}

/** The scenes the bench can use, in their file order. */
export const BENCH_SCENES = SITUATIONS.map((s) => ({ id: s.id, point: s.point, story: s.story }));
