// The Jev world decider: one choice question per world question, the
// option's doctrine bias in words, and the fallback when Jev is unsure or down.
import { describe, expect, it } from "vitest";
import type { JevCall } from "../../rules/jev";
import type { RtConfig, RtState } from "../engine/types";
import { biasWords, jevWorldDecider } from "./jevWorldDecider";
import type { WorldQuestion } from "./types";

const state = { time: 0, plan: {}, game: { forceElements: { b1: { label: "1 Coy" } } }, world: { entities: {} } } as unknown as RtState;
const config = {} as RtConfig;
const question = (id: number): WorldQuestion => ({
  id,
  agentId: "b1",
  side: "blue",
  trigger: "fireRequestReceived",
  raisedAt: 0,
  dueAt: 30,
  title: "Call for fire",
  question: "Which shooter answers?",
  doctrine: ["Mass fires at the decisive point."],
  template: "t",
  path: [],
  options: [
    { id: "guns", action: "fire", summary: "the guns", bias: 1 },
    { id: "hold", action: "hold", summary: "hold fire", bias: -2 },
  ],
  fallback: "hold",
  about: {},
});

describe("jevWorldDecider", () => {
  it("asks one choice per question and takes a confident answer", async () => {
    let seen: Parameters<JevCall>[0] | undefined;
    const call: JevCall = async (req) => {
      seen = req;
      return { latencyMs: 1, answers: Object.fromEntries(Object.keys(req.questions).map((k) => [k, { type: "choice", choice: "o0", probabilities: { o0: 0.9, o1: 0.1 }, confidence: 0.9 }])) };
    };
    const answers = await jevWorldDecider({ side: "blue", call, log: false }).decide(state, "blue", [question(1), question(2)], config);
    expect(Object.keys(seen!.questions)).toHaveLength(2);
    const q = Object.values(seen!.questions)[0];
    expect(q.type).toBe("choice");
    expect(q.instructions).toContain("Mass fires");
    expect(Object.values((q as { criteria: Record<string, string> }).criteria)[1]).toContain("strongly discouraged");
    expect(answers.map((a) => [a.questionId, a.optionId, a.by])).toEqual([
      [1, "guns", "jev"],
      [2, "guns", "jev"],
    ]);
  });

  it("falls back to the rules when unsure or unreachable", async () => {
    const unsure: JevCall = async (req) => ({ latencyMs: 1, answers: Object.fromEntries(Object.keys(req.questions).map((k) => [k, { type: "choice", choice: "o0", probabilities: {}, confidence: 0.1 }])) });
    const down: JevCall = async () => {
      throw new Error("offline");
    };
    for (const call of [unsure, down]) {
      const [a] = await jevWorldDecider({ side: "blue", call, log: false }).decide(state, "blue", [question(3)], config);
      expect(a).toMatchObject({ questionId: 3, optionId: "hold", by: "rules" });
    }
    expect(biasWords(0)).toBe("");
  });
});
