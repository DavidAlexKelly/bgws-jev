// ── realtime/world/deciders.ts ───────────────────────────────────────────
// Who answers a world question (world/questions.ts). The rules decider takes
// the rules' choice at once; Jev (world/jevWorldDecider.ts) is asked in one
// batch per side, as land decisions are.

import type { Side } from "../../lib/state";
import type { RtConfig, RtState } from "../engine/types";
import type { WorldQuestion } from "./types";

export interface WorldAnswer {
  questionId: number;
  optionId: string;
  by: "jev" | "rules";
  rationale?: string;
}

export interface WorldDecider {
  readonly name: string;
  /** Answer every question. Must resolve, never reject: fall back instead. */
  decide(state: RtState, side: Side, questions: WorldQuestion[], config: RtConfig): Promise<WorldAnswer[]>;
}

export const ruleWorldDecider: WorldDecider = {
  name: "rules",
  async decide(_state, _side, questions) {
    return questions.map((q) => ({ questionId: q.id, optionId: q.fallback, by: "rules" as const }));
  },
};
