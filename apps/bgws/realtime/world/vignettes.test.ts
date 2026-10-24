// W12: every calibration vignette's outcome, as a distribution over seeded
// runs with the rules deciding, falls in its plausible band; the sensitivity
// sweep says which conclusions a figure's range can change; and every figure
// in the registry is read by the code that claims it.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { researchBacklog, runBatch, sensitivity, sensitivityReport } from "./batch";
import { WORLD_FIGURES } from "./figures";
import { VIGNETTES } from "./vignettes";

const RUNS: Record<string, number> = { breach: 12, battalion: 4, fpv: 4, shahed: 4, "pnt-clear": 4, "pnt-jammed": 4, redsea: 3, "killweb-datalink": 3, "killweb-voice": 3 };

describe("calibration vignettes (W12)", () => {
  for (const v of VIGNETTES) {
    it(`${v.name}: ${v.reference} — median in ${v.band[0]}–${v.band[1]} ${v.unit}`, async () => {
      const r = await runBatch(v, RUNS[v.id] ?? 4);
      expect(r.outcome.n, "runs that produced an outcome").toBeGreaterThan(0);
      expect(r.inBand, JSON.stringify(r.outcome)).toBe(true);
    }, 300_000);
  }

  it("a sensitivity sweep reports, for each driving figure, the outcome at both ends of its range", async () => {
    const v = VIGNETTES.find((x) => x.id === "pnt-jammed")!;
    const result = await sensitivity(v, 2);
    expect(result.rows.length).toBe(v.figures.length);
    for (const row of result.rows) {
      expect(row.low.value).toBeLessThan(row.high.value);
      expect(Number.isFinite(row.low.p50) && Number.isFinite(row.high.p50)).toBe(true);
    }
    const report = sensitivityReport(v, result);
    expect(report).toMatch(/\| Figure \| Base \|/);
    expect(report).toMatch(/world\/pnt\.JAM_DEPTH/);
  }, 120_000);

  it("the research backlog puts declared figures first", () => {
    const backlog = researchBacklog(10);
    expect(backlog.length).toBe(10);
    expect(backlog[0].source).toBe("declared");
  });
});

describe("the figures registry (W0, W12)", () => {
  const root = join(__dirname, "..");
  const sources: string[] = [];
  const walk = (dir: string) => {
    for (const name of readdirSync(dir)) {
      const p = join(dir, name);
      if (statSync(p).isDirectory()) walk(p);
      else if (/\.tsx?$/.test(name) && !/\.test\./.test(name) && !name.endsWith("figures.ts")) sources.push(readFileSync(p, "utf8"));
    }
  };
  walk(root);
  const all = sources.join("\n");

  it("every world figure is read by the code, and its module exists", () => {
    const unused = Object.keys(WORLD_FIGURES).filter((k) => !all.includes(`"${k}"`));
    expect(unused).toEqual([]);
    for (const [key, f] of Object.entries(WORLD_FIGURES)) {
      expect(key.startsWith(`${f.module}.`), key).toBe(true);
      expect(f.range[0] <= f.value && f.value <= f.range[1], `${key}: ${f.value} outside ${f.range}`).toBe(true);
    }
  });
});
