// Behaviour templates in play: assignments, conditions, preferences and
// unit-type defaults, exercised through the real optionsAt on every named
// situation (engine/situations.ts).

import { describe, expect, it } from "vitest";
import { optionsAt } from "../engine/decisions";
import { SITUATIONS } from "../engine/situations";
import type { RtConfig, RtOption } from "../engine/types";
import { behaviourDoctrine, templateIdFor } from "./actions/land";
import { DEFAULT_TEMPLATE_ID, defaultTemplateForClass } from "./archetypes";
import type { Template } from "./types";

const scenes = SITUATIONS.map((s) => ({ name: s.id, ...s.build() }));

/** The options a scene's unit is offered under this template (no template = as configured). */
function offered(scene: (typeof scenes)[number], template?: Template): RtOption[] {
  const { state, config, request } = scene;
  const cfg: RtConfig = template
    ? { ...config, behaviours: { [request.unitId]: template.id }, behaviourTemplates: [template] }
    : config;
  return optionsAt(state, request.unitId, request.point, request.event, cfg);
}
const ids = (options: RtOption[]) => options.map((o) => o.id);

const variant = (id: string, extra: Partial<Template>): Template => ({ id, version: 1, parent: DEFAULT_TEMPLATE_ID, ...extra });

describe("behaviour templates in play", () => {
  it("a unit assigned 'stand fast' is never offered pulling back", () => {
    const standFast = variant("custom/stand-fast", { remove: ["pullBack"] });
    const withPullBack = scenes.filter((s) => ids(offered(s)).some((id) => id.startsWith("pullBack")));
    expect(withPullBack.length).toBeGreaterThan(0);
    for (const s of withPullBack) {
      const after = ids(offered(s, standFast));
      expect(after.filter((id) => id.startsWith("pullBack")), s.name).toEqual([]);
      // Everything else is still offered, in the same order.
      expect(after, s.name).toEqual(ids(offered(s)).filter((id) => !id.startsWith("pullBack")));
    }
  });

  it("an 'only when' condition removes an option when it does not hold", () => {
    const pointBlank = variant("custom/point-blank", { when: { engage: { fact: "enemy.rangeM", op: "<=", value: 1 } } });
    const withEngage = scenes.filter((s) => ids(offered(s)).includes("engage"));
    expect(withEngage.length).toBeGreaterThan(0);
    for (const s of withEngage) expect(ids(offered(s, pointBlank)), s.name).not.toContain("engage");
  });

  it("a preference travels with the options its action offers", () => {
    const wary = variant("custom/wary", { bias: { observe: -2 } });
    const withObserve = scenes.filter((s) => ids(offered(s)).includes("observe"));
    expect(withObserve.length).toBeGreaterThan(0);
    for (const s of withObserve) {
      const after = offered(s, wary);
      expect(after.find((o) => o.id === "observe")?.preference, s.name).toBe(-2);
      expect(after.filter((o) => o.id !== "observe").every((o) => o.preference === undefined), s.name).toBe(true);
    }
  });

  it("an unknown or invalid template falls back to the built-in behaviour", () => {
    const s = scenes[0];
    const broken = variant("custom/broken", { remove: ["keep"] }); // removes fallbacks: invalid
    expect(ids(offered(s, broken))).toEqual(ids(offered(s)));
    const cfg = { ...s.config, behaviours: { [s.request.unitId]: "custom/missing" } };
    expect(ids(optionsAt(s.state, s.request.unitId, s.request.point, s.request.event, cfg))).toEqual(ids(offered(s)));
  });

  it("units default to their type's template, which carries doctrine for Jev", () => {
    expect(defaultTemplateForClass("armoured_vehicle")).toBe("core/armour");
    expect(defaultTemplateForClass("foot")).toBe("core/infantry");
    expect(defaultTemplateForClass("soft_skin")).toBe(DEFAULT_TEMPLATE_ID);
    expect(defaultTemplateForClass(undefined)).toBe(DEFAULT_TEMPLATE_ID);
    for (const s of scenes) {
      const fe = s.state.game.forceElements[s.request.unitId];
      const id = templateIdFor(s.request.unitId, s.state, s.config);
      expect(id).toBe(defaultTemplateForClass(fe?.targetClass));
      const doctrine = behaviourDoctrine(s.request.unitId, s.state, s.config);
      expect(doctrine.length > 0).toBe(id !== DEFAULT_TEMPLATE_ID);
    }
  });
});
