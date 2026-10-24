// M8, joint all-domain (W9, W11): satellite imagery reaching a side's picture
// late; a provider cutting it off slowing the kill chain; cyber on logistics
// delaying resupply; missile warning cueing defences; SATCOM jamming dropping
// a drone's feed; and the kill web's speed measured — data link against
// voice, minutes against hours — with every chain's steps in the AAR.
import { describe, expect, it } from "vitest";
import { launch } from "./munitions";
import { withWorldCtx } from "./standalone";
import { at, kitGame, runKit, type KitOptions } from "./testkit";

function killWeb(seed = "kw"): KitOptions {
  return {
    seed,
    units: [
      { id: "b-bn", side: "blue", platform: "generic/cp-battalion", count: 3, east: 0, north: -8000, commandRating: 3 },
      { id: "b-coy", side: "blue", platform: "generic/cp-company", count: 2, east: 0, north: -3000, commandRating: 2 },
      { id: "b-isr", side: "blue", platform: "generic/droneIsrTeam", count: 1, east: 0, north: 1500 },
      { id: "b-guns", side: "blue", platform: "generic/sph155", count: 3, east: 0, north: -12_000 },
      { id: "r-guns", side: "red", platform: "generic/sph155", count: 3, east: 1000, north: 6000, order: { kind: "hold" } },
      { id: "r-hq", side: "red", platform: "generic/cp-company", count: 2, east: -1500, north: 5000, commandRating: 2 },
    ],
    hqs: [
      { id: "blue-bn", side: "blue", echelon: "battalion", postId: "b-bn", subordinates: ["blue-coy", "b-guns"], seat: "heuristic" },
      { id: "blue-coy", side: "blue", echelon: "company", postId: "b-coy", parent: "blue-bn", subordinates: ["b-isr"], seat: "heuristic" },
    ],
    domains: { drones: "simulated", fires: "simulated", ew: "simulated" },
    comms: "radio",
    breakpoint: 1,
    maxDurationS: 6 * 3600,
  };
}

/** When blue first fired on anything: kill-web chains and direct calls alike. */
async function firstStrike(network: "voice" | "datalink", seed: string): Promise<{ at: number; chains: number }> {
  const g = kitGame(killWeb(seed));
  g.state.world!.network = { blue: network, red: "datalink" };
  const r = await runKit(g, 3 * 3600);
  const w = r.state.world!;
  const fired = w.missions.filter((m) => m.side === "blue" && m.times.fired != null).map((m) => m.times.fired! - (m.times.found ?? m.times.requested));
  return { at: Math.min(...fired, Infinity), chains: w.chains.filter((k) => k.side === "blue" && k.engaged != null).length };
}

describe("the kill web (W11)", () => {
  it("over a data link the chain takes minutes; up and down the voice nets, many times longer [ref R3, P2]", async () => {
    let dl = 0;
    let voice = 0;
    for (let i = 0; i < 3; i++) {
      dl += (await firstStrike("datalink", `kw-${i}`)).at;
      voice += (await firstStrike("voice", `kw-${i}`)).at;
    }
    expect(dl / 3).toBeLessThan(15 * 60);
    expect(voice / dl).toBeGreaterThan(4);
  }, 120_000);

  it("the AAR shows every kill chain step by step", async () => {
    const g = kitGame(killWeb());
    const r = await runKit(g, 3 * 3600);
    const w = r.state.world!;
    const done = w.chains.filter((k) => k.assessed != null);
    expect(done.length).toBeGreaterThan(0);
    for (const k of done) {
      expect(k.found).toBeLessThanOrEqual(k.fixed!);
      expect(k.fixed!).toBeLessThanOrEqual(k.targeted!);
      expect(k.targeted!).toBeLessThanOrEqual(k.engaged!);
      expect(k.engaged!).toBeLessThan(k.assessed!);
    }
    expect(r.log.some((e) => e.type === "world" && e.item.kind === "assessed" && /found→fix .*engage→assess/.test(e.item.text))).toBe(true);
    expect(w.stats.killChainS?.blue.length ?? 0).toBeGreaterThan(0);
  }, 60_000);
});

describe("space (W9)", () => {
  const imageryGame = () =>
    kitGame({
      seed: "imagery",
      units: [
        { id: "b-hq", side: "blue", platform: "generic/cp-brigade", count: 5, east: 0, north: -40_000, commandRating: 3 },
        { id: "r-x", side: "red", platform: "generic/infantrySection", count: 3, east: 30_000, north: 50_000, order: { kind: "hold" } },
      ],
      hqs: [{ id: "blue-bde", side: "blue", echelon: "brigade", postId: "b-hq", subordinates: [], seat: "heuristic" }],
      entities: [
        { id: "b-sat", owner: "blue", archetype: "generic/satEo", east: 0, north: 0, provider: "commercial" },
        { id: "b-sat-2", owner: "blue", archetype: "generic/satSar", east: 0, north: 0, provider: "commercial" },
        // A red depot blue knows nothing of, and a submarine that could strike it.
        { id: "r-depot", owner: "red", archetype: "generic/depot", east: 0, north: 60_000 },
        { id: "b-ssn", owner: "blue", archetype: "generic/ssn", east: 0, north: -150_000, altM: -100 },
      ],
      // Blue's collection plan: the road hub where red is thought to keep its stocks.
      nais: [{ side: "blue", label: "NAI 1: the rear road hub", at: at(2000, 58_000), priority: 5 }],
      domains: { space: "simulated", naval: "simulated" },
      env: { cloudCover: 0.2 },
      breakpoint: 1,
      maxDurationS: 12 * 3600,
    });

  it("the other side's picture updates on satellite passes, after a processing latency", async () => {
    const r = await runKit(imageryGame(), 8 * 3600);
    const w = r.state.world!;
    const lat = w.stats.imageryLatencyS ?? [];
    expect(lat.length).toBeGreaterThan(0);
    for (const s of lat) {
      expect(s).toBeGreaterThanOrEqual(1800);
      expect(s).toBeLessThanOrEqual(5400);
    }
    expect(r.log.some((e) => e.type === "world" && e.item.kind === "imagery" && e.item.ids?.includes("r-depot"))).toBe(true);
  }, 120_000);

  it("cutting the commercial imagery provider slows the kill chain: no strike on what only imagery could find", async () => {
    const struck = async (cut: boolean) => {
      const g = imageryGame();
      if (cut) g.state.world!.outages["imagery:blue"] = Number.MAX_SAFE_INTEGER;
      const r = await runKit(g, 8 * 3600);
      return r.state.world!.chains.filter((k) => k.side === "blue" && k.truthId === "r-depot" && k.engaged != null).length;
    };
    expect(await struck(false)).toBeGreaterThan(0);
    expect(await struck(true)).toBe(0);
  }, 120_000);

  it("missile warning puts a ballistic launch in the defender's picture at once", async () => {
    const g = kitGame({
      seed: "mw",
      units: [{ id: "b-x", side: "blue", platform: "generic/infantrySection", count: 1, east: 0, north: 0, order: { kind: "hold" } }],
      entities: [{ id: "b-mw", owner: "blue", archetype: "generic/missileWarning", east: 0, north: 0 }],
      domains: { space: "simulated" },
      breakpoint: 1,
    });
    const fired = withWorldCtx(g.state, g.config, (c) => void launch(c, { launcherId: "r-x", owner: "red", weaponId: "ballistic", from: at(0, 300_000), fromAltM: 10, aimAt: at(0, 0) }));
    const r = await runKit({ state: fired.state, config: g.config }, 5);
    expect(Object.values(r.state.world!.tracks.blue).some((t) => t.thought === "ballistic missile")).toBe(true);
    expect(r.state.world!.log.some((l) => l.kind === "missileWarning")).toBe(true);
  });

  it("jamming military SATCOM drops a satellite-linked drone's feed from the HQ's picture", async () => {
    const feed = async (jam: boolean) => {
      const g = kitGame({
        seed: "satcom",
        units: [
          { id: "b-hq", side: "blue", platform: "generic/cp-battalion", count: 3, east: 0, north: -10_000, commandRating: 3 },
          { id: "r-x", side: "red", platform: "generic/mbt", count: 4, east: 0, north: 15_000, order: { kind: "hold" } },
        ],
        hqs: [{ id: "blue-bn", side: "blue", echelon: "battalion", postId: "b-hq", subordinates: ["b-male"], seat: "heuristic" }],
        entities: [
          { id: "b-male", owner: "blue", archetype: "generic/male", east: 0, north: 13_000, altM: 4000, task: { kind: "orbit", at: at(0, 14_000), radiusM: 2000, altM: 4000 } },
          { id: "b-geo", owner: "blue", archetype: "generic/satcomGeo", east: 0, north: 0 },
        ],
        domains: { space: "simulated", drones: "simulated" },
        events: jam ? [{ at: 0, kind: "jamSatcom", side: "blue" }] : [],
        breakpoint: 1,
      });
      const r = await runKit(g, 600);
      return Object.keys(r.state.world!.hqs["blue-bn"].cop).includes("r-x");
    };
    expect(await feed(false)).toBe(true);
    expect(await feed(true)).toBe(false);
  }, 60_000);
});

describe("cyber (W9)", () => {
  it("a cyber effect on the logistics system delays resupply", async () => {
    const firstConvoy = async (hacked: boolean) => {
      const g = kitGame({
        seed: "cyber",
        units: [
          { id: "b-inf", side: "blue", platform: "generic/infantrySection", count: 3, east: 0, north: 0, order: { kind: "hold" } },
          { id: "r-hq", side: "red", platform: "generic/cp-brigade", count: 5, east: 0, north: 40_000, commandRating: 3 },
        ],
        hqs: [{ id: "red-bde", side: "red", echelon: "brigade", postId: "r-hq", subordinates: [], seat: "heuristic" }],
        entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: -15_000 }, { id: "r-cyber", owner: "red", archetype: "generic/cyberCell", east: 0, north: 60_000 }],
        cyber: hacked ? [{ id: "acc-1", side: "red", target: "logistics", system: "logistics", readiness: 0, pEffect: 1, durationS: 4 * 3600, detectionRisk: 0, window: { from: 0, until: 3600 } }] : [],
        domains: { logistics: "simulated", cyber: "simulated" },
        breakpoint: 1,
        maxDurationS: 8 * 3600,
      });
      g.state.world!.land["b-inf"] = { ...g.state.world!.land["b-inf"], stores: { ...g.state.world!.land["b-inf"].stores, ammo: { smallArms: 10 } } };
      const r = await runKit(g, 6 * 3600);
      const sent = r.log.find((e) => e.type === "world" && e.item.kind === "convoy");
      return sent && sent.type === "world" ? sent.item.time : Infinity;
    };
    const normal = await firstConvoy(false);
    const hacked = await firstConvoy(true);
    expect(normal).toBeLessThan(1800);
    expect(hacked).toBeGreaterThan(normal + 3600);
  }, 60_000);
});
