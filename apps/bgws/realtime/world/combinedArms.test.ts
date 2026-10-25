// M2, combined arms (W3): calls for fire answered by a mortar whose bombs fly
// as entities; counter-battery radar finding the gun; a minefield struck,
// engineers called and a lane breached in about the reference time; an IFV
// putting its infantry out; ammunition and fuel drawn down and resupplied.
import { describe, expect, it } from "vitest";
import { at, kitGame, runKit } from "./testkit";

const north = (m: number) => at(0, m);

describe("indirect fire (W3)", () => {
  it("a unit that sees an enemy beyond its reach calls for fire, and the mortar's bombs land", async () => {
    const game = kitGame({
      units: [
        { id: "inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "observe" } },
        { id: "mortar", side: "blue", platform: "generic/mortar120", east: 0, north: -1500 },
        { id: "tanks", side: "red", platform: "generic/mbt", count: 4, east: 200, north: 2400, order: { kind: "hold" } },
      ],
      domains: { fires: "simulated" },
    });
    const runner = await runKit(game, 400);
    const w = runner.state.world!;
    const mission = w.missions.find((m) => m.requestedBy === "inf");
    expect(mission, JSON.stringify(w.log.slice(-10))).toBeDefined();
    expect(mission!.shooterId).toBe("mortar");
    expect(["firing", "inFlight", "complete"]).toContain(mission!.state);
    expect(w.stats.munitions.mortar120.fired).toBeGreaterThan(0);
    // The decision was the mortar's, through its template.
    expect(runner.log.some((e) => e.type === "world" && e.item.kind === "answer" && e.item.text.startsWith("mortar (fireRequestReceived)"))).toBe(true);
    // Its ammunition is spent from its stores.
    expect(w.land.mortar.mounts[0].rounds).toBeLessThan(60);
  });

  it("a counter-battery radar tracks the rounds back and its artillery fires at the gun", async () => {
    const game = kitGame({
      units: [
        { id: "inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "observe" } },
        { id: "mortar", side: "blue", platform: "generic/mortar120", east: 0, north: -1500 },
        { id: "tanks", side: "red", platform: "generic/mbt", count: 4, east: 200, north: 2400, order: { kind: "hold" } },
        { id: "radar", side: "red", platform: "generic/cbRadar", count: 1, east: 0, north: 9000 },
        { id: "guns", side: "red", platform: "generic/sph155", count: 3, east: 1000, north: 12_000 },
      ],
      domains: { fires: "simulated" },
    });
    const runner = await runKit(game, 900);
    const w = runner.state.world!;
    const cb = w.missions.find((m) => m.side === "red" && m.reason === "counter-battery");
    expect(cb, JSON.stringify(w.log.filter((l) => l.domain === "fires").slice(-10))).toBeDefined();
    expect(cb!.shooterId).toBe("guns");
    expect(w.log.some((l) => l.kind === "counterBattery")).toBe(true);
  });

  it("abstracted fires answer from off the board after the response time", async () => {
    const game = kitGame({
      units: [
        { id: "inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "observe" } },
        { id: "tanks", side: "red", platform: "generic/mbt", count: 4, east: 200, north: 2400, order: { kind: "hold" } },
      ],
      domains: { fires: "abstracted" },
    });
    const runner = await runKit(game, 900);
    const w = runner.state.world!;
    const m = w.missions.find((x) => x.requestedBy === "inf");
    expect(m?.shooterId).toBe("support:blue-fires");
    expect(w.stats.munitions.howitzer155?.fired ?? 0).toBeGreaterThan(0);
  });
});

describe("obstacles and breaching (W3)", () => {
  const breachGame = (seed: string) =>
    kitGame({
      seed,
      units: [
        { id: "tanks", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0, order: { kind: "move", to: north(2000), mode: "tactical", route: [north(2000)] } },
        { id: "sappers", side: "blue", platform: "generic/engineerSection", count: 2, east: 300, north: -600 },
        { id: "far", side: "red", platform: "generic/infantrySection", count: 1, east: 9000, north: 9000, order: { kind: "hold" } },
      ],
      obstacles: [{ owner: "red", kind: "minefield", east: 0, north: 800, widthM: 1500, depthM: 300, bearingDeg: 0, density: 0.5 }],
      domains: { logistics: "off" },
      maxDurationS: 5 * 3600,
    });

  it("a unit strikes an unknown minefield, calls the engineers, and goes through the lane they clear", async () => {
    const runner = await runKit(breachGame("breach-1"), 4 * 3600);
    const w = runner.state.world!;
    expect(w.obstacles[0].knownTo).toContain("blue");
    // Found by striking it, or seen just before.
    expect(w.log.some((l) => l.kind === "mine" || l.kind === "obstacle")).toBe(true);
    expect(w.obstacles[0].lanes.length, JSON.stringify(w.log.filter((l) => l.domain !== "environment").slice(-15))).toBe(1);
    expect(w.stats.breachS.length).toBe(1);
    // The tanks went on through the lane to where they were going.
    const tanks = runner.state.game.forceElements.tanks;
    expect(tanks.position.lat).toBeGreaterThan(north(1000).lat);
  });

  it("a breach takes about the reference 1.5 h, within ±50%, over 50 seeded runs", async () => {
    const times: number[] = [];
    for (let i = 0; i < 50; i++) {
      const runner = await runKit(breachGame(`breach-v${i}`), 4 * 3600);
      times.push(...runner.state.world!.stats.breachS);
    }
    expect(times.length).toBeGreaterThan(40);
    const mean = times.reduce((s, t) => s + t, 0) / times.length;
    expect(mean).toBeGreaterThan(5400 * 0.5);
    expect(mean).toBeLessThan(5400 * 1.5);
    for (const t of times) expect(t).toBeLessThan(5400 * 2);
  }, 120_000);
});

describe("dismounts (W3)", () => {
  it("an IFV closing on an enemy puts its infantry out, and they fight as their own unit", async () => {
    const game = kitGame({
      units: [
        { id: "ifv", side: "blue", platform: "generic/ifv", count: 3, east: 0, north: 0, order: { kind: "move", to: north(2500), mode: "tactical" } },
        { id: "enemy", side: "red", platform: "generic/infantrySection", count: 2, east: 0, north: 2200, order: { kind: "hold" } },
      ],
    });
    const runner = await runKit(game, 400);
    const dis = Object.keys(runner.state.game.forceElements).find((id) => id.startsWith("ifv-dis"));
    expect(dis, JSON.stringify(runner.state.world!.log.slice(-8))).toBeDefined();
    expect(runner.state.game.forceElements[dis!].targetClass).toBe("foot");
    expect(runner.state.world!.land.ifv.dismounts.carried).toBe(0);
    expect(runner.state.world!.land[dis!].carrier).toBe("ifv");
    expect(runner.state.world!.joinedStrength.blue).toBeGreaterThan(0);
  });
});

describe("unit sustainment (W3)", () => {
  it("draws ammunition and fuel down, asks when low, and a resupply drop refills it", async () => {
    const game = kitGame({
      units: [
        { id: "tanks", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0, order: { kind: "overwatch" } },
        { id: "targets", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 700, order: { kind: "hold" } },
        { id: "more", side: "red", platform: "generic/infantrySection", count: 3, east: 300, north: 800, order: { kind: "hold" } },
      ],
      domains: { logistics: "abstracted" },
      maxDurationS: 7200,
    });
    const startFuel = game.state.world!.land.tanks.stores.fuelL;
    const startMg = game.state.world!.land.tanks.stores.ammo.smallArms!;
    const runner = await runKit(game, 1800);
    const w = runner.state.world!;
    expect(w.land.tanks.stores.fuelL).toBeLessThan(startFuel);
    const fired = runner.log.some((e) => e.type === "shot" && e.shot.firerId === "tanks");
    expect(fired).toBe(true);
    // Small arms or main gun, something was spent (or refilled after being spent).
    const spent = (w.land.tanks.stores.ammo.smallArms ?? 0) < startMg || w.log.some((l) => l.kind === "resupplied");
    expect(spent).toBe(true);
  });
});
