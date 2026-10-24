import { describe, expect, it } from "vitest";
import { SITUATIONS } from "../engine/situations";
import { defaultTemplateForClass } from "./archetypes";
import { benchScene } from "./bench";
import type { Template } from "./types";

const standFast: Template = { id: "custom/stand-fast", version: 1, name: "Stand fast", parent: "core/armour", remove: ["pullBack"] };

describe("behaviour test bench", () => {
  it("shows exactly what the game offers and chooses, in every scene", () => {
    for (const situation of SITUATIONS) {
      const { state, request } = situation.build();
      const template = defaultTemplateForClass(state.game.forceElements[request.unitId].targetClass);
      const bench = benchScene(situation.id, template);
      expect(bench.offered, situation.id).toEqual(request.options);
      expect(bench.rulesChoice, situation.id).toBe(request.fallback);
      expect(bench.path.length, situation.id).toBeGreaterThan(0);
      // Every option offered came from a row marked offered.
      const fromRows = bench.rows.filter((r) => r.status === "offered").flatMap((r) => r.options.map((o) => o.id));
      for (const option of bench.offered) if (option.id !== "keep") expect(fromRows, `${situation.id}:${option.id}`).toContain(option.id);
    }
  });

  it("says what a template changed: Stand fast removes pulling back, and the rows say so", () => {
    const withPullBack = SITUATIONS.find((s) => benchScene(s.id, "core/armour").rows.some((r) => r.action.includes("pullBack") && r.status === "offered"));
    expect(withPullBack).toBeDefined();
    const before = benchScene(withPullBack!.id, "core/armour");
    const after = benchScene(withPullBack!.id, standFast.id, [standFast]);
    expect(after.lineage).toEqual(["custom/stand-fast", "core/armour", "core/default"]);
    expect(after.rows.filter((r) => r.status === "removed").map((r) => r.action).join(" ")).toMatch(/pullBack/i);
    expect(after.offered.length).toBeLessThan(before.offered.length);
    expect(after.offered.some((o) => o.id === "pullBack")).toBe(false);
  });
});
