import { describe, expect, it } from "vitest";

import { heuristicCommander } from "../rules/commander";
import { createRng } from "../rules/dice";
import { EventLog } from "../rules/events";
import { MEETING_ENGAGEMENT_V1, SYMMETRIC_CONTROL_V1 } from "../rules/forceList";
import { HOUSE_V1 } from "../rules/ruleset";
import { runGame } from "../rules/turnLoop";
import { scenarioFactory } from "./forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "./proceduralTerrain";
import {
  decisionsAt,
  describeResult,
  narrativeAt,
  recordGame,
  stateAt,
  strengthSeries,
  type Replay,
} from "./replay";

const terrain = proceduralTerrain(STANDARD_GROUND);

async function record(seed: string, list = MEETING_ENGAGEMENT_V1): Promise<Replay> {
  return recordGame({
    initial: scenarioFactory(list, HOUSE_V1)(),
    ruleset: HOUSE_V1,
    terrain,
    seed,
    groundSeed: STANDARD_GROUND.seed ?? "test",
    maxTurns: 40,
  });
}

describe("a recorded game is the same game", () => {
  it("agrees with runGame on the winner and the turn count", async () => {
    // THE LOAD-BEARING TEST. If a replay diverges from the harness it is
    // showing a game nobody measured, and every finding in the reports stops
    // describing what a viewer sees.
    const seed = "agree";
    const replay = await record(seed);
    const direct = await runGame(scenarioFactory(MEETING_ENGAGEMENT_V1, HOUSE_V1)(), {
      ruleset: HOUSE_V1,
      terrain,
      rng: createRng(`${seed}:dice`),
      commanders: {
        blue: heuristicCommander("blue", createRng(`${seed}:commander:blue`)),
        red: heuristicCommander("red", createRng(`${seed}:commander:red`)),
      },
      log: new EventLog(),
      maxTurns: 40,
    });

    expect(replay.winner).toBe(direct.winner);
    expect(replay.reason).toBe(direct.reason);
    expect(replay.turns.length).toBe(direct.turns);
  }, 20_000);

  it("replays identically from the same seed", async () => {
    const a = await record("same");
    const b = await record("same");
    expect(a.winner).toBe(b.winner);
    expect(a.turns.length).toBe(b.turns.length);
    expect(strengthSeries(a)).toEqual(strengthSeries(b));
  }, 20_000);

  it("gives a different game for a different seed", async () => {
    const a = await record("seed-a");
    const b = await record("seed-b");
    expect(strengthSeries(a)).not.toEqual(strengthSeries(b));
  }, 20_000);
});

describe("snapshots", () => {
  it("keeps one per turn", async () => {
    const replay = await record("snap");
    expect(replay.turns.length).toBeGreaterThan(0);
    replay.turns.forEach((snapshot, i) => expect(snapshot.turn).toBe(i + 1));
  }, 20_000);

  it("index 0 is setup, before anything has happened", async () => {
    const replay = await record("snap");
    const setup = stateAt(replay, 0);
    expect(setup.turn).toBe(1);
    for (const fe of Object.values(setup.forceElements)) {
      expect(fe.combatStrength).toBe(fe.combatStrengthStart);
      expect(fe.morale).toBe("good");
    }
  }, 20_000);

  it("clamps a scrubber position past the end", async () => {
    const replay = await record("snap");
    expect(stateAt(replay, 9_999)).toBe(stateAt(replay, replay.turns.length));
  }, 20_000);

  it("loses strength over the game rather than gaining it", async () => {
    // Catches a snapshot that aliases a mutable state object: if every turn
    // pointed at the same live state the series would be flat, and if turns
    // were recorded out of order it would rise.
    const series = strengthSeries(await record("attrition"));
    const total = series.map((s) => s.blue + s.red);
    for (let i = 1; i < total.length; i += 1) {
      expect(total[i]).toBeLessThanOrEqual(total[i - 1]);
    }
    expect(total[total.length - 1]).toBeLessThan(total[0]);
  }, 20_000);
});

describe("narrative", () => {
  it("gives readable lines for a turn", async () => {
    const replay = await record("story");
    const lines = narrativeAt(replay, 1);
    expect(lines.length).toBeGreaterThan(0);
    for (const line of lines) expect(line.length).toBeGreaterThan(0);
    expect(lines.some((line) => line.includes("Initiative"))).toBe(true);
  }, 20_000);

  it("is empty at setup and past the end", async () => {
    const replay = await record("story");
    expect(narrativeAt(replay, 0)).toEqual([]);
    expect(narrativeAt(replay, replay.turns.length + 5)).toEqual([]);
  }, 20_000);

  it("reports decisions with how many options there were", async () => {
    // Option count is the interesting part: it is the measure of whether a
    // commander had a choice at all.
    const replay = await record("story");
    const decisions = decisionsAt(replay, 1);
    expect(decisions.length).toBeGreaterThan(0);
    for (const decision of decisions) {
      expect(["blue", "red"]).toContain(decision.side);
      expect(decision.from).toBeGreaterThan(0);
      expect(decision.summary).toBeTruthy();
    }
  }, 20_000);
});

describe("provenance", () => {
  it("records everything needed to reproduce the game", async () => {
    const replay = await record("prov", SYMMETRIC_CONTROL_V1);
    expect(replay.provenance.rulesetId).toBe(HOUSE_V1.id);
    expect(replay.provenance.scenarioId).toBe(SYMMETRIC_CONTROL_V1.id);
    expect(replay.provenance.seed).toBe("prov");
    expect(replay.provenance.groundSeed).toBe(STANDARD_GROUND.seed);
  }, 20_000);

  it("describes the result in words", async () => {
    const replay = await record("prov");
    expect(describeResult(replay)).toMatch(/(blue|red) wins|drawn/);
    expect(describeResult(replay)).toContain("turn");
  }, 20_000);
});
