// ── realtime/world/batch.ts ──────────────────────────────────────────────
// Headless batches and the sensitivity report (MULTI_DOMAIN_PLAN.md W12).
// A batch plays N seeded runs of a vignette with the rules deciding and
// reports the outcome as a distribution; the sensitivity sweep plays each
// driving figure at the low and high ends of its plausible range and says
// which conclusions change. Results are ranges, never point predictions.

import { allFigures, withFigures, WORLD_FIGURES, type WorldFigureKey } from "./figures";
import type { Vignette } from "./vignettes";

export interface Distribution {
  n: number;
  mean: number;
  p10: number;
  p50: number;
  p90: number;
  min: number;
  max: number;
}

export function distribution(values: number[]): Distribution {
  const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  const q = (p: number) => (v.length ? v[Math.min(v.length - 1, Math.max(0, Math.round(p * (v.length - 1))))] : NaN);
  return { n: v.length, mean: v.length ? v.reduce((s, x) => s + x, 0) / v.length : NaN, p10: q(0.1), p50: q(0.5), p90: q(0.9), min: v[0] ?? NaN, max: v[v.length - 1] ?? NaN };
}

export interface BatchResult {
  vignette: string;
  runs: number;
  outcome: Distribution;
  /** The median inside the vignette's band. */
  inBand: boolean;
}

/** N seeded runs of a vignette, with whatever figures are in force. */
export async function runBatch(v: Vignette, runs: number, seedPrefix = v.id): Promise<BatchResult> {
  const values: number[] = [];
  for (let i = 0; i < runs; i++) {
    const x = await v.run(`${seedPrefix}-${i}`);
    if (x != null) values.push(x);
  }
  const outcome = distribution(values);
  return { vignette: v.id, runs, outcome, inBand: outcome.p50 >= v.band[0] && outcome.p50 <= v.band[1] };
}

export interface SensitivityRow {
  figure: WorldFigureKey;
  base: number;
  low: { value: number; p50: number; inBand: boolean };
  high: { value: number; p50: number; inBand: boolean };
  /** The conclusion (in band or not) differs at one end of the range. */
  changesConclusion: boolean;
  /** How far the median moves across the range, as a share of the base median. */
  swing: number;
}

export async function sensitivity(v: Vignette, runs: number, figures: WorldFigureKey[] = v.figures): Promise<{ base: BatchResult; rows: SensitivityRow[] }> {
  const base = await runBatch(v, runs);
  const rows: SensitivityRow[] = [];
  for (const key of figures) {
    const fig = WORLD_FIGURES[key];
    const [lo, hi] = fig.range;
    const low = await withFigures({ [key]: lo }, () => runBatch(v, runs));
    const high = await withFigures({ [key]: hi }, () => runBatch(v, runs));
    rows.push({
      figure: key,
      base: fig.value,
      low: { value: lo, p50: low.outcome.p50, inBand: low.inBand },
      high: { value: hi, p50: high.outcome.p50, inBand: high.inBand },
      changesConclusion: low.inBand !== base.inBand || high.inBand !== base.inBand,
      swing: Math.abs(high.outcome.p50 - low.outcome.p50) / Math.max(1e-9, Math.abs(base.outcome.p50)),
    });
  }
  return { base, rows: rows.sort((a, b) => b.swing - a.swing) };
}

const fmt = (x: number) => (Number.isFinite(x) ? (Math.abs(x) >= 100 ? Math.round(x).toString() : x.toFixed(2)) : "—");

/** The sensitivity report as Markdown, for the UI and the Node runner. */
export function sensitivityReport(v: Vignette, result: { base: BatchResult; rows: SensitivityRow[] }): string {
  const b = result.base.outcome;
  const lines = [
    `## ${v.name}`,
    "",
    `Reference: ${v.reference}. Plausible band: ${fmt(v.band[0])}–${fmt(v.band[1])} ${v.unit}.`,
    "",
    `Base (${result.base.runs} runs): median ${fmt(b.p50)} ${v.unit}, P10–P90 ${fmt(b.p10)}–${fmt(b.p90)}; ${result.base.inBand ? "in band" : "**out of band**"}.`,
    "",
    "| Figure | Base | Low → median | High → median | Swing | Conclusion |",
    "|---|---|---|---|---|---|",
    ...result.rows.map((r) => `| ${r.figure} | ${fmt(r.base)} | ${fmt(r.low.value)} → ${fmt(r.low.p50)} | ${fmt(r.high.value)} → ${fmt(r.high.p50)} | ${Math.round(r.swing * 100)}% | ${r.changesConclusion ? "**changes**" : "holds"} |`),
  ];
  return lines.join("\n");
}

/** The research backlog: figures with the weakest sources, worst first (declared before referenced, widest range first). */
export function researchBacklog(limit = 25): { key: string; source: string; note: string; spread: number }[] {
  return Object.entries(allFigures())
    .map(([key, f]) => ({ key, source: f.source.kind, note: f.note, spread: f.range ? Math.abs(f.range[1] - f.range[0]) / Math.max(1e-9, Math.abs(f.value)) : 0 }))
    .sort((a, b) => (a.source === b.source ? b.spread - a.spread : a.source === "declared" ? -1 : 1))
    .slice(0, limit);
}
