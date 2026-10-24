// ── realtime/world/vignettes.ts ──────────────────────────────────────────
// The calibration vignettes (MULTI_DOMAIN_PLAN.md W12): small games whose
// outcome the reference gives as a figure, each with the band the outcome
// should fall in while the catalogue is synthetic — plausibility, the right
// order of magnitude, not a match. The vignette tests, the headless batch
// (`npm run bgws:run -- --vignettes`) and the sensitivity report all run
// these same games.

import { distanceM } from "../../lib/board";
import { launch } from "./munitions";
import { withWorldCtx } from "./standalone";
import { at, kitGame, runKit, type KitEntity } from "./testkit";
import type { WorldFigureKey } from "./figures";

export interface Vignette {
  id: string;
  name: string;
  /** The reference's figure, in words, and where it comes from. */
  reference: string;
  unit: string;
  /** The plausible band the batch's median must fall in. */
  band: [number, number];
  /** Play one seeded game and measure it; undefined if this run produced nothing to measure. */
  run(seed: string): Promise<number | undefined>;
  /** The figures that drive it, for the sensitivity sweep. */
  figures: WorldFigureKey[];
}

const advance = (north: number) => ({ kind: "move" as const, to: at(0, north), mode: "tactical" as const });

async function breach(seed: string): Promise<number | undefined> {
  const g = kitGame({
    seed,
    units: [
      { id: "tanks", side: "blue", platform: "generic/mbt", count: 4, east: 0, north: 0, order: { kind: "move", to: at(0, 2000), mode: "tactical", route: [at(0, 2000)] } },
      { id: "sappers", side: "blue", platform: "generic/engineerSection", count: 2, east: 300, north: -600 },
      { id: "far", side: "red", platform: "generic/infantrySection", count: 1, east: 9000, north: 9000, order: { kind: "hold" } },
    ],
    obstacles: [{ owner: "red", kind: "minefield", east: 0, north: 800, widthM: 1500, depthM: 300, bearingDeg: 0, density: 0.5 }],
    domains: { logistics: "off" },
    maxDurationS: 5 * 3600,
  });
  const r = await runKit(g, 4 * 3600);
  return r.state.world!.stats.breachS[0];
}

async function fpv(seed: string): Promise<number | undefined> {
  const g = kitGame({
    seed,
    units: [
      { id: "r-isr", side: "red", platform: "generic/droneIsrTeam", count: 1, east: -1500, north: 8000 },
      { id: "r-fpv-1", side: "red", platform: "generic/fpvTeamRf", count: 1, east: 0, north: 9000 },
      { id: "r-fpv-2", side: "red", platform: "generic/fpvTeamRf", count: 1, east: 2000, north: 8500 },
      { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 7000, order: { kind: "hold" } },
      { id: "b-tanks", side: "blue", platform: "generic/mbt", count: 4, east: -600, north: 0, order: advance(7000) },
      { id: "b-ifv", side: "blue", platform: "generic/ifv", count: 3, east: 600, north: 0, order: advance(7000) },
    ],
    domains: { drones: "simulated", ew: "simulated", fires: "off" },
    maxDurationS: 3 * 3600,
  });
  const r = await runKit(g, 3000);
  const m = r.state.world!.stats.munitions.fpv;
  return m && m.fired > 0 ? 1 - m.kills / m.fired : undefined;
}

async function shahed(seed: string): Promise<number | undefined> {
  const raid: KitEntity[] = [];
  for (let i = 0; i < 30; i++) {
    const decoy = i % 5 >= 3;
    raid.push({ id: `owa-${i}`, owner: "red", archetype: decoy ? "generic/owaDecoy" : "generic/owaDrone", east: -20_000 + i * 1300, north: 120_000 + (i % 3) * 2000, altM: 1500, task: { kind: "strike", aimAt: at((i % 3) * 60 - 60, (i % 2) * 60 - 30), altM: 1500 } });
  }
  const g = kitGame({
    seed,
    units: [
      { id: "b-sam", side: "blue", platform: "generic/samMedium", count: 2, east: 0, north: 8000 },
      { id: "b-shorad-1", side: "blue", platform: "generic/shorad", count: 2, east: -2000, north: 2500 },
      { id: "b-shorad-2", side: "blue", platform: "generic/shorad", count: 2, east: 2000, north: 2500 },
      { id: "r-inf", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 150_000, order: { kind: "hold" } },
    ],
    entities: [{ id: "b-depot", owner: "blue", archetype: "generic/depot", east: 0, north: 0 }, ...raid],
    domains: { air: "simulated", airDefence: "simulated", ew: "simulated", drones: "simulated" },
    maxDurationS: 6 * 3600,
  });
  const r = await runKit(g, 3600);
  const owa = Object.values(r.state.world!.entities).filter((e) => e.id.startsWith("owa-"));
  return owa.filter((e) => e.status === "destroyed").length / owa.length;
}

function pnt(jammer: boolean) {
  return async (seed: string): Promise<number | undefined> => {
    const units = [
      { id: "guns", side: "blue" as const, platform: "generic/sph155", count: 3, east: 0, north: -15_000 },
      { id: "target", side: "red" as const, platform: "generic/convoyTruck", count: 4, east: 0, north: 0 },
      ...(jammer ? [{ id: "jam", side: "red" as const, platform: "generic/ewJammer", count: 1, east: 2000, north: 1000 }] : []),
    ];
    const g = kitGame({ units, domains: { ew: "simulated", fires: "simulated" }, seed });
    let close = 0;
    withWorldCtx(g.state, g.config, (c) => {
      if (jammer) c.world.jammers.push({ id: "jam", side: "red", at: at(2000, 1000), radiusM: 25_000, bands: ["gnss", "l", "vhf"], on: true });
      for (let i = 0; i < 100; i++) {
        const round = launch(c, { launcherId: "guns", owner: "blue", weaponId: "precision155", from: at(0, -15_000), fromAltM: 2, aimAt: at(0, 0) });
        if (distanceM(round.munition!.aimAt, at(0, 0)) <= 20) close++;
      }
    });
    return close / 100;
  };
}

async function redSea(seed: string): Promise<number | undefined> {
  const raid: KitEntity[] = [];
  for (let i = 0; i < 20; i++) raid.push({ id: `owa-${i}`, owner: "red", archetype: i % 4 === 3 ? "generic/owaDecoy" : "generic/owaDrone", east: 140_000 + i * 500, north: -20_000 + i * 2000, altM: 500, task: { kind: "strike", targetId: i % 2 ? "b-ddg" : "b-ffg", aimAt: at(0, 0), altM: 500 } });
  const g = kitGame({
    seed,
    units: [{ id: "r-asm", side: "red", platform: "generic/coastalAsm", count: 2, east: 150_000, north: 0, order: { kind: "hold" } }],
    entities: [
      { id: "b-ddg", owner: "blue", archetype: "generic/destroyer", east: 0, north: 0, task: { kind: "station", at: at(0, 0), radiusM: 10_000 } },
      { id: "b-ffg", owner: "blue", archetype: "generic/frigate", east: 5000, north: 3000, task: { kind: "station", at: at(5000, 3000), radiusM: 10_000 } },
      { id: "r-uav", owner: "red", archetype: "generic/male", east: 30_000, north: 0, altM: 5000, task: { kind: "orbit", at: at(20_000, 0), radiusM: 5000, altM: 5000 } },
      ...raid,
    ],
    domains: { naval: "simulated", air: "simulated", airDefence: "simulated", drones: "simulated" },
    breakpoint: 1,
    maxDurationS: 6 * 3600,
  });
  const r = await runKit(g, 2 * 3600);
  const w = r.state.world!;
  // Interceptors fired per threat engaged: the Red Sea's question of magazine depth.
  const interceptors = ["samLong", "samMedium", "samShort"].reduce((s, k) => s + (w.stats.munitions[k]?.fired ?? 0), 0);
  const threats = Object.values(w.entities).filter((e) => (e.id.startsWith("owa-") || e.munition?.weaponId === "ascm") && e.status === "destroyed").length;
  return threats ? interceptors / threats : undefined;
}

function killWeb(network: "voice" | "datalink") {
  return async (seed: string): Promise<number | undefined> => {
    const g = kitGame({
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
    });
    g.state.world!.network = { blue: network, red: "datalink" };
    const r = await runKit(g, 3 * 3600);
    const fired = r.state.world!.missions.filter((m) => m.side === "blue" && m.times.fired != null).map((m) => m.times.fired! - (m.times.found ?? m.times.requested));
    return fired.length ? Math.min(...fired) / 60 : undefined;
  };
}

/** The share of its vehicles an attacking battalion had lost when its HQ decided to halt the attack. */
async function battalionHalt(seed: string): Promise<number | undefined> {
  const units = [
    { id: "b-hq", side: "blue" as const, platform: "generic/cp-battalion", count: 3, east: 0, north: -2500, commandRating: 3 },
    ...[-1500, 0, 1500].map((e, i) => ({ id: `b-${i}`, side: "blue" as const, platform: "generic/mbt", count: 4, east: e, north: 0, order: advance(4000) })),
    ...[-1500, 0, 1500].map((e, i) => ({ id: `b-ifv-${i}`, side: "blue" as const, platform: "generic/ifv", count: 3, east: e + 300, north: -400, order: advance(4000) })),
    ...[-1200, 0, 1200].map((e, i) => ({ id: `r-${i}`, side: "red" as const, platform: "generic/atgmTeam", count: 2, east: e, north: 3800, order: { kind: "hold" as const } })),
    { id: "r-inf", side: "red" as const, platform: "generic/infantrySection", count: 3, east: 0, north: 4200, order: { kind: "hold" as const } },
  ];
  const subs = units.filter((u) => u.side === "blue" && u.id !== "b-hq").map((u) => u.id);
  const g = kitGame({ seed, units, hqs: [{ id: "blue-bn", side: "blue", echelon: "battalion", postId: "b-hq", subordinates: subs, planner: "land", seat: "heuristic" }], scope: { planners: ["land"] }, domains: { logistics: "off" }, maxDurationS: 3 * 3600 });
  const r = await runKit(g, 2 * 3600);
  // The moment the commander decided to halt: the losses that made the HQ replan, and the plan that halted.
  const halted = r.log.find((e) => e.type === "world" && /CCIR: (\d+)% losses/.test(e.item.text));
  if (halted && halted.type === "world") return Number(/CCIR: (\d+)% losses/.exec(halted.item.text)![1]) / 100;
  // Never halted: what it had lost when the fight ended.
  const fleet = subs.map((id) => r.state.units[id]).filter(Boolean);
  return 1 - fleet.reduce((s, u) => s + u.vehicles.fit, 0) / fleet.reduce((s, u) => s + u.vehicles.total, 0);
}


export const VIGNETTES: Vignette[] = [
  { id: "breach", name: "Minefield breach", reference: "about 1.5 h per lane [ref R4.5, P13]", unit: "s", band: [0.5 * 5400, 1.5 * 5400], run: breach, figures: ["world/domains/landExt.BREACH_S", "world/domains/landExt.UNDER_FIRE_SLOW"] },
  { id: "battalion", name: "Attacking battalion", reference: "halts near 20% casualties [ref, doctrine]", unit: "share lost", band: [0.05, 0.6], run: battalionHalt, figures: ["world/command/heuristicPlanner.CULMINATION_LOSS"] },
  { id: "fpv", name: "FPV effectiveness", reference: "60–80% of strikes fail to destroy [ref P8]", unit: "share failing", band: [0.5, 0.92], run: fpv, figures: ["world/domains/drones.MOVING_HIT", "world/domains/drones.ACQUIRE_IN_COVER", "world/domains/drones.JAMMED_HIT"] },
  { id: "shahed", name: "Shahed raid", reference: "about 85% intercepted [ref P10]", unit: "share intercepted", band: [0.65, 0.97], run: shahed, figures: ["world/domains/air.AD_ASK_S", "world/domains/air.LOW_FACTOR"] },
  { id: "pnt-clear", name: "Precision rounds, GNSS clear", reference: "more than 50% effective [ref R3, P15]", unit: "share within 20 m", band: [0.5, 1], run: pnt(false), figures: ["world/pnt.INERTIAL_CEP_M"] },
  { id: "pnt-jammed", name: "Precision rounds under GNSS jamming", reference: "down toward 10% [ref R3, P15]", unit: "share within 20 m", band: [0, 0.15], run: pnt(true), figures: ["world/pnt.JAM_DEPTH", "world/pnt.INERTIAL_CEP_M"] },
  { id: "redsea", name: "Red Sea exchange", reference: "interceptors used per threat downed [ref R6.2]", unit: "interceptors per kill", band: [1, 6], run: redSea, figures: ["world/domains/naval.VLS_PER_S", "world/domains/air.ENERGY_FALLOFF"] },
  { id: "killweb-datalink", name: "Kill chain, data link", reference: "minutes [ref R3, P2]", unit: "min", band: [1, 20], run: killWeb("datalink"), figures: ["world/domains/killweb.DATALINK_STAFF_S"] },
  { id: "killweb-voice", name: "Kill chain, voice nets", reference: "hours [ref R3, P2]", unit: "min", band: [15, 240], run: killWeb("voice"), figures: ["world/domains/killweb.VOICE_STAFF_S"] },
];
