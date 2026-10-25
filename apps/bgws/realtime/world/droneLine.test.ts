// M4, the transparent battlefield (W5, W6): drone teams on both sides with
// ground units moving between them; the kill zone's reach and FPV
// effectiveness coming out of the entities, not a table; fibre against
// jamming; GNSS jamming cutting precision fire; an emitter on the air too
// long found and struck by an emergent chain; a jammer degrading its own
// side's radios.
import { describe, expect, it } from "vitest";
import { WorldCtx } from "./context";
import { distanceM } from "./geo";
import { launch } from "./munitions";
import { launchDrone } from "./domains/drones";
import { withWorldCtx } from "./standalone";
import { landRadioJamming } from "./jamming";
import { sendNet } from "./net";
import { at, kitGame, runKit, type KitUnit } from "./testkit";
import type { RtState } from "../engine/types";

const advance = (north: number) => ({ kind: "move" as const, to: at(0, north), mode: "tactical" as const });

function droneLine(seed: string, blueStrike = "generic/fpvTeamRf", redJammer = false) {
  const units: KitUnit[] = [
    // Red holds a drone line: ISR and FPV teams 6–9 km back from where blue starts.
    { id: "r-isr", side: "red", platform: "generic/droneIsrTeam", count: 1, east: -1500, north: 8000 },
    { id: "r-fpv-1", side: "red", platform: "generic/fpvTeamRf", count: 1, east: 0, north: 9000 },
    { id: "r-fpv-2", side: "red", platform: "generic/fpvTeamRf", count: 1, east: 2000, north: 8500 },
    { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 7000, order: { kind: "hold" } },
    // Blue advances across it.
    { id: "b-tanks", side: "blue", platform: "generic/mbt", count: 4, east: -600, north: 0, order: advance(7000) },
    { id: "b-ifv", side: "blue", platform: "generic/ifv", count: 3, east: 600, north: 0, order: advance(7000) },
    { id: "b-isr", side: "blue", platform: "generic/droneIsrTeam", count: 1, east: 0, north: -1500 },
    { id: "b-fpv", side: "blue", platform: blueStrike, count: 1, east: 0, north: -1000 },
  ];
  if (redJammer) units.push({ id: "r-jam", side: "red", platform: "generic/ewJammer", count: 1, east: 0, north: 7500 });
  return kitGame({ units, seed, domains: { drones: "simulated", ew: "simulated", fires: "off" }, maxDurationS: 3 * 3600 });
}

describe("drones (W5)", () => {
  it("a drone line: strikes land where the drones can reach, and losses happen between the lines", async () => {
    const runner = await runKit(droneLine("line-1"), 2400);
    const w = runner.state.world!;
    const ranges = w.stats.droneStrikeRangesKm ?? [];
    expect(ranges.length, JSON.stringify(w.log.filter((l) => l.domain === "drones").slice(-10))).toBeGreaterThan(3);
    const mean = ranges.reduce((s, r) => s + r, 0) / ranges.length;
    // The kill zone's reach: kilometres, inside what an FPV can fly (P8: 5–20 km typical).
    expect(mean).toBeGreaterThan(1);
    expect(mean).toBeLessThan(20);
    expect(w.log.some((l) => l.domain === "drones" && l.kind === "launch")).toBe(true);
  });

  it("most FPV strikes fail to destroy, as the reference says (60–80%), over many strikes", async () => {
    let fired = 0;
    let kills = 0;
    for (let i = 0; i < 6; i++) {
      const runner = await runKit(droneLine(`fpv-${i}`), 3000);
      const m = runner.state.world!.stats.munitions.fpv;
      fired += m?.fired ?? 0;
      kills += m?.kills ?? 0;
    }
    expect(fired).toBeGreaterThan(20);
    const fail = 1 - kills / fired;
    // Plausibility while the catalogue is synthetic (W12): the right order, not the exact band.
    expect(fail).toBeGreaterThan(0.5);
    expect(fail).toBeLessThan(0.92);
  }, 120_000);

  it("switching to fibre changes what the other side's jamming does", async () => {
    // A blue strike team sends drones at a red unit sitting under a red jammer.
    const reached = async (team: string, seed: string) => {
      const game = kitGame({
        seed,
        units: [
          { id: "b-team", side: "blue", platform: team, count: 1, east: 0, north: 0 },
          { id: "r-target", side: "red", platform: "generic/mbt", count: 4, east: 0, north: 7000, order: { kind: "hold" } },
          { id: "r-jam", side: "red", platform: "generic/ewJammer", count: 1, east: 300, north: 7300 },
        ],
        domains: { drones: "simulated", ew: "simulated" },
      });
      // Four drones away at once, at where the target is known to be; the team then stands down.
      const launched = withWorldCtx(game.state, game.config, (c) => {
        for (let i = 0; i < 4; i++) launchDrone(c, "b-team", { kind: "strike", targetId: "r-target", aimAt: at(0, 7000), altM: 100 }, "test");
        c.setLand("b-team", { drones: 0 });
      });
      const runner = await runKit({ state: launched.state, config: game.config }, 900);
      return runner.state.world!.log.filter((l) => l.side === "blue" && l.domain === "drones" && (l.kind === "strike" || l.kind === "miss")).length;
    };
    let rf = 0;
    let fibre = 0;
    for (let i = 0; i < 4; i++) {
      rf += await reached("generic/fpvTeamRf", `fibre-${i}`);
      fibre += await reached("generic/fpvTeamFibre", `fibre-${i}`);
    }
    // Radio drones fall to the jamming before they reach; fibre drones get there.
    expect(fibre).toBeGreaterThan(12);
    expect(rf).toBeLessThan(fibre / 2);
  }, 120_000);
});

describe("electronic warfare (W6)", () => {
  it("a GNSS jammer cuts precision rounds' accuracy toward the reference band (>50% → <10%)", () => {
    const land = (jammer: boolean) => {
      const units: KitUnit[] = [
        { id: "guns", side: "blue", platform: "generic/sph155", count: 3, east: 0, north: -15_000 },
        { id: "target", side: "red", platform: "generic/convoyTruck", count: 4, east: 0, north: 0 },
      ];
      if (jammer) units.push({ id: "jam", side: "red", platform: "generic/ewJammer", count: 1, east: 2000, north: 1000 });
      const game = kitGame({ units, domains: { ew: "simulated", fires: "simulated" }, seed: "pnt" });
      const view = viewOf(game.state, game.config);
      const c = new WorldCtx(game.state.world!, view);
      if (jammer) c.world.jammers.push({ id: "jam", side: "red", at: at(2000, 1000), radiusM: 25_000, bands: ["gnss", "l", "vhf"], on: true });
      let close = 0;
      for (let i = 0; i < 300; i++) {
        const round = launch(c, { launcherId: "guns", owner: "blue", weaponId: "precision155", from: at(0, -15_000), fromAltM: 2, aimAt: at(0, 0) });
        if (distanceM(round.munition!.aimAt, at(0, 0)) <= 20) close++;
      }
      return close / 300;
    };
    expect(land(false)).toBeGreaterThan(0.5);
    expect(land(true)).toBeLessThan(0.15);
  });

  it("an emitter on the air too long is fixed by bearings and struck: the chain emerges", async () => {
    const game = kitGame({
      units: [
        // Red's company post keeps talking to its platoons.
        { id: "r-hq", side: "red", platform: "generic/cp-company", count: 2, east: 0, north: 12_000, commandRating: 2 },
        { id: "r-1", side: "red", platform: "generic/infantrySection", count: 3, east: -2000, north: 9000, order: { kind: "hold" } },
        { id: "r-2", side: "red", platform: "generic/infantrySection", count: 3, east: 2000, north: 9000, order: { kind: "hold" } },
        // Blue listens from two places, and has guns in range.
        { id: "b-esm-1", side: "blue", platform: "generic/esmTeam", count: 1, east: -8000, north: 0 },
        { id: "b-esm-2", side: "blue", platform: "generic/esmTeam", count: 1, east: 8000, north: 0 },
        { id: "b-guns", side: "blue", platform: "generic/sph155", count: 3, east: 0, north: -4000 },
      ],
      domains: { ew: "simulated", fires: "simulated" },
      comms: "radio",
      maxDurationS: 3 * 3600,
    });
    const runner = await runKit(game, 3600);
    const w = runner.state.world!;
    expect(w.log.some((l) => l.kind === "fix" && l.ids?.includes("r-hq")), JSON.stringify(w.log.filter((l) => l.domain === "ew").slice(-6))).toBe(true);
    const mission = w.missions.find((m) => m.side === "blue" && m.reason?.startsWith("emitter fix"));
    expect(mission).toBeDefined();
    expect(["firing", "inFlight", "complete"]).toContain(mission!.state);
    expect(mission!.times.fired).toBeGreaterThan(mission!.times.requested - 1);
  });

  it("a jammer degrades its own side's radios in its footprint", () => {
    const game = kitGame({
      units: [
        { id: "b-jam", side: "blue", platform: "generic/ewJammer", count: 1, east: 0, north: 0 },
        { id: "b-near", side: "blue", platform: "generic/mbt", count: 4, east: 1000, north: 0 },
        { id: "b-near-2", side: "blue", platform: "generic/mbt", count: 4, east: 2000, north: 500 },
        { id: "b-far", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: -20_000 },
        { id: "b-far-2", side: "blue", platform: "generic/mbt", count: 4, east: 1500, north: -20_500 },
      ],
      domains: { ew: "abstracted" },
      comms: "radio",
    });
    const world = { ...game.state.world!, jammers: [{ id: "b-jam", side: "blue" as const, at: at(0, 0), radiusM: 8000, radii: { vhf: 8000 }, bands: ["vhf", "l", "gnss"], on: true }] };
    // The land radio net: blue's own transmissions, heard inside and outside blue's own jammer.
    const lost = landRadioJamming(world, () => game.state.game, game.config.rng);
    const rate = (hearer: string, sender: string) => Array.from({ length: 2000 }, () => lost(sender, hearer)).filter(Boolean).length / 2000;
    expect(rate("b-near", "b-near-2")).toBeGreaterThan(0.3);
    expect(rate("b-far", "b-far-2")).toBe(0);
    // The C2 nets too: inside the footprint VHF fails and messages fall back down the PACE plan.
    const c = new WorldCtx(world, viewOf(game.state, game.config));
    const bearers = Array.from({ length: 200 }, () => sendNet(c, { side: "blue", kind: "sitrep", from: "b-near-2", to: "b-near", text: "x" }).bearer);
    const farBearers = Array.from({ length: 200 }, () => sendNet(c, { side: "blue", kind: "sitrep", from: "b-far-2", to: "b-far", text: "x" }).bearer);
    expect(bearers.filter((b) => b === "vhf").length).toBeLessThan(160);
    expect(farBearers.every((b) => b === "vhf")).toBe(true);
  });
});

function viewOf(state: RtState, config: Parameters<typeof kitGame>[0] extends never ? never : ReturnType<typeof kitGame>["config"]) {
  return {
    time: state.time,
    config,
    rng: config.rng,
    game: state.game,
    units: { ...state.units },
    setFe: () => {},
    setUnit: () => {},
    order: () => {},
    addUnit: () => {},
    shots: [],
    emit: () => {},
    radio: () => {},
    known: () => "none" as const,
    prev: state,
  };
}
