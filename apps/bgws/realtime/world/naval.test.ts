// M7, the littoral (W8): a Red Sea-style defence of a task group, with
// magazines that run down and are only refilled in port; a Black Sea-style
// strike that drives a fleet out of its port; salvo outcomes that vary;
// submarines that hear and are heard; neutral shipping that ROE protects
// and that stops sailing when ships are hit; the shore as water's edge.
import { describe, expect, it } from "vitest";
import { launch } from "./munitions";
import { depthAt, isWater } from "./sea";
import { withWorldCtx } from "./standalone";
import { at, kitGame, runKit, type KitEntity } from "./testkit";

const SEA = { naval: "simulated", air: "simulated", airDefence: "simulated", drones: "simulated" } as const;

function redSea(seed: string, drones = 30) {
  const raid: KitEntity[] = [];
  for (let i = 0; i < drones; i++) raid.push({ id: `owa-${i}`, owner: "red", archetype: i % 4 === 3 ? "generic/owaDecoy" : "generic/owaDrone", east: 140_000 + i * 500, north: -20_000 + i * 1500, altM: 500, task: { kind: "strike", targetId: i % 2 ? "b-ddg" : "b-ffg", aimAt: at(0, 0), altM: 500 } });
  for (let i = 0; i < 4; i++) raid.push({ id: `usv-${i}`, owner: "red", archetype: "generic/usv", east: 60_000, north: -5000 + i * 3000, task: { kind: "strike", targetId: "b-ffg", aimAt: at(0, 0) } });
  return kitGame({
    seed,
    units: [
      { id: "r-asm", side: "red", platform: "generic/coastalAsm", count: 2, east: 150_000, north: 0, order: { kind: "hold" } },
      { id: "r-asm-2", side: "red", platform: "generic/coastalAsm", count: 2, east: 150_000, north: 30_000, order: { kind: "hold" } },
    ],
    entities: [
      { id: "b-ddg", owner: "blue", archetype: "generic/destroyer", east: 0, north: 0, task: { kind: "station", at: at(0, 0), radiusM: 10_000 } },
      { id: "b-ffg", owner: "blue", archetype: "generic/frigate", east: 5000, north: 3000, task: { kind: "station", at: at(5000, 3000), radiusM: 10_000 } },
      { id: "b-port", owner: "blue", archetype: "generic/port", east: -150_000, north: 0 },
      // Red's eyes on the task group: a drone that cues the coastal batteries.
      { id: "r-uav", owner: "red", archetype: "generic/male", east: 30_000, north: 0, altM: 5000, task: { kind: "orbit", at: at(20_000, 0), radiusM: 5000, altM: 5000 } },
      { id: "m-1", owner: "neutral", archetype: "generic/merchant", east: -100_000, north: 20_000, lane: [at(-100_000, 20_000), at(100_000, 20_000)] },
      ...raid,
    ],
    domains: SEA,
    breakpoint: 1,
    maxDurationS: 30 * 3600,
  });
}

describe("naval (W8)", () => {
  it("Red Sea: the task group's layers engage drones, missiles and USVs; interceptors are spent and the defender pays more", async () => {
    let spent = 0;
    let lost = 0;
    let engagements = 0;
    let shipsSunk = 0;
    for (let i = 0; i < 3; i++) {
      const r = await runKit(redSea(`redsea-${i}`), 3 * 3600);
      const w = r.state.world!;
      spent += w.stats.spentUsd.blue;
      lost += w.stats.lostUsd.red;
      engagements += r.log.filter((e) => e.type === "world" && e.item.kind === "engage").length;
      shipsSunk += ["b-ddg", "b-ffg"].filter((id) => w.entities[id].status === "destroyed").length;
      // The merchant is never engaged: ROE and closing to identify protect it.
      expect(w.stats.neutralHits ?? 0).toBe(0);
    }
    expect(engagements).toBeGreaterThan(30);
    // Interceptors cost more than the drones they bring down [ref R6.2]: the order of magnitude, several times over.
    expect(spent / Math.max(1, lost)).toBeGreaterThan(2);
    expect(shipsSunk).toBeLessThan(3);
  }, 120_000);

  it("a ship whose cells run low leaves station to reload in port, and comes back full", async () => {
    const r = await runKit(redSea("magazine", 40), 26 * 3600);
    const kinds = r.log.filter((e) => e.type === "world").map((e) => (e.type === "world" ? e.item.kind : ""));
    expect(kinds).toContain("magazineLow");
    expect(kinds).toContain("leaveStation");
    expect(kinds).toContain("reloaded");
  }, 120_000);

  it("Black Sea: a port struck hard is no longer tenable, and its fleet moves to the next port", async () => {
    const g = kitGame({
      seed: "sevastopol",
      units: [{ id: "b-x", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: -200_000, order: { kind: "hold" } }],
      entities: [
        { id: "r-port", owner: "red", archetype: "generic/port", east: 0, north: 0, label: "Red home port" },
        { id: "r-port-2", owner: "red", archetype: "generic/port", east: 250_000, north: 100_000, label: "Red far port" },
        { id: "r-ffg", owner: "red", archetype: "generic/frigate", east: 500, north: 300, parent: "r-port" },
        { id: "r-fsg", owner: "red", archetype: "generic/corvette", east: -400, north: 200, parent: "r-port" },
      ],
      domains: SEA,
      breakpoint: 1,
    });
    // A long-range strike in two dozen cruise missiles: the ships alongside defend the port too.
    const struck = withWorldCtx(g.state, g.config, (c) => {
      for (let i = 0; i < 24; i++) launch(c, { launcherId: "b-x", owner: "blue", weaponId: "cruise", from: at(0, -150_000), fromAltM: 50, targetId: "r-port", aimAt: at(0, 0) });
    });
    const r = await runKit({ state: struck.state, config: g.config }, 3600);
    const w = r.state.world!;
    expect(w.entities["r-port"].hp).toBeLessThan(15 * 0.5);
    expect(w.log.some((l) => l.kind === "fleetRelocates")).toBe(true);
    const moved = ["r-ffg", "r-fsg"].filter((id) => w.entities[id].status === "active" && w.entities[id].parent === "r-port-2");
    expect(moved.length).toBeGreaterThan(0);
  }, 60_000);

  it("salvo outcomes emerge: the same exchange goes differently with the dice", async () => {
    const outcomes = new Set<string>();
    for (let i = 0; i < 8; i++) {
      const g = kitGame({
        seed: `salvo-${i}`,
        units: [{ id: "b-x", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: -300_000, order: { kind: "hold" } }],
        entities: [
          { id: "b-ffg", owner: "blue", archetype: "generic/frigate", east: 0, north: 0 },
          { id: "r-fsg-1", owner: "red", archetype: "generic/corvette", east: 0, north: 120_000 },
          { id: "r-fsg-2", owner: "red", archetype: "generic/corvette", east: 3000, north: 120_000 },
          // Both sides have eyes on the other.
          { id: "r-uav", owner: "red", archetype: "generic/male", east: 0, north: 10_000, altM: 5000, task: { kind: "orbit", at: at(0, 15_000), radiusM: 3000, altM: 5000 } },
          { id: "b-mpa", owner: "blue", archetype: "generic/mpa", east: 0, north: 100_000, altM: 6000, status: "active", task: { kind: "orbit", at: at(0, 110_000), radiusM: 5000, altM: 6000 } },
        ],
        domains: SEA,
        breakpoint: 1,
      });
      // Weapons free on both sides: whatever they see, they shoot.
      for (const e of Object.values(g.state.world!.entities)) g.state.world!.entities[e.id] = { ...e, weaponControl: "free" };
      const r = await runKit(g, 1800);
      const w = r.state.world!;
      outcomes.add(["b-ffg", "r-fsg-1", "r-fsg-2"].map((id) => (w.entities[id].hp < (id === "b-ffg" ? 3 : 1.5) ? "hit" : "ok")).join(","));
    }
    expect(outcomes.size).toBeGreaterThan(1);
  }, 120_000);

  it("undersea: a frigate's sonar pings tell a quiet submarine it may be found; the submarine evades or attacks", async () => {
    const g = kitGame({
      seed: "asw",
      units: [{ id: "b-x", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: -300_000, order: { kind: "hold" } }],
      entities: [
        { id: "b-ffg", owner: "blue", archetype: "generic/frigate", east: 0, north: 0, task: { kind: "route", waypoints: [at(0, 60_000)] } },
        { id: "r-ssk", owner: "red", archetype: "generic/ssk", east: 2000, north: 25_000, altM: -80 },
      ],
      domains: SEA,
      breakpoint: 1,
    });
    const r = await runKit(g, 3600);
    const asked = r.log.filter((e) => e.type === "world" && e.item.kind === "question").map((e) => (e.type === "world" ? e.item.text : ""));
    expect(asked.some((t) => /counter|detected/i.test(t)) || asked.some((t) => /sonar/i.test(t))).toBe(true);
  }, 60_000);

  it("neutral shipping sails its lane, and stops sailing while ships are being hit", async () => {
    const lane = [at(0, 0), at(30_000, 0)];
    const g = kitGame({
      seed: "lanes",
      units: [{ id: "b-x", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: -300_000, order: { kind: "hold" } }],
      entities: [{ id: "m-1", owner: "neutral", archetype: "generic/merchant", east: 0, north: 0, lane }],
      domains: SEA,
      breakpoint: 1,
      maxDurationS: 12 * 3600,
    });
    const calm = await runKit(g, 8 * 3600);
    expect(calm.state.world!.stats.transits ?? 0).toBeGreaterThan(0);
    // A ship hit nearby: merchants stay in port.
    const hit = { ...g.state, world: { ...g.state.world!, memo: { ...g.state.world!.memo, "shipHit:any": 0 } } };
    const scared = await runKit({ state: hit, config: g.config }, 5 * 3600);
    expect(scared.state.world!.stats.transits ?? 0).toBe(0);
  }, 60_000);

  it("water: ships stay at sea, and the shelf deepens away from the shore", () => {
    const sea = { waters: [[at(0, 0), at(50_000, 0), at(50_000, 50_000), at(0, 50_000)]] };
    expect(isWater({ sea }, at(25_000, 25_000))).toBe(true);
    expect(isWater({ sea }, at(-5000, 25_000))).toBe(false);
    expect(depthAt({ sea }, at(25_000, 25_000))).toBeGreaterThan(depthAt({ sea }, at(1000, 25_000)));
    expect(isWater({}, at(-5000, 25_000))).toBe(true);
  });
});
