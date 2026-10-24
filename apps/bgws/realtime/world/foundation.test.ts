// The world layer's foundation (W0, W2, W14): the catalogue is complete and
// sane, a game with an empty world plays exactly as one without, munitions
// in flight cannot tunnel through a target, and world sensors see what the
// land engine cannot — a helicopter beyond 3 km, but not past the horizon.
import { describe, expect, it } from "vitest";
import { toGameStateFromPlaced } from "../../lib/forceBuilder";
import { HOUSE_V1 } from "../../rules/ruleset";
import { CATALOGUE, catalogueProblems, recordFor } from "../catalogue";
import { GENERIC_LAND_SNAPSHOTS } from "../catalogue/snapshots";
import { InThreadRunner } from "../engine/runnerHandle";
import { EventFingerprint } from "../engine/fingerprint";
import { archetypeFor } from "../model/archetypes";
import { allPlatforms, toPlatform } from "../model/platform";
import { closestApproachM, horizonM, scatter } from "./geo";
import { at, kitGame, runKit } from "./testkit";
import { WorldCtx } from "./context";
import { launch } from "./munitions";
import { stepWorld } from "./tickHook";

describe("the synthetic catalogue (W14)", () => {
  it("has every archetype the plan lists, complete and in sane ranges", () => {
    expect(catalogueProblems()).toEqual([]);
    for (const id of [
      "generic/mbt", "generic/ifv", "generic/infantrySection", "generic/atgmTeam", "generic/mortar120", "generic/sph155", "generic/mlrs",
      "generic/shorad", "generic/samMedium", "generic/samLong", "generic/ewJammer", "generic/droneIsrTeam", "generic/fpvTeamRf",
      "generic/fpvTeamFibre", "generic/loiterTeam", "generic/owaDrone", "generic/male", "generic/interceptorTeam", "generic/ugv",
      "generic/fighter4", "generic/fighter5", "generic/attackHelo", "generic/transportHelo", "generic/tanker", "generic/aew", "generic/mpa",
      "generic/destroyer", "generic/frigate", "generic/corvette", "generic/ssk", "generic/ssn", "generic/usv", "generic/coastalAsm",
      "generic/satEo", "generic/satSar", "generic/satcomGeo", "generic/gnss", "generic/depot", "generic/convoyTruck", "generic/railhead",
      "generic/port", "generic/airbase", "generic/cp-company", "generic/cp-battalion", "generic/cp-brigade", "generic/cp-division",
    ]) {
      expect(() => recordFor(id), id).not.toThrow();
    }
  });

  it("labels every record synthetic and gives it a rationale", () => {
    for (const r of CATALOGUE) {
      expect(r.provenance, r.id).toBe("synthetic");
      expect(r.rationale.length, r.id).toBeGreaterThan(10);
    }
  });

  it("makes every land record placeable as a force-list platform", () => {
    const placed = Object.keys(GENERIC_LAND_SNAPSHOTS).map((platform, i) => ({
      id: `u${i}`, label: platform, side: (i % 2 ? "red" : "blue") as "blue" | "red", platform, platformCount: 2, troopQuality: "regular" as const,
      position: at(i * 50, 0),
    }));
    const game = toGameStateFromPlaced(placed, HOUSE_V1);
    for (const fe of Object.values(game.forceElements)) {
      expect(fe.combatStrength, fe.label).toBeGreaterThan(0);
      expect(fe.capabilities.length, fe.label).toBeGreaterThan(0);
    }
  });

  it("matches sourced platforms to archetypes by their data", () => {
    expect(archetypeFor("var_11_default")).toBe("generic/mbt");
    expect(archetypeFor("declared/mortar_section")).toBe("generic/mortar120");
    expect(archetypeFor("infantry/rifle_infantry")).toBe("generic/infantrySection");
    expect(archetypeFor("generic/fpvTeamRf")).toBe("generic/fpvTeamRf");
    expect(archetypeFor("var_11_default", { commandRating: 2 })).toBe("generic/cp-company");
  });
});

describe("the world layer leaves the land game alone", () => {
  it("a game with an empty world (clear daylight, nothing else) plays exactly as one without", async () => {
    const units = [
      { id: "b1", side: "blue" as const, platform: "var_11_default", count: 4, east: -300, north: 0 },
      { id: "b2", side: "blue" as const, platform: "infantry/rifle_infantry", count: 3, east: 300, north: 0 },
      { id: "r1", side: "red" as const, platform: "var_11_default", count: 4, east: 0, north: 1500 },
    ];
    const target = at(0, 1500);
    const order = { kind: "move" as const, to: target, mode: "tactical" as const };
    const withWorld = kitGame({ units: units.map((u) => (u.side === "blue" ? { ...u, order } : u)), seed: "same", ground: "generated" });
    const without = { state: { ...withWorld.state, world: undefined }, config: { ...withWorld.config } };
    const fp = async (g: typeof withWorld, seed: string) => {
      const f = new EventFingerprint();
      const { createRng } = await import("../../rules/dice");
      const runner = new InThreadRunner(g.state, { ...g.config, rng: createRng(seed) }, { onEntry: (e) => e.type !== "world" && f.add(e) });
      await runner.advance(600);
      return { fp: f.value.fingerprint, fe: JSON.stringify(runner.state.game.forceElements) };
    };
    expect(await fp(withWorld, "s1")).toEqual(await fp(without, "s1"));
  });
});

describe("munitions in flight (W2)", () => {
  it("finds a hit by closest approach inside one second, at any speed", () => {
    const a0 = { at: at(0, 0), altM: 1000 };
    const a1 = { at: at(1500, 0), altM: 1000 };
    // The target crosses the interceptor's path halfway through the second.
    const b0 = { at: at(750, 200), altM: 1000 };
    const b1 = { at: at(750, -200), altM: 1000 };
    expect(closestApproachM(a0, a1, b0, b1)).toBeLessThan(5);
    // A straight-line check at the two ends would have missed it by 750 m.
    expect(closestApproachM(a0, a0, b0, b0)).toBeGreaterThan(700);
  });

  it("scatters unguided rounds so half land inside their CEP", () => {
    let roll = 0;
    const rng = (n: number) => (roll = (roll * 1103515245 + 12345) >>> 0) % n;
    const centre = at(0, 0);
    const inside = Array.from({ length: 4000 }, () => scatter(centre, 100, rng)).filter((p) => {
      const dx = (p.lng - centre.lng) * 111_320 * Math.cos((centre.lat * Math.PI) / 180);
      const dy = (p.lat - centre.lat) * 111_320;
      return Math.hypot(dx, dy) <= 100;
    }).length;
    expect(inside / 4000).toBeGreaterThan(0.45);
    expect(inside / 4000).toBeLessThan(0.55);
  });

  it("a mortar's bombs are entities in flight, then land", async () => {
    const game = kitGame({
      units: [
        { id: "mortar", side: "blue", platform: "generic/mortar120", east: 0, north: 0 },
        { id: "target", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 3000 },
      ],
      domains: { fires: "simulated" },
    });
    const view = landView(game);
    const c = new WorldCtx(game.state.world!, view);
    const shell = launch(c, { launcherId: "mortar", owner: "blue", weaponId: "mortar120", from: at(0, 0), fromAltM: 2, aimAt: at(0, 3000) });
    expect(shell.domain).toBe("munition");
    expect(shell.munition!.impactAt).toBeGreaterThan(10);
    const state = { ...game.state, world: c.world };
    const runner = new InThreadRunner(state, game.config);
    await runner.advance(5);
    const inFlight = runner.state.world!.entities[shell.id];
    expect(inFlight.status).toBe("active");
    expect(inFlight.altM).toBeGreaterThan(100);
    await runner.advance(20);
    expect(runner.state.world!.entities[shell.id]?.status ?? "expended").toBe("expended");
    expect(runner.state.world!.stats.munitions.mortar120.fired).toBe(1);
  });
});

describe("world sensors (W2)", () => {
  it("a ground radar sees a helicopter beyond 3 km, and not a vehicle on the ground", async () => {
    const game = kitGame({
      units: [
        { id: "radar", side: "blue", platform: "generic/shorad", east: 0, north: 0 },
        { id: "tank", side: "red", platform: "generic/mbt", east: 0, north: 2500 },
      ],
      entities: [{ id: "helo", owner: "red", archetype: "generic/attackHelo", east: 0, north: 9000, altM: 300 }],
    });
    const runner = await runKit(game, 30);
    const tracks = runner.state.world!.tracks.blue;
    expect(tracks.helo).toBeDefined();
    expect(tracks.helo.by).toBe("radar");
    expect(tracks.tank).toBeUndefined();
  });

  it("cannot see a low drone past the radar horizon", async () => {
    expect(horizonM(6, 50) / 1000).toBeGreaterThan(38);
    expect(horizonM(6, 50) / 1000).toBeLessThan(40);
    const game = kitGame({
      units: [{ id: "radar", side: "blue", platform: "generic/samMedium", east: 0, north: 0 }],
      entities: [
        { id: "low", owner: "red", archetype: "generic/male", east: 0, north: 45_000, altM: 50 },
        { id: "high", owner: "red", archetype: "generic/male", east: 0, north: 45_000, altM: 6000 },
      ],
    });
    const runner = await runKit(game, 60);
    expect(runner.state.world!.tracks.blue.high).toBeDefined();
    expect(runner.state.world!.tracks.blue.low).toBeUndefined();
  });
});

describe("the platform view (W2 adapter)", () => {
  it("describes a land unit and an entity the same way", () => {
    const game = kitGame({
      units: [{ id: "ifv", side: "blue", platform: "generic/ifv", count: 3, east: 0, north: 0 }],
      entities: [{ id: "ddg", owner: "blue", archetype: "generic/destroyer", east: 0, north: -20_000 }],
    });
    const ifv = toPlatform(game.state, "ifv")!;
    expect(ifv.domain).toBe("land");
    expect(ifv.body.fit).toBe(3);
    expect(ifv.carried.dismounts).toBe(3);
    expect(ifv.stores.ammo.cannon30).toBeGreaterThan(0);
    const ddg = toPlatform(game.state, "ddg")!;
    expect(ddg.domain).toBe("surface");
    expect(ddg.weapons.find((w) => w.id === "samLong")?.rounds).toBe(16);
    expect(allPlatforms(game.state).map((p) => p.id)).toEqual(["ifv", "ddg"]);
  });
});

/** A LandView over a kit game's start, for setting things up outside a tick. */
function landView(game: ReturnType<typeof kitGame>) {
  const state = game.state;
  return {
    time: state.time,
    config: game.config,
    rng: game.config.rng,
    game: state.game,
    units: { ...state.units },
    setFe: () => {},
    setUnit: () => {},
    order: () => {},
    emit: () => {},
    radio: () => {},
    known: () => "none" as const,
    prev: state,
  };
}
void stepWorld;
