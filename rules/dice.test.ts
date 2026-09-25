import { describe, expect, it } from "vitest";

import { createRng } from "./dice";

describe("seeded dice", () => {
  it("replays exactly from the same seed", () => {
    // The whole after-action review depends on this one property.
    const a = createRng("battle-1");
    const b = createRng("battle-1");
    const rollsA = Array.from({ length: 50 }, () => a.d66().total);
    const rollsB = Array.from({ length: 50 }, () => b.d66().total);
    expect(rollsA).toEqual(rollsB);
  });

  it("differs between seeds", () => {
    const a = Array.from({ length: 50 }, (_, i) => createRng("seed-a").d66().total + i);
    const b = Array.from({ length: 50 }, (_, i) => createRng("seed-b").d66().total + i);
    expect(a).not.toEqual(b);
  });

  it("resumes mid-game from a cursor", () => {
    // A saved game stores the seed and the cursor, not the generator's state.
    const continuous = createRng("resume");
    for (let i = 0; i < 10; i++) continuous.d66();
    const after = continuous.d66().total;

    const resumed = createRng("resume", 20); // 10 x d66 = 20 draws
    expect(resumed.d66().total).toBe(after);
  });

  it("produces 2D6 in range, and both dice", () => {
    const rng = createRng("range");
    for (let i = 0; i < 500; i++) {
      const roll = rng.d66();
      expect(roll.dice).toHaveLength(2);
      expect(roll.total).toBeGreaterThanOrEqual(2);
      expect(roll.total).toBeLessThanOrEqual(12);
      for (const die of roll.dice) {
        expect(die).toBeGreaterThanOrEqual(1);
        expect(die).toBeLessThanOrEqual(6);
      }
    }
  });

  it("is shaped like 2D6, not like a uniform 2-12", () => {
    // A wargame tuned against a bell curve behaves very differently on a flat
    // one. Worth one test that the distribution is the one the tables assume.
    const rng = createRng("distribution");
    const counts = new Map<number, number>();
    const n = 20_000;
    for (let i = 0; i < n; i++) {
      const total = rng.d66().total;
      counts.set(total, (counts.get(total) ?? 0) + 1);
    }
    const sevens = (counts.get(7) ?? 0) / n;
    const twos = (counts.get(2) ?? 0) / n;
    expect(sevens).toBeGreaterThan(0.13); // theoretical 1/6
    expect(sevens).toBeLessThan(0.20);
    expect(twos).toBeGreaterThan(0.015); // theoretical 1/36
    expect(twos).toBeLessThan(0.045);
  });

  it("reports a cursor that advances with every draw", () => {
    const rng = createRng("cursor");
    expect(rng.cursor).toBe(0);
    rng.d66();
    expect(rng.cursor).toBe(2);
    rng.d6();
    expect(rng.cursor).toBe(3);
  });

  it("picks reproducibly", () => {
    const options = ["a", "b", "c", "d"];
    const first = createRng("pick").pick(options);
    const second = createRng("pick").pick(options);
    expect(first).toBe(second);
    expect(options).toContain(first);
  });
});
