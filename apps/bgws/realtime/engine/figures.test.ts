import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { FIGURES, type Figure } from "./figures";

const entries = Object.entries(FIGURES) as [string, Figure][];
const source = (module: string) => readFileSync(join(__dirname, `${module}.ts`), "utf8");

describe("the figures registry", () => {
  it("gives every figure a finite value, a note and a provenance", () => {
    expect(entries.length).toBeGreaterThan(40);
    for (const [key, f] of entries) {
      expect(Number.isFinite(f.value), key).toBe(true);
      expect(f.note.length, key).toBeGreaterThan(10);
      expect(f.note, key).not.toMatch(/no note in the source module/);
      expect(["declared", "ref"], key).toContain(f.source.kind);
      if (f.source.kind === "ref") expect(f.source.ref, key).toMatch(/^(R\d+(\.\d+)?|P\d+)/);
      expect(key.startsWith(`${f.module}.`), key).toBe(true);
    }
  });

  it("is what the modules actually read — no orphan entries, no unknown keys", () => {
    const modules = [...new Set(entries.map(([, f]) => f.module))];
    const used = new Set<string>();
    for (const m of modules)
      for (const [, key] of source(m).matchAll(/figure\("([^"]+)"\)/g)) {
        expect(FIGURES, `${m}.ts reads unknown figure ${key}`).toHaveProperty([key]);
        used.add(key);
      }
    expect(entries.map(([k]) => k).filter((k) => !used.has(k))).toEqual([]);
  });
});
