// ── realtime/world/figures.ts ────────────────────────────────────────────
// Every hand-set number the world layer runs on, with its unit, the module
// that reads it, what it means, where it came from, and the range it could
// plausibly take (docs/plan/MULTI_DOMAIN_PLAN.md W0 step 4, W12).
//
// The engine's own figures are in engine/figures.ts; this is the same
// registry for the systems that came after, kept beside them so the land
// engine's file and its test are untouched. `ALL_FIGURES` joins the two for
// the figures table and the sensitivity report.
//
// `range` is what the sensitivity analysis sweeps (W12): the low and high
// ends of what the value could be, not its error bars.

import { FIGURES, type Figure } from "../engine/figures";

export interface WorldFigure extends Figure {
  /** Plausible low and high values, for the sensitivity sweep. */
  range: [number, number];
}

const declared = { kind: "declared" } as const;
const ref = (r: string) => ({ kind: "ref", ref: r }) as const;

export const WORLD_FIGURES = {
  // ── Effects on land units (world/landEffects) ──
  "world/landEffects.SPREAD_M": { value: 60, unit: "m", module: "world/landEffects", source: declared, range: [30, 120], note: "How far a platoon's vehicles are spread about its centre, for blast and area fire." },
  "world/landEffects.COVER_FACTOR": { value: 0.6, unit: "×", module: "world/landEffects", source: declared, range: [0.4, 0.8], note: "A unit in woods or buildings takes this share of the blast it would in the open." },
  "world/landEffects.DUG_IN_FACTOR": { value: 0.6, unit: "×", module: "world/landEffects", source: declared, range: [0.4, 0.8], note: "A fully dug-in position cuts blast casualties by this much (scaled by how dug in)." },
  "world/landEffects.BLAST_VS_ARMOUR": { value: 0.12, unit: "×", module: "world/landEffects", source: declared, range: [0.05, 0.25], note: "Blast that cannot penetrate still disables an armoured vehicle this share of the time (tracks, optics)." },
  "world/landEffects.BLAST_VS_LIGHT": { value: 0.5, unit: "×", module: "world/landEffects", source: declared, range: [0.3, 0.8], note: "Blast against a light vehicle, as a share of its effect on a soft one." },
  "world/landEffects.SUPPRESSION_NEAR": { value: 35, module: "world/landEffects", source: declared, range: [20, 50], note: "Suppression a round adds to a unit it lands on; it falls off with distance." },
  "world/landEffects.SUPPRESSION_FALLOFF_M": { value: 200, unit: "m", module: "world/landEffects", source: declared, range: [100, 400], note: "Distance over which a round's suppression falls to a third." },
  "world/landEffects.KILLED_SHARE": { value: 0.3, unit: "×", module: "world/landEffects", source: declared, range: [0.2, 0.4], note: "Of the people in a knocked-out vehicle or team, the share killed; the rest are wounded." },
  "world/landEffects.CREW_HURT_SHARE": { value: 0.5, unit: "×", module: "world/landEffects", source: declared, range: [0.3, 0.8], note: "Of a knocked-out vehicle's crew, the share hurt at all." },

  // ── The step itself (world/step) ──
  "world/step.LOG_KEEP": { value: 300, module: "world/step", source: declared, range: [300, 300], note: "World feed lines kept in the state (the full feed streams out as it is made)." },

  // ── Environment (world/environment) ──
  "world/environment.NIGHT_DETECT": { value: 0.35, unit: "×", module: "world/environment", source: declared, range: [0.2, 0.5], note: "Unaided eyes at night see this share of what they see by day; thermal sights are not affected." },
  "world/environment.CROSSOVER_THERMAL": { value: 0.6, unit: "×", module: "world/environment", source: ref("R4.1"), range: [0.4, 0.8], note: "Thermal sights near dawn and dusk crossover, when targets and ground are the same temperature." },
  "world/environment.DRONE_MAX_WIND_MS": { value: 12, unit: "m/s", module: "world/environment", source: declared, range: [9, 15], note: "Small multirotor drones cannot fly in more wind than this." },

  // ── PNT (world/pnt) ──
  "world/pnt.JAM_DEPTH": { value: 0.9, unit: "×", module: "world/pnt", source: declared, range: [0.7, 0.98], note: "At a GNSS jammer's centre, GNSS quality falls by this share; less towards the edge of its footprint." },
  "world/pnt.INERTIAL_CEP_M": { value: 150, unit: "m", module: "world/pnt", source: declared, range: [80, 300], note: "A GNSS-guided round with no GNSS at all falls back on inertial guidance: its circular error grows to about this." },
} as const satisfies Record<string, WorldFigure>;

export type WorldFigureKey = keyof typeof WORLD_FIGURES;

/** Overrides for this game (a scenario's figure overrides, or a sensitivity run). */
let overrides: Partial<Record<string, number>> = {};

/** A world figure's value, or this game's override of it. */
export function wfig(key: WorldFigureKey): number {
  return overrides[key] ?? WORLD_FIGURES[key].value;
}

/**
 * Run `fn` with some figures overridden (a sensitivity sweep, a scenario's
 * overrides in a headless batch). Restored afterwards, even if it throws.
 */
export async function withFigures<T>(values: Partial<Record<string, number>>, fn: () => Promise<T> | T): Promise<T> {
  const before = overrides;
  overrides = { ...before, ...values };
  try {
    return await fn();
  } finally {
    overrides = before;
  }
}

/** Every figure the real-time game runs on, engine and world, for the table and the research backlog. */
export function allFigures(): Record<string, Figure & { range?: [number, number] }> {
  return { ...FIGURES, ...WORLD_FIGURES };
}
