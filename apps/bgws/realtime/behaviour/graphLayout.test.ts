import { describe, expect, it } from "vitest";
import { layoutTemplate } from "./graphLayout";
import { bundledTemplateFiles } from "./load";
import { resolveTemplate } from "./walk";
import type { Template } from "./types";

describe("the graph view's layout", () => {
  it("lays out every built-in template: triggers and nodes placed, edges to placed boxes, no overlaps in a column", () => {
    const all = Object.values(bundledTemplateFiles()) as Template[];
    const reg = new Map(all.map((t) => [t.id, t]));
    for (const tpl of all) {
      const t = resolveTemplate(tpl.id, reg);
      const g = layoutTemplate(t);
      const ids = new Set(g.nodes.map((n) => n.id));
      for (const k of Object.keys(t.triggers)) expect(ids.has(`trigger:${k}`)).toBe(true);
      for (const k of Object.keys(t.nodes)) expect(ids.has(k), `${tpl.id}: ${k}`).toBe(true);
      for (const e of g.edges) expect(ids.has(e.from) && ids.has(e.to), `${tpl.id}: ${e.from}→${e.to}`).toBe(true);
      const byCol = new Map<number, typeof g.nodes>();
      for (const n of g.nodes) byCol.set(n.x, [...(byCol.get(n.x) ?? []), n]);
      for (const col of byCol.values()) {
        const sorted = [...col].sort((a, b) => a.y - b.y);
        for (let i = 1; i < sorted.length; i++) expect(sorted[i].y).toBeGreaterThanOrEqual(sorted[i - 1].y + sorted[i - 1].h);
      }
    }
  });
});
