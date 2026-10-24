// Headless play (W0): a game plays the same however it is driven — in steps
// of any size, from a record written to JSON and read back, with orders
// issued mid-game, and through the worker host — and the fingerprint says so.
import { MessageChannel } from "node:worker_threads";
import { describe, expect, it } from "vitest";
import { placedFromList } from "../lib/forceBuilder";
import { COMBINED_ARMS_V1, SYMMETRIC_CONTROL_V1 } from "../rules/forceList";
import { configFromSpec } from "./engine/configSpec";
import { EventFingerprint, fromJson, toJson } from "./engine/fingerprint";
import { heuristicOrders } from "./engine/orders";
import { InThreadRunner, WorkerRunner } from "./engine/runnerHandle";
import { hostRunner, type MessageEndpoint } from "./engine/workerHost";
import { decodeRecord, encodeRecord, playRecord, recordFromScenario, type GameRecord } from "./headless";
import { SCENARIO_FORMAT, SCENARIO_VERSION, type RealtimeScenario } from "./scenario";
import { scopeFor } from "./scope";

function scenario(over: Partial<RealtimeScenario> = {}): RealtimeScenario {
  return {
    format: SCENARIO_FORMAT,
    version: SCENARIO_VERSION,
    name: "Headless test",
    savedAt: "2026-10-24T00:00:00.000Z",
    seed: "headless-1",
    ground: { choice: "generated", seed: "ground-1" },
    scope: { ...scopeFor("skirmish"), durationMin: 60 },
    rules: { comms: "radio", breakpointPct: 50 },
    command: { useCommander: false, model: "any", useJev: false, directives: { blue: "", red: "" } },
    forces: placedFromList(SYMMETRIC_CONTROL_V1),
    behaviours: {},
    behaviourTemplates: [],
    ...over,
  };
}

describe("fingerprint JSON", () => {
  it("keeps what plain JSON loses: -Infinity and byte arrays", () => {
    const value = { a: -Infinity, b: Infinity, c: new Uint8Array([0, 7, 255]), d: [1, "x", null] };
    const back = fromJson<typeof value>(toJson(value));
    expect(back.a).toBe(-Infinity);
    expect(back.b).toBe(Infinity);
    expect(back.c).toEqual(new Uint8Array([0, 7, 255]));
    expect(back.d).toEqual([1, "x", null]);
  });

  it("depends on every entry and their order", () => {
    const of = (entries: unknown[]) => {
      const f = new EventFingerprint();
      for (const e of entries) f.add(e);
      return f.value.fingerprint;
    };
    expect(of([{ a: 1 }, { b: 2 }])).toBe(of([{ a: 1 }, { b: 2 }]));
    expect(of([{ a: 1 }, { b: 2 }])).not.toBe(of([{ b: 2 }, { a: 1 }]));
    expect(of([{ a: 1 }])).not.toBe(of([{ a: 2 }]));
  });
});

describe("headless play", () => {
  it("the same game whatever the step size, and the same after a trip through JSON", async () => {
    const record = recordFromScenario(scenario());
    const whole = await playRecord(record);
    expect(whole.entries).toBeGreaterThan(20);
    expect(whole.state.over).toBeDefined();
    for (const chunkS of [1, 7]) expect((await playRecord(record, { chunkS })).fingerprint).toBe(whole.fingerprint);
    const { record: read, errors } = decodeRecord(encodeRecord(record));
    expect(errors).toEqual([]);
    expect((await playRecord(read!)).fingerprint).toBe(whole.fingerprint);
  });

  it("orders issued mid-game replay at the moment they were issued", async () => {
    const base = recordFromScenario(scenario({ forces: placedFromList(COMBINED_ARMS_V1), seed: "headless-2" }));
    // Play it as the browser would: run, pause at t=40, give blue new orders, run on.
    const fingerprint = new EventFingerprint();
    const runner = new InThreadRunner(base.initial, configFromSpec(base.spec), { onEntry: (entry) => fingerprint.add(entry) });
    await runner.advance(40);
    const orders = heuristicOrders(runner.state, "blue");
    const issuedAt = runner.state.time;
    await runner.issueOrders(orders.orders, { side: "blue", text: orders.plan });
    while (!runner.state.over) await runner.advance(300);

    const record: GameRecord = { ...base, issued: [{ time: issuedAt, orders: orders.orders, plan: { side: "blue", text: orders.plan } }] };
    const replay = await playRecord(fromJson<GameRecord>(encodeRecord(record)));
    expect(replay.fingerprint).toBe(fingerprint.value.fingerprint);
    expect(replay.entries).toBe(fingerprint.value.entries);
    // And it is a different game from the one without them.
    expect((await playRecord(base)).fingerprint).not.toBe(replay.fingerprint);
  });

  it("the worker host plays the same stream", async () => {
    const record = recordFromScenario(scenario({ seed: "headless-3" }));
    const here = await playRecord(record);
    const { port1, port2 } = new MessageChannel();
    port1.start();
    port2.start();
    const stop = hostRunner(port2 as unknown as MessageEndpoint);
    const fingerprint = new EventFingerprint();
    const there = new WorkerRunner(
      port1 as unknown as MessageEndpoint,
      { spec: record.spec, raster: null, initial: record.initial, config: configFromSpec(record.spec) },
      { onEntry: (entry) => fingerprint.add(entry) },
      () => {
        stop();
        port1.close();
        port2.close();
      },
    );
    while (!there.state.over) await there.advance(300);
    there.dispose();
    expect(fingerprint.value).toEqual({ fingerprint: here.fingerprint, entries: here.entries });
  });

  it("says what it cannot replay", async () => {
    expect(() => recordFromScenario(scenario({ ground: { choice: "raster", seed: "x" } }))).toThrow(/needs that raster/);
    expect(decodeRecord('{"format":"nope"}').errors[0]).toMatch(/Not a game record/);
  });
});
