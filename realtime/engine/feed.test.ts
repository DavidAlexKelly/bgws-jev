/** The feed as stream rows: every row fits the event stream's schema. */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { scenarioFactory } from "../../lib/forceBuilder";
import { flatTerrain } from "../../lib/lineOfSight";
import { createRng } from "../../rules/dice";
import { SYMMETRIC_CONTROL_V1 } from "../../rules/forceList";
import { HOUSE_V1 } from "../../rules/ruleset";
import { EventStreamQueue } from "../../data/eventStreamQueue";
import { createRealtimeState } from "./engine";
import { describeEntry, streamRow, type StreamRow } from "./feed";
import { heuristicInitialOrders } from "./orders";
import { RealtimeRunner, type RtLogEntry } from "./runner";
import { DEFAULT_TIMING } from "./timing";
import type { RtState } from "./types";

beforeEach(() => {
  for (const k of ["groupCollapsed", "groupEnd", "log", "info", "table", "warn"] as const) {
    vi.spyOn(console, k).mockImplementation(() => {});
  }
});

/** The stream's schema: column → [type, nullable], and each struct's fields. */
const COLUMNS: Record<string, [string, boolean]> = {
  runId: ["string", false],
  sequence: ["number", false],
  emittedAt: ["number", false],
  simTimeS: ["number", false],
  gameSeed: ["string", false],
  entryType: ["string", false],
  side: ["string", false],
  unitId: ["string", true],
  unitLat: ["number", true],
  unitLng: ["number", true],
  text: ["string", false],
  event: ["object", true],
  shot: ["object", true],
  decision: ["object", true],
  flag: ["object", true],
  message: ["object", true],
};
const STRUCTS: Record<string, string[]> = {
  event: ["kind", "detail", "severe", "info", "aboutId", "located", "bearingDeg"],
  shot: ["firerId", "targetId", "targetSide", "result", "narrative", "rangeM", "rounds", "hits", "knockedOut", "pHit"],
  decision: ["decisionPoint", "question", "optionId", "summary", "chosenBy", "fallback", "confidence", "latencyMs", "askedAtS", "rationale", "options"],
  flag: ["kind", "text"],
  message: ["kind", "fromId", "senderId", "toIds", "viaId", "hop", "sentAtS", "enemyId", "text"],
};

async function playedGame(): Promise<{ rows: StreamRow[]; entries: RtLogEntry[] }> {
  const cfg = { ruleset: HOUSE_V1, terrain: flatTerrain(), rng: createRng("feed"), timing: DEFAULT_TIMING, comms: "radio" as const };
  let state: RtState = createRealtimeState(scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1)());
  state = heuristicInitialOrders(heuristicInitialOrders(state, "blue", cfg), "red", cfg);
  const rows: StreamRow[] = [];
  const entries: RtLogEntry[] = [];
  const run = { runId: "run-1", gameSeed: "feed" };
  const runner = new RealtimeRunner(state, cfg, {
    logLimit: 1e6,
    onEntry: (entry, now) => {
      entries.push(entry);
      rows.push(streamRow(entry, now, run, rows.length));
    },
  });
  while (!runner.state.over) await runner.advance(300);
  return { rows, entries };
}

describe("the event stream's rows", () => {
  it("fit the schema: every column, the right types, one struct per row matching its type", async () => {
    const { rows, entries } = await playedGame();
    expect(new Set(rows.map((r) => r.entryType))).toEqual(new Set(["event", "shot", "decision", "flag", "message"]));
    rows.forEach((row, index) => {
      // `message` is there only on message rows (older streams have no such column).
      expect(Object.keys(row).sort()).toEqual(
        Object.keys(COLUMNS)
          .filter((c) => c !== "message" || row.entryType === "message")
          .sort(),
      );
      for (const [column, [type, nullable]] of Object.entries(COLUMNS)) {
        const value = (row as unknown as Record<string, unknown>)[column];
        if (value === undefined && column === "message") continue;
        if (value === null) expect(nullable, column).toBe(true);
        else expect(typeof value, column).toBe(type);
      }
      const structs = Object.keys(STRUCTS).filter((key) => (row as unknown as Record<string, unknown>)[key] != null);
      expect(structs).toEqual([row.entryType]);
      expect(Object.keys((row as unknown as Record<string, object>)[row.entryType]).sort()).toEqual([...STRUCTS[row.entryType]].sort());
      expect(row.sequence).toBe(index);
      expect(row.text).toBe(describeEntry(entries[index]));
    });
    const shot = rows.find((r) => r.shot)!.shot!;
    expect(shot.rounds).toBeGreaterThan(0);
    expect(shot.pHit).toBeGreaterThanOrEqual(0);
    const decision = rows.find((r) => r.decision)!.decision!;
    expect(decision.decisionPoint).toMatch(/^D\d+$/);
    expect(decision.options.length).toBeGreaterThan(0);
    expect(JSON.parse(JSON.stringify(rows[0]))).toEqual(rows[0]);
  }, 60_000);
});

describe("the stream queue", () => {
  const row = (sequence: number) => ({ sequence }) as unknown as StreamRow;

  it("sends in batches, in order", async () => {
    const sent: number[][] = [];
    const queue = new EventStreamQueue(async (rows) => void sent.push(rows.map((r) => r.sequence)), { maxBatch: 2 });
    for (let i = 0; i < 5; i += 1) queue.push(row(i));
    await queue.stop();
    expect(sent).toEqual([[0, 1], [2, 3], [4]]);
    expect(queue.status).toMatchObject({ sent: 5, queued: 0, dropped: 0, lastError: null });
  });

  it("retries a failed batch, then drops it and says why, without stopping", async () => {
    let calls = 0;
    const queue = new EventStreamQueue(async () => {
      calls += 1;
      throw new Error("stream 403: forbidden");
    }, { retries: 2 });
    queue.push(row(0));
    for (let i = 0; i < 3; i += 1) await queue.flush();
    expect(calls).toBe(3);
    expect(queue.status).toMatchObject({ queued: 0, dropped: 1, lastError: "stream 403: forbidden" });
  });
});
