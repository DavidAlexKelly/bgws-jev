// ── realtime/world/jevWorldDecider.ts ────────────────────────────────────
// Jev at the world's decision points (MULTI_DOMAIN_PLAN.md W2 step 4): one
// request per side per moment, one `choice` question per world question,
// as jevRealtimeDecider does for land units.
//
// WHAT JEV IS SHOWN: the question's title and words, the agent's doctrine,
// and each option's summary with its doctrine bias in words. The question
// was raised from what the agent knows (questions.ts), so nothing here reads
// the truth. When Jev is unsure, or cannot be reached, the question's
// fallback stands: the rules' choice.

import type { Side } from "../../lib/state";
import { JEV_MAX_CHOICES, JEV_MODEL, choiceOf, keyed, withDeadline, type JevCall, type JevQuestion, type JevResponse } from "../../rules/jev";
import type { WorldAnswer, WorldDecider } from "./deciders";

export interface JevWorldOptions {
  side: Side;
  call: JevCall;
  /** Doctrine or temperament, put in front of every question. */
  directive?: string;
  timeoutMs?: number;
  /** Below this, Jev's answer is not taken and the fallback stands. */
  minConfidence?: number;
  log?: boolean;
}

/** How an option's doctrine bias reads to Jev. */
export function biasWords(bias: number): string {
  if (bias >= 2) return " [strongly preferred by its doctrine]";
  if (bias > 0) return " [preferred by its doctrine]";
  if (bias <= -2) return " [strongly discouraged by its doctrine]";
  if (bias < 0) return " [discouraged by its doctrine]";
  return "";
}

export function jevWorldDecider(options: JevWorldOptions): WorldDecider {
  const minConfidence = options.minConfidence ?? 0.25;
  const timeoutMs = options.timeoutMs ?? 4000;
  const directive = options.directive?.trim() ? `${options.directive.trim()}\n\n` : "";
  return {
    name: `${JEV_MODEL}-${options.side}-world`,
    async decide(state, side, questions): Promise<WorldAnswer[]> {
      if (!questions.length) return [];
      const asked = keyed(questions, "q");
      const keysFor = new Map<string, { key: string; id: string }[]>();
      const jq: Record<string, JevQuestion> = {};
      for (const { key, item } of asked) {
        const choices = item.options.slice(0, JEV_MAX_CHOICES).map((o, i) => ({ key: `o${i}`, id: o.id, summary: o.summary + biasWords(o.bias) }));
        keysFor.set(key, choices);
        const label = state.game.forceElements[item.agentId]?.label ?? state.world?.entities[item.agentId]?.label ?? item.agentId;
        jq[key] = {
          type: "choice",
          instructions:
            `${directive}You decide for ${item.agentId} (${label}), on the ${side} side. ` +
            (item.doctrine.length ? `Its doctrine: ${item.doctrine.join(" ")}\n\n` : "") +
            `${item.title}. ${item.question}`,
          criteria: Object.fromEntries(choices.map((c) => [c.key, c.summary])),
        };
      }
      const stateSent = {
        you: side,
        ...(state.plan[side] ? { commandersPlan: state.plan[side] } : {}),
        note: "Each question is raised from what that agent knows, not the truth. Choose within its doctrine.",
      };
      let response: JevResponse;
      try {
        response = await withDeadline(options.call({ state: stateSent, questions: jq }), timeoutMs);
      } catch (err) {
        const why = err instanceof Error ? err.message : String(err);
        if (options.log !== false && typeof console !== "undefined") console.warn?.(`[Jev ${side} world] call failed, the rules decided ${questions.length} question(s): ${why}`);
        return questions.map((q) => ({ questionId: q.id, optionId: q.fallback, by: "rules", rationale: `Jev unavailable (${why}) — the rules decided` }));
      }
      return asked.map(({ key, item }) => {
        const answer = choiceOf(response.answers, key);
        const picked = keysFor.get(key)?.find((c) => c.key === answer?.choice);
        if (answer && picked && answer.confidence >= minConfidence) {
          return { questionId: item.id, optionId: picked.id, by: "jev", rationale: `Jev chose ${picked.id} at ${Math.round(answer.confidence * 100)}% confidence` };
        }
        return {
          questionId: item.id,
          optionId: item.fallback,
          by: "rules",
          rationale: answer ? `Jev was unsure (${Math.round((answer.confidence ?? 0) * 100)}%) — the rules chose` : "Jev gave no answer — the rules chose",
        };
      });
    },
  };
}
