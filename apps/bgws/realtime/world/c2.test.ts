// M3, battalion C2 (W4): a battalion HQ and three company HQs, each on its
// own nets; plans made in sim-time and sent down as FRAGOs that units act on
// only when they arrive; Claude re-planning without pausing; a destroyed HQ
// visibly slowing reports; a unit cut off acting on its intent.
import { describe, expect, it } from "vitest";
import type { RtState } from "../engine/types";
import type { PlannerSeat } from "./command/seat";
import { heuristicPlan } from "./command/heuristicPlanner";
import { at, kitGame, runKit, type KitUnit } from "./testkit";
import type { HqSpec } from "./create";

/** Blue: a battalion HQ 9 km back, three companies of two platoons, each with its HQ. Red: a platoon line. */
function battalion(options: { seat?: HqSpec["seat"]; killCompanyHqs?: boolean; seed?: string } = {}) {
  const units: KitUnit[] = [{ id: "bn", side: "blue", platform: "generic/cp-battalion", count: 3, east: 0, north: -9000, commandRating: 3 }];
  const hqs: HqSpec[] = [{ id: "blue-bn", side: "blue", echelon: "battalion", postId: "bn", subordinates: ["blue-a", "blue-b", "blue-c"], planner: "land", seat: options.seat ?? "heuristic" }];
  ["a", "b", "c"].forEach((coy, i) => {
    const east = (i - 1) * 2500;
    units.push({ id: `${coy}-hq`, side: "blue", platform: "generic/cp-company", count: 2, east, north: -2500, commandRating: 2 });
    units.push({ id: `${coy}-1`, side: "blue", platform: "generic/mbt", count: 4, east: east - 400, north: -1500 });
    units.push({ id: `${coy}-2`, side: "blue", platform: "generic/ifv", count: 3, east: east + 400, north: -1500 });
    hqs.push({ id: `blue-${coy}`, side: "blue", echelon: "company", postId: `${coy}-hq`, parent: "blue-bn", subordinates: [`${coy}-1`, `${coy}-2`], planner: "land", seat: "heuristic", alternate: "blue-bn" });
  });
  for (let i = 0; i < 3; i++) units.push({ id: `r-${i}`, side: "red", platform: "generic/infantrySection", count: 3, east: (i - 1) * 2500, north: 1500, order: { kind: "hold" } });
  const game = kitGame({ units, hqs, comms: "radio", seed: options.seed ?? "bn", scope: { echelons: { highest: "battalion", lowest: "platoon" }, planners: ["land"] }, maxDurationS: 3 * 3600 });
  if (options.killCompanyHqs) {
    for (const id of ["a-hq", "b-hq", "c-hq"]) {
      const fe = game.state.game.forceElements[id];
      game.state = { ...game.state, game: { ...game.state.game, forceElements: { ...game.state.game.forceElements, [id]: { ...fe, combatStrength: 0 } } } };
    }
  }
  return game;
}

describe("battalion C2 (W4)", () => {
  it("plans in sim-time and sends them down as FRAGOs the units act on when they arrive", async () => {
    const runner = await runKit(battalion(), 1800);
    const w = runner.state.world!;
    const plans = w.log.filter((l) => l.kind === "plan");
    expect(plans.some((l) => l.text.startsWith("blue-bn issues"))).toBe(true);
    // Companies planned after the battalion's guidance reached them.
    const bnAt = plans.find((l) => l.text.startsWith("blue-bn"))!.time;
    expect(plans.some((l) => l.text.startsWith("blue-a") && l.time > bnAt)).toBe(true);
    // Planning took time: nothing was issued before the battalion's planning time.
    expect(bnAt).toBeGreaterThanOrEqual(900);
    // FRAGOs went from each company post to its own platoons.
    const fragos = w.messages.filter((m) => m.kind === "frago" && m.delivered);
    expect(fragos.some((m) => m.from === "a-hq" && m.to === "a-1")).toBe(true);
    expect(fragos.some((m) => m.from === "b-hq" && m.to === "b-2")).toBe(true);
    expect(fragos.every((m) => m.dueAt > m.sentAt)).toBe(true);
    // The platoons are on the plan's orders.
    expect(runner.state.units["a-1"].orders?.by).toBe("heuristic");
  });

  it("Claude in the battalion seat re-plans while the clock runs, and the plan goes out when ready", async () => {
    const asked: number[] = [];
    const claude: PlannerSeat = {
      name: "fake-claude",
      async plan(state: RtState, request) {
        asked.push(state.time);
        await new Promise((r) => setTimeout(r, 5));
        return { ...heuristicPlan(state, state.world!.hqs[request.hq]), by: "claude", plan: "fake Claude plan" };
      },
    };
    const game = battalion({ seat: "claude" });
    const { InThreadRunner } = await import("../engine/runnerHandle");
    const runner = new InThreadRunner(game.state, game.config, { planners: { blue: claude }, logLimit: 5000 });
    await runner.advance(1800);
    const w = runner.state.world!;
    expect(asked.length).toBeGreaterThan(0);
    const issued = w.log.find((l) => l.text.startsWith("blue-bn issues its plan (claude)"));
    expect(issued, JSON.stringify(w.log.filter((l) => l.domain === "c2").slice(0, 8))).toBeDefined();
    // Asked at t=0 (the initial plan); issued only when the planning time was up — the clock never paused for it.
    expect(issued!.time).toBeGreaterThanOrEqual(900);
  });

  it("a destroyed company HQ visibly slows what reaches the battalion", async () => {
    const intact = await runKit(battalion({ seed: "relay" }), 2400);
    const cut = await runKit(battalion({ seed: "relay", killCompanyHqs: true }), 2400);
    const reached = (r: Awaited<ReturnType<typeof runKit>>) => r.state.world!.stats.reportLatencyByHq["blue-bn"] ?? [];
    const mean = (xs: number[]) => (xs.length ? xs.reduce((s, x) => s + x, 0) / xs.length : Infinity);
    // With the relays gone, fewer reports reach the battalion, and later.
    expect(reached(cut).length < reached(intact).length || mean(reached(cut)) > mean(reached(intact))).toBe(true);
    expect(cut.state.world!.log.some((l) => l.kind === "hqLost")).toBe(true);
    expect(cut.state.world!.log.some((l) => l.kind === "seatMoved")).toBe(true);
  });

  it("a unit cut off from its HQ acts on intent", async () => {
    const game = kitGame({
      units: [
        { id: "hq", side: "blue", platform: "generic/cp-company", count: 2, east: 0, north: -12_000, commandRating: 2 },
        { id: "lost", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0 },
        { id: "far", side: "red", platform: "generic/infantrySection", count: 1, east: 9000, north: 9000, order: { kind: "hold" } },
      ],
      hqs: [{ id: "blue-hq", side: "blue", echelon: "company", postId: "hq", subordinates: ["lost"] }],
      comms: "radio",
      maxDurationS: 3 * 3600,
    });
    // Its radios are out: only a courier gets through, slowly.
    const w = game.state.world!;
    game.state = { ...game.state, world: { ...w, land: { ...w.land, lost: { ...w.land.lost, damage: { ...w.land.lost.damage, comms: true } } } } };
    const goal = at(0, 2000);
    const unit = game.state.units.lost;
    game.state = {
      ...game.state,
      units: {
        ...game.state.units,
        lost: {
          ...unit,
          order: { kind: "overwatch", phase: 0 },
          orders: {
            task: "wait for the word, then advance",
            phases: [
              { label: "wait for the word", order: { kind: "overwatch" } },
              { label: "advance", order: { kind: "move", to: goal, mode: "tactical" } },
            ],
            phase: 0,
            intent: "be on the ridge by the time the attack goes in",
            urgency: "whenAble",
            roe: "withinShortRange",
            onContact: "engage",
            boundaries: [],
            by: "heuristic",
            issuedAt: 0,
          },
        },
      },
    };
    const runner = await runKit(game, 1800);
    const log = runner.state.world!.log;
    expect(log.some((l) => l.kind === "outOfComms" && l.text.startsWith("lost"))).toBe(true);
    expect(log.some((l) => l.text.startsWith("lost (outOfComms): Act on the commander's intent"))).toBe(true);
    expect(runner.state.units.lost.orders?.phase).toBe(1);
  });
});
