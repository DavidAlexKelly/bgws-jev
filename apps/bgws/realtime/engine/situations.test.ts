/**
 * The calibration set (situations.ts): the rules' fallback must be sensible
 * in every hand-checked situation. With OPENROUTER_API_KEY set, the same set
 * goes to Jev and a table of confidence thresholds is printed, to set
 * `minConfidence` from this game's own situations.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { JEV_MODEL, parseJevAnswers, type JevCall, type JevRequest } from "../../rules/jev";
import { jevRealtimeDecider } from "./jevDecider";
import { SITUATIONS } from "./situations";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

describe("the calibration set", () => {
  it("covers every decision point", () => {
    const points = new Set(SITUATIONS.map((s) => s.point));
    for (let i = 0; i <= 12; i += 1) expect(points).toContain(`D${i}`);
  });

  for (const situation of SITUATIONS) {
    it(`${situation.id}: the rules take a sensible option — ${situation.story}`, () => {
      const { request } = situation.build();
      expect(request.point).toBe(situation.point);
      expect(request.options.length).toBeGreaterThanOrEqual(2);
      // Every id is unique, and every option says what it is in words.
      expect(new Set(request.options.map((o) => o.id)).size).toBe(request.options.length);
      for (const option of request.options) expect(option.summary).not.toMatch(/\d+ ?%/);
      expect(situation.sensible).toContain(request.fallback);
    });
  }
});

/** Jev over plain fetch, for node. */
function nodeJev(key: string): JevCall {
  return async (request: JevRequest) => {
    const started = Date.now();
    const response = await fetch("https://openrouter.ai/api/alpha/decisions", {
      method: "POST",
      headers: { Authorization: `Bearer ${key}`, "Content-Type": "application/json", "X-Title": "BGWS calibration" },
      body: JSON.stringify({ model: JEV_MODEL, state: request.state, questions: request.questions }),
    });
    const text = await response.text();
    if (!response.ok) throw new Error(`OpenRouter ${response.status}: ${text.slice(0, 300)}`);
    return { answers: parseJevAnswers(JSON.parse(text), request.questions), latencyMs: Date.now() - started };
  };
}

const KEY = process.env.OPENROUTER_API_KEY ?? "";

describe.skipIf(!KEY)("Jev on the calibration set", () => {
  it("reports agreement at each confidence threshold", async () => {
    const rows: { id: string; chose: string; confidence: number; sensible: boolean }[] = [];
    for (const situation of SITUATIONS) {
      const { state, config, request } = situation.build();
      const [decision] = await jevRealtimeDecider({ side: "blue", call: nodeJev(KEY), minConfidence: 0, timeoutMs: 30_000, log: false }).decide(
        state,
        "blue",
        [request],
        config,
      );
      rows.push({
        id: situation.id,
        chose: decision.optionId,
        confidence: decision.trace.confidence ?? 0,
        sensible: situation.sensible.includes(decision.optionId),
      });
    }
    const lines = rows.map((r) => `${r.id.padEnd(24)} ${r.chose.padEnd(14)} ${r.confidence.toFixed(2)} ${r.sensible ? "sensible" : "NOT SENSIBLE"}`);
    for (const threshold of [0, 0.2, 0.25, 0.3, 0.4, 0.5, 0.6]) {
      const taken = rows.filter((r) => r.confidence >= threshold);
      const good = taken.filter((r) => r.sensible).length;
      lines.push(`threshold ${threshold.toFixed(2)}: Jev's answer taken in ${taken.length}/${rows.length}, sensible ${good}/${taken.length}`);
    }
    process.stdout.write(`\n${lines.join("\n")}\n`);
    expect(rows).toHaveLength(SITUATIONS.length);
  }, 300_000);
});
