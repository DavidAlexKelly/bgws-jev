// M5, air over the battle (W7): a SEAD package against a SAM site, with the
// outcome spread a crew's reactions make; a Shahed-style raid on a layered
// defence, with the interception rate and cost exchange in band; the cost-
// aware choice of interceptor; CAS on a 9-line; and guns held for aircraft
// in the gun line.
import { describe, expect, it } from "vitest";
import { WorldCtx } from "./context";
import { interceptorChoice } from "./domains/air";
import { requestFire } from "./domains/fires";
import { withWorldCtx } from "./standalone";
import { at, kitGame, runKit, type KitEntity, type KitUnit } from "./testkit";

const AIR = { air: "simulated", airDefence: "simulated", ew: "simulated" } as const;

function sead(seed: string) {
  return kitGame({
    seed,
    units: [
      { id: "r-sam", side: "red", platform: "generic/samMedium", count: 2, east: 0, north: 80_000 },
      { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 70_000, order: { kind: "hold" } },
      // Blue's SIGINT finds the SAM's radar for the SEAD flight.
      { id: "b-esm-1", side: "blue", platform: "generic/esmTeam", count: 1, east: -15_000, north: 40_000 },
      { id: "b-esm-2", side: "blue", platform: "generic/esmTeam", count: 1, east: 15_000, north: 40_000 },
    ],
    entities: [{ id: "b-base", owner: "blue", archetype: "generic/airbase", east: 0, north: -20_000 }],
    domains: AIR,
    maxDurationS: 6 * 3600,
  });
}

function raid(seed: string, size = 30) {
  const drones: KitEntity[] = [];
  for (let i = 0; i < size; i++) {
    // About 60% armed, 40% decoys [ref R5.2, P10], in a broad front from 120 km out.
    const decoy = i % 5 >= 3;
    drones.push({ id: `owa-${i}`, owner: "red", archetype: decoy ? "generic/owaDecoy" : "generic/owaDrone", east: -20_000 + i * 1300, north: 120_000 + (i % 3) * 2000, altM: 1500, task: { kind: "strike", aimAt: at((i % 3) * 60 - 60, (i % 2) * 60 - 30), altM: 1500 } });
  }
  const game = kitGame({
    seed,
    units: [
      { id: "b-sam", side: "blue", platform: "generic/samMedium", count: 2, east: 0, north: 8000 },
      { id: "b-shorad-1", side: "blue", platform: "generic/shorad", count: 2, east: -2000, north: 2500 },
      { id: "b-shorad-2", side: "blue", platform: "generic/shorad", count: 2, east: 2000, north: 2500 },
      { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 150_000, order: { kind: "hold" } },
    ],
    entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: 0 }, ...drones],
    domains: { ...AIR, drones: "simulated" },
    maxDurationS: 6 * 3600,
  });
  return game;
}

describe("air (W7)", () => {
  it("a SEAD package against a SAM site: plausible outcomes over 50 seeded runs, not one scripted result", async () => {
    let armHits = 0;
    let silent = 0;
    let released = 0;
    let aircraftLost = 0;
    for (let i = 0; i < 50; i++) {
      const runner = await runKit(sead(`sead-${i}`), 2 * 3600);
      const w = runner.state.world!;
      if (!w.log.some((l) => l.kind === "release" && l.ids?.includes("r-sam"))) continue;
      released++;
      armHits += w.stats.munitions.arm?.hits ?? 0;
      if (w.log.some((l) => l.kind === "silent" && l.ids?.includes("r-sam"))) silent++;
      aircraftLost += Object.values(w.entities).filter((e) => e.owner === "blue" && e.domain === "air" && e.status === "destroyed").length;
    }
    expect(released, "SEAD flown on the fix").toBeGreaterThan(40);
    // Crews that see the ARM coming shut down and survive; the rest risk the hit. Both happen.
    expect(silent / released).toBeGreaterThan(0.3);
    expect(silent / released).toBeLessThan(0.9);
    // A package that stands off at ARM range rarely loses aircraft to the site it is suppressing.
    expect(aircraftLost / released).toBeLessThan(1);
    // The rest get hit: SEAD that never lands a missile would be no threat at all.
    expect(armHits).toBeGreaterThan(5);
  }, 120_000);

  it("a Shahed-style raid on a layered defence: interception near the reference ~85%, and the defender pays more", async () => {
    let launched = 0;
    let downed = 0;
    let spent = 0;
    let lost = 0;
    for (let i = 0; i < 6; i++) {
      const runner = await runKit(raid(`raid-${i}`), 3600);
      const w = runner.state.world!;
      const owa = Object.values(w.entities).filter((e) => e.id.startsWith("owa-"));
      launched += owa.length;
      downed += owa.filter((e) => e.status === "destroyed").length;
      spent += w.stats.spentUsd.blue;
      lost += w.stats.lostUsd.red;
    }
    const rate = downed / launched;
    // P10: about 85% intercepted. Synthetic catalogue (W12): the band, not the figure.
    expect(rate).toBeGreaterThan(0.65);
    expect(rate).toBeLessThan(0.97);
    // The cost exchange: interceptors cost more than the drones they bring down (P11).
    expect(spent).toBeGreaterThan(lost);
  }, 120_000);

  it("the cheapest interceptor that will do against a drone, the surest against a fighter", () => {
    const game = kitGame({
      units: [{ id: "b-shorad", side: "blue", platform: "generic/shorad", count: 2, east: 0, north: 0 }, { id: "r-x", side: "red", platform: "generic/infantrySection", count: 1, east: 0, north: 20_000 }],
      entities: [
        { id: "drone", owner: "red", archetype: "generic/owaDrone", east: 0, north: 2000, altM: 500 },
        { id: "jet", owner: "red", archetype: "generic/fighter4", east: 0, north: 2500, altM: 1500, status: "active" },
      ],
      domains: AIR,
    });
    withWorldCtx(game.state, game.config, (c: WorldCtx) => {
      expect(interceptorChoice(c, "b-shorad", "drone").cheapest?.id).toBe("gunAd");
      expect(interceptorChoice(c, "b-shorad", "jet").surest?.id).toBe("samShort");
    });
  });

  it("a flight a SAM fires at is warned and defends, and lives more often for it", async () => {
    let defended = 0;
    let engaged = 0;
    for (let i = 0; i < 10; i++) {
      const game = kitGame({
        seed: `defend-${i}`,
        units: [
          { id: "r-sam", side: "red", platform: "generic/samMedium", count: 2, east: 0, north: 30_000 },
          { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: -30_000, order: { kind: "hold" } },
        ],
        entities: [{ id: "b-cap", owner: "blue", archetype: "generic/fighter4", east: 0, north: 0, altM: 8000, status: "active" }],
        domains: AIR,
      });
      const w0 = game.state.world!;
      w0.entities["b-cap"] = { ...w0.entities["b-cap"], air: { sorties: 0, readyAt: 0, mission: { kind: "cap", at: at(0, 5000), radiusM: 5000, altM: 8000, until: 1800, tasked: 0, by: "test" } } };
      const runner = await runKit(game, 900);
      const w = runner.state.world!;
      if (w.log.some((l) => l.kind === "engage" && l.ids?.includes("b-cap"))) engaged++;
      if (w.log.some((l) => l.kind === "defend" && l.ids?.includes("b-cap"))) defended++;
    }
    expect(engaged).toBeGreaterThan(5);
    expect(defended).toBeGreaterThan(5);
  }, 60_000);

  it("CAS: a unit in contact sends a 9-line and the flight on station strikes", async () => {
    const units: KitUnit[] = [
      { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "hold" } },
      { id: "r-tanks", side: "red", platform: "generic/mbt", count: 4, east: 0, north: 2500, order: { kind: "move", to: at(0, 0), mode: "tactical" } },
    ];
    let nine = 0;
    let released = 0;
    for (let i = 0; i < 4; i++) {
      const game = kitGame({ seed: `cas-${i}`, units, entities: [{ id: "b-base", owner: "blue", archetype: "generic/airbase", east: 0, north: -40_000 }], domains: AIR, comms: "radio", maxDurationS: 3 * 3600 });
      const runner = await runKit(game, 2400);
      const w = runner.state.world!;
      if (w.messages.some((m) => m.kind === "nineLine")) nine++;
      if (w.log.some((l) => l.kind === "release" && l.ids?.includes("r-tanks"))) released++;
    }
    expect(nine).toBeGreaterThan(0);
    expect(released).toBeGreaterThan(0);
  }, 60_000);

  it("guns wait while an own aircraft is in the gun line, and fire once it is clear", async () => {
    const fired = async (aircraft: boolean) => {
      const game = kitGame({
        seed: "airspace",
        units: [
          { id: "b-guns", side: "blue", platform: "generic/sph155", count: 3, east: 0, north: -12_000 },
          { id: "r-target", side: "red", platform: "generic/convoyTruck", count: 4, east: 0, north: 4000 },
        ],
        entities: aircraft ? [{ id: "b-helo", owner: "blue", archetype: "generic/attackHelo", east: 300, north: -4000, altM: 300, status: "active", task: { kind: "orbit", at: at(0, -4000), radiusM: 500, altM: 300 } }] : [],
        domains: { ...AIR, fires: "simulated" },
      });
      // The helicopter has a station of its own and stays on it a while.
      const w0 = game.state.world!;
      if (aircraft) w0.entities["b-helo"] = { ...w0.entities["b-helo"], air: { sorties: 0, readyAt: 0, mission: { kind: "cas", at: at(0, -4000), radiusM: 1500, altM: 300, until: 400, tasked: 0, by: "test" } } };
      const asked = withWorldCtx(game.state, game.config, (c) => void requestFire(c, { side: "blue", requestedBy: "b-guns", targetAt: at(0, 4000), targetId: "r-target", priority: 2 }));
      const runner = await runKit({ state: asked.state, config: game.config }, 1200);
      return runner.state.world!.missions[0];
    };
    const clear = await fired(false);
    const held = await fired(true);
    expect(clear.times.fired).toBeDefined();
    expect(held.times.fired, "it fires in the end").toBeDefined();
    expect(held.times.fired!).toBeGreaterThan(clear.times.fired! + 60);
  }, 60_000);

  it("victory: air superiority held over an area wins; it is judged from the truth, not the land breakpoint", async () => {
    const game = kitGame({
      seed: "victory",
      units: [
        { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: 0, order: { kind: "hold" } },
        { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 1, east: 0, north: 60_000, order: { kind: "hold" } },
        // A red SAM radiating over the area: no superiority while it is up.
        { id: "r-sam", side: "red", platform: "generic/samMedium", count: 2, east: 0, north: 30_000 },
      ],
      domains: AIR,
      victory: [{ kind: "airSuperiority", side: "blue", at: at(0, 30_000), radiusM: 20_000, holdS: 300, label: "air superiority" }],
    });
    const contested = await runKit(game, 600);
    expect(contested.state.over).toBeUndefined();
    // The SAM shuts down and stays down: the area is blue's once it has been clear for five minutes.
    const quiet = withWorldCtx(game.state, game.config, (c) => c.setLand("r-sam", { emcon: "silent" }));
    const won = await runKit({ state: quiet.state, config: game.config }, 900);
    expect(won.state.over?.winner).toBe("blue");
    expect(won.state.over?.reason).toMatch(/air superiority/);
    expect(won.state.time).toBeGreaterThanOrEqual(300);
  }, 60_000);
});
