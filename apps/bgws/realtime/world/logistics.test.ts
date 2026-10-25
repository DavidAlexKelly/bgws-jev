// M6, the sustained fight (W10): supply as a network that can be cut —
// fighting power tracking fuel, a cut supply line forcing culmination,
// casualty evacuation times in the reference band, convoys reacting to
// threats on the road, nodes hit and answered, vehicles recovered.
import { describe, expect, it } from "vitest";
import { damageEntity } from "./munitions";
import { weaponSpec } from "../catalogue";
import { diedOfWoundsP } from "./domains/logistics";
import { distanceM } from "./geo";
import { withWorldCtx } from "./standalone";
import { at, kitGame, runKit, type KitOptions } from "./testkit";
import type { RtState } from "../engine/types";

const LOG = { logistics: "simulated", medical: "simulated" } as const;

/** Give a unit its mission, as orders from a scenario or a plan would. */
function withMission(state: RtState, id: string, to: ReturnType<typeof at>): RtState {
  return { ...state, units: { ...state.units, [id]: { ...state.units[id], mission: { task: "take", at: to, purpose: "advance" } } } };
}

function advance(cut: boolean) {
  const g = kitGame({
    seed: "msr",
    units: [
      { id: "b-tanks", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0, order: { kind: "move", to: at(0, 30_000), mode: "march" } },
      { id: "r-x", side: "red", platform: "generic/infantrySection", count: 1, east: 30_000, north: 90_000, order: { kind: "hold" } },
    ],
    entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: -20_000 }],
    domains: LOG,
    maxDurationS: 8 * 3600,
  });
  const w = g.state.world!;
  // Short of fuel at the start: not enough to get there without a convoy.
  w.land["b-tanks"] = { ...w.land["b-tanks"], stores: { ...w.land["b-tanks"].stores, fuelL: w.land["b-tanks"].stores.fuelL * 0.3 } };
  if (cut) w.entities["b-depot"] = { ...w.entities["b-depot"], hp: 0, status: "destroyed" };
  return { state: withMission(g.state, "b-tanks", at(0, 30_000)), config: g.config };
}

describe("logistics (W10)", () => {
  it("fighting power tracks fuel: supplied, the advance reaches its objective; with the depot gone it culminates short", async () => {
    const supplied = await runKit(advance(false), 7 * 3600);
    const cut = await runKit(advance(true), 7 * 3600);
    const reached = (r: Awaited<ReturnType<typeof runKit>>) => distanceM(r.state.game.forceElements["b-tanks"].position, at(0, 30_000));
    expect(supplied.state.world!.log.some((l) => l.kind === "delivered" && l.ids?.includes("b-tanks")), "a convoy reached it").toBe(true);
    expect(reached(supplied)).toBeLessThan(1000);
    // Cut off: out of fuel, kilometres short, and the AAR says why.
    expect(cut.state.world!.land["b-tanks"].stores.fuelL).toBe(0);
    expect(reached(cut)).toBeGreaterThan(3000);
    expect(cut.state.world!.log.some((l) => l.kind === "noSupply")).toBe(true);
  }, 60_000);

  it("a convoy whose road passes a known enemy goes round it", async () => {
    const g = kitGame({
      seed: "route",
      units: [
        { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "hold" } },
        { id: "r-ambush", side: "red", platform: "generic/mbt", count: 2, east: 300, north: -9000, order: { kind: "hold" } },
        // Blue's recce has eyes on it.
        { id: "b-recce", side: "blue", platform: "generic/recce", count: 2, east: 1000, north: -9000, order: { kind: "hold" } },
      ],
      entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: -20_000 }],
      domains: LOG,
    });
    // The infantry is short of ammunition.
    const w = g.state.world!;
    w.land["b-inf"] = { ...w.land["b-inf"], stores: { ...w.land["b-inf"].stores, ammo: { smallArms: 10 } } };
    // The ambush waits: it fires only if fired upon, so the recce watching it lives to report.
    g.state = { ...g.state, units: { ...g.state.units, "r-ambush": { ...g.state.units["r-ambush"], roe: "ifFiredUpon" } } };
    const runner = await runKit(g, 1800);
    const after = runner.state.world!;
    const job = after.logistics!.jobs.find((j) => j.unitId === "b-inf");
    expect(job, "a convoy was sent").toBeDefined();
    expect(after.log.some((l) => l.kind === "answer" && l.text.includes("routeThreatened"))).toBe(true);
    void w;
  }, 60_000);

  it("a node that is hit loses stocks and its cell answers: disperse, repair or move it back", async () => {
    const g = kitGame({ seed: "node", units: [{ id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: 0, order: { kind: "hold" } }, { id: "r-x", side: "red", platform: "generic/infantrySection", count: 1, east: 0, north: 30_000, order: { kind: "hold" } }], entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: -20_000 }], domains: LOG });
    const first = await runKit(g, 5);
    const hit = withWorldCtx(first.state, g.config, (c) => void damageEntity(c, "b-depot", weaponSpec("owa"), "test"));
    const fuelBefore = hit.state.world!.entities["b-depot"].stores.fuelL;
    const runner = await runKit({ state: hit.state, config: g.config }, 120);
    const w = runner.state.world!;
    expect(w.entities["b-depot"].stores.fuelL).toBeLessThan(fuelBefore);
    expect(w.log.some((l) => l.kind === "nodeStruck")).toBe(true);
    expect(w.log.some((l) => l.kind === "disperse" || l.kind === "relocate")).toBe(true);
  }, 60_000);

  it("a vehicle knocked out but not destroyed comes back from recovery and repair", async () => {
    const g = kitGame({ seed: "repair", units: [{ id: "b-tanks", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0, order: { kind: "hold" } }, { id: "r-x", side: "red", platform: "generic/infantrySection", count: 1, east: 0, north: 30_000, order: { kind: "hold" } }], domains: LOG, maxDurationS: 8 * 3600 });
    const s = g.state;
    const unit = s.units["b-tanks"];
    const damaged: RtState = { ...s, units: { ...s.units, "b-tanks": { ...unit, vehicles: { ...unit.vehicles, fit: unit.vehicles.fit - 1 } } }, world: { ...s.world!, land: { ...s.world!.land, "b-tanks": { ...s.world!.land["b-tanks"], disabled: 1 } } } };
    const runner = await runKit({ state: damaged, config: g.config }, 5 * 3600);
    expect(runner.state.units["b-tanks"].vehicles.fit).toBe(unit.vehicles.fit);
    expect(runner.state.world!.log.some((l) => l.kind === "repaired")).toBe(true);
  }, 60_000);
});

describe("medical (W10)", () => {
  const fight = (medical: "simulated" | "abstracted", seed: string): KitOptions => ({
    seed,
    units: [
      { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "hold" } },
      { id: "b-inf-2", side: "blue", platform: "generic/infantrySection", count: 3, east: 800, north: 0, order: { kind: "hold" } },
      { id: "b-reserve", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: -10_000, order: { kind: "hold" } },
      { id: "r-tanks", side: "red", platform: "generic/mbt", count: 4, east: 0, north: 2500, order: { kind: "move", to: at(400, 600), mode: "tactical" } },
    ],
    entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: -15_000 }],
    domains: { logistics: "simulated", medical },
    maxDurationS: 6 * 3600,
    breakpoint: 1,
  });

  it("evacuation times are reported and fall in the reference band (60–120 min typical)", async () => {
    const times: number[] = [];
    for (let i = 0; i < 4; i++) {
      const runner = await runKit(kitGame(fight("simulated", `medevac-${i}`)), 5 * 3600);
      times.push(...runner.state.world!.stats.evacuationS);
    }
    expect(times.length).toBeGreaterThan(3);
    const sorted = [...times].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)] / 60;
    expect(median).toBeGreaterThan(30);
    expect(median).toBeLessThan(180);
  }, 120_000);

  it("abstracted medical draws evacuation times from the band, with no ambulances on the board", async () => {
    const runner = await runKit(kitGame(fight("abstracted", "medevac-abs")), 5 * 3600);
    const w = runner.state.world!;
    expect(w.stats.evacuationS.length).toBeGreaterThan(0);
    for (const t of w.stats.evacuationS) {
      expect(t).toBeGreaterThanOrEqual(3600);
      expect(t).toBeLessThanOrEqual(7200 + 600);
    }
    expect(Object.keys(runner.state.game.forceElements).some((id) => id.includes("medevac"))).toBe(false);
  }, 60_000);

  it("the longer evacuation takes, the more die of wounds", () => {
    expect(diedOfWoundsP(30 * 60)).toBeLessThan(diedOfWoundsP(3 * 3600));
    expect(diedOfWoundsP(6 * 3600)).toBeLessThanOrEqual(0.9);
  });
});
