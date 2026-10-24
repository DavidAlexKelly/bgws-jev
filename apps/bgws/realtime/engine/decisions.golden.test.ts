// ── decisions.golden.test.ts ─────────────────────────────────────────────
// Pins what the decision layer offers TODAY, before it is rebuilt as
// decision graphs (docs/plan/DECISION_FLOW_GRAPHS.md).
//
// Two sources of decisions:
//   1. every hand-built scene in situations.ts — the cases someone judged
//      worth naming;
//   2. 20 whole seeded games, played by the rules decider, recording every
//      decision any unit on either side was asked.
//
// For each decision it records the point, the events that raised it, every
// option offered (id, the words Jev would see, the order it issues, any ROE it
// sets) and the rules' fallback. That is everything the graph rebuild has to
// reproduce: the snapshots under __golden__/ must not change when the old
// switch statements are replaced. If they do, the diff says exactly which
// unit, at which moment, was offered something different.
//
// Updating them on purpose: `npx vitest run -u` — and say why in the PR.

import { describe, expect, it } from "vitest";
import { scenarioFactory } from "../../lib/forceBuilder";
import { flatTerrain } from "../../lib/lineOfSight";
import type { Side } from "../../lib/state";
import { createRng } from "../../rules/dice";
import { COMBINED_ARMS_V1, SYMMETRIC_CONTROL_V1, type ForceList } from "../../rules/forceList";
import { HOUSE_V1 } from "../../rules/ruleset";
import { ruleDecider, type RtDecider, type RtDecisionRequest } from "./deciders";
import { createRealtimeState } from "./engine";
import { heuristicInitialOrders } from "./orders";
import { RealtimeRunner } from "./runner";
import { SITUATIONS } from "./situations";
import { DEFAULT_TIMING } from "./timing";
import type { RtConfig, RtOption } from "./types";

/** One option, as one line: everything the graph must reproduce about it. */
function optionLine(option: RtOption): string {
  const roe = option.roe ? ` roe=${JSON.stringify(option.roe)}` : "";
  return [
    `    ${option.id} → ${option.order.kind}${roe} — ${option.summary}`,
    // The figures behind the words (P(located), damage odds) and the radio
    // request the option sends, if any: Jev sees both, so both are pinned.
    ...(option.exact ? [`        exact: ${option.exact}`] : []),
    ...(option.message ? [`        message: ${JSON.stringify(option.message)}`] : []),
  ].join("\n");
}

/** One decision request, as a block of lines. */
function requestBlock(request: RtDecisionRequest, heading: string): string {
  const events = request.events.map((event) => event.kind).join(" + ");
  return [
    `${heading} ${request.unitId} ${request.point} [${events}] fallback=${request.fallback}`,
    ...request.options.map(optionLine),
  ].join("\n");
}

describe("the decision layer, pinned before the graph rebuild", () => {
  it("offers the same options in every named situation", async () => {
    const blocks = SITUATIONS.map((situation) => {
      const { request } = situation.build();
      return requestBlock(request, `# ${situation.id}`);
    });
    await expect(blocks.join("\n\n") + "\n").toMatchFileSnapshot("__golden__/situations.txt");
  });

  const LISTS: [string, ForceList][] = [
    ["symmetric", SYMMETRIC_CONTROL_V1],
    ["combined-arms", COMBINED_ARMS_V1],
  ];
  const SEEDS = Array.from({ length: 10 }, (_, i) => `golden${i}`);

  for (const [name, list] of LISTS) {
    it(`asks the same questions throughout ${SEEDS.length} seeded ${name} games`, async () => {
      const games: string[] = [];
      for (const seed of SEEDS) {
        const cfg: RtConfig = {
          ruleset: HOUSE_V1,
          terrain: flatTerrain(),
          rng: createRng(seed),
          timing: DEFAULT_TIMING,
        };
        const lines: string[] = [];
        // The rules decider, unchanged, with every request written down on
        // the way through — so the game plays exactly as it would without it.
        const recording = (side: Side): RtDecider => ({
          name: "rules (recorded)",
          decide(state, _side, requests, config) {
            for (const request of requests) {
              lines.push(requestBlock(request, `t=${state.time} ${side}`));
            }
            return ruleDecider.decide(state, side, requests, config);
          },
        });

        let state = createRealtimeState(scenarioFactory(list, HOUSE_V1)());
        state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
        const runner = new RealtimeRunner(state, cfg, {
          deciders: { blue: recording("blue"), red: recording("red") },
        });
        while (!runner.state.over) await runner.advance(300);

        const over = runner.state.over;
        games.push(
          [
            `## seed ${seed}: ${lines.length} decisions, ` +
              `over at t=${runner.state.time} (${over?.winner ?? "draw"})`,
            ...lines,
          ].join("\n"),
        );
      }
      await expect(games.join("\n\n") + "\n").toMatchFileSnapshot(`__golden__/games-${name}.txt`);
    }, 300_000);
  }
});
