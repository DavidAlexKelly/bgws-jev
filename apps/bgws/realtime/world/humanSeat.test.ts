// M9, human command (W15): a person takes an HQ's seat and commands through
// the console — seeing only what reaches the HQ, sending orders that travel
// the nets like any other, delayed by EW — with an AAR against the planner.
// And the worker carries world deciders and planners to the main thread.
import { MessageChannel } from "node:worker_threads";
import { describe, expect, it } from "vitest";
import { configFromSpec } from "../engine/configSpec";
import { InThreadRunner, WorkerRunner } from "../engine/runnerHandle";
import { hostRunner, type MessageEndpoint } from "../engine/workerHost";
import { compareWithPlanner, recordFromScenario } from "../headless";
import { PRESET_SCENARIOS } from "../presets";
import { commandable, commanderAnswer, consoleView, frago } from "./command/console";
import type { PlannerSeat } from "./command/seat";
import { ruleWorldDecider, type WorldDecider } from "./deciders";

const record = () => recordFromScenario(PRESET_SCENARIOS.M3());

describe("a person at an HQ (W15)", () => {
  it("takes the battalion's seat: no plan is made for them, and the console shows only what has reached the HQ", async () => {
    const r = record();
    const config = configFromSpec(r.spec);
    const runner = new InThreadRunner(r.initial, config);
    await runner.setSeat("blue-bn", "human");
    await runner.advance(1800);
    const w = runner.state.world!;
    expect(w.log.some((l) => l.text.includes("blue-bn issues its plan (heuristic)"))).toBe(false);
    expect(w.log.some((l) => /over to the commander/.test(l.text))).toBe(true);
    const view = consoleView(runner.state, config, "blue-bn")!;
    expect(view.seat).toBe("human");
    // No truth: every contact in the picture is in the HQ's own COP, and every message has arrived.
    for (const t of view.cop) expect(w.hqs["blue-bn"].cop[t.id]).toBeDefined();
    for (const m of view.inbox) expect(m.arrivedAt).toBeLessThanOrEqual(runner.state.time);
    expect(view.comms.length).toBe(w.hqs["blue-bn"].subordinates.length);
  }, 60_000);

  it("their orders go out over the nets: a unit acts only once its FRAGO has arrived", async () => {
    const r = record();
    const config = configFromSpec(r.spec);
    const runner = new InThreadRunner(r.initial, config);
    await runner.setSeat("blue-bn", "human");
    await runner.advance(60);
    const { units } = commandable(runner.state, "blue-bn");
    expect(units).toContain("b-mortar");
    const at = runner.state.game.forceElements["b-mortar"].position;
    const order = frago(runner.state, { kind: "move", to: { lat: at.lat - 0.01, lng: at.lng }, mode: "march" }, "displace south");
    await runner.command("blue-bn", commanderAnswer("mortars displace", { "b-mortar": order }));
    const sent = consoleView(runner.state, config, "blue-bn")!.sent[0];
    expect(sent.to).toBe("b-mortar");
    expect(["on its way", "arrived"]).toContain(sent.status);
    await runner.advance(600);
    const after = consoleView(runner.state, config, "blue-bn")!.sent.find((s) => s.id === sent.id)!;
    expect(after.status).toBe("arrived");
    expect(after.arrivedAt!).toBeGreaterThan(sent.sentAt);
    expect(runner.state.units["b-mortar"].orders?.task).toBe("displace south");
  }, 60_000);

  it("jamming delays their orders as it delays everyone's", async () => {
    const arrival = async (jam: boolean) => {
      const r = record();
      const config = configFromSpec(r.spec);
      const runner = new InThreadRunner(r.initial, config);
      await runner.setSeat("blue-bn", "human");
      if (jam) {
        const post = runner.state.game.forceElements["b-bn"].position;
        runner.state.world!.jammers.push({ id: "red-jam", side: "red", at: post, radiusM: 20_000, bands: ["vhf", "uhf", "hf", "l"], on: true });
      }
      const at = runner.state.game.forceElements["b-mortar"].position;
      await runner.command("blue-bn", commanderAnswer("move", { "b-mortar": frago(runner.state, { kind: "move", to: at, mode: "march" }, "hold here") }));
      const m = runner.state.world!.messages.filter((x) => x.kind === "frago").at(-1)!;
      return m.lost ? Infinity : m.dueAt - m.sentAt;
    };
    const clear = await arrival(false);
    const jammed = await arrival(true);
    expect(jammed).toBeGreaterThanOrEqual(clear);
  }, 60_000);

  it("the AAR plays the same seed with the planner in their seat, and compares", async () => {
    const r = record();
    r.issued.push({ time: 0, orders: {}, seat: { hq: "blue-bn", seat: "human" } });
    const result = await compareWithPlanner(r, 1200);
    expect(result.person.time).toBe(1200);
    expect(result.planner.time).toBe(1200);
    expect(result.person.strength.blue).toBeGreaterThan(0);
  }, 120_000);
});

describe("the worker carries world deciders and planners to the main thread", () => {
  it("world questions and Claude's plans are answered on the main thread, through the channel", async () => {
    const r = record();
    const { port1, port2 } = new MessageChannel();
    port1.start();
    port2.start();
    const stop = hostRunner(port2 as unknown as MessageEndpoint);
    const asked: string[] = [];
    const counting: WorldDecider = { name: "counting", decide: (state, side, questions, config) => (asked.push(...questions.map((q) => q.trigger)), ruleWorldDecider.decide(state, side, questions, config)) };
    let planned = 0;
    const planner: PlannerSeat = { name: "test planner", plan: async (_state, request) => (planned++, { by: "claude", plan: `planned for ${request.hq}`, orders: {} }) };
    const state = { ...r.initial, world: { ...r.initial.world!, hqs: { ...r.initial.world!.hqs, "blue-bn": { ...r.initial.world!.hqs["blue-bn"], seat: "claude" as const } } } };
    const runner = new WorkerRunner(port1 as unknown as MessageEndpoint, { spec: r.spec, raster: null, initial: state, config: configFromSpec(r.spec) }, { worldDeciders: { blue: counting }, planners: { blue: planner } }, () => {
      stop();
      port1.close();
      port2.close();
    });
    await runner.advance(1200);
    runner.dispose();
    expect(asked.length).toBeGreaterThan(0);
    expect(planned).toBeGreaterThan(0);
    expect(runner.log.some((e) => e.type === "world" && e.item.text.includes("planned for blue-bn"))).toBe(true);
  }, 60_000);
});
