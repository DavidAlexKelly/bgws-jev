import { describe, expect, it } from "vitest";
import { placedFromList } from "../lib/forceBuilder";
import { SYMMETRIC_CONTROL_V1 } from "../rules/forceList";
import type { Template } from "./behaviour/types";
import { parseScenario, referencedTemplates, scenarioFileName, SCENARIO_FORMAT, SCENARIO_VERSION, type RealtimeScenario } from "./scenario";
import { scopeFor } from "./scope";

const standFast: Template = { id: "custom/stand-fast", version: 1, parent: "core/default", remove: ["pullBack"] };
const forces = placedFromList(SYMMETRIC_CONTROL_V1);

function scenario(over: Partial<RealtimeScenario> = {}): RealtimeScenario {
  return {
    format: SCENARIO_FORMAT,
    version: SCENARIO_VERSION,
    name: "Meeting engagement",
    savedAt: "2026-10-21T12:00:00.000Z",
    seed: "1",
    ground: { choice: "raster+relief", seed: "baltic-v1" },
    scope: { ...scopeFor("skirmish"), durationMin: 75 },
    rules: { comms: "radio", breakpointPct: 50 },
    command: { useCommander: true, model: "any", useJev: true, directives: { blue: "", red: "" } },
    forces,
    behaviours: { [forces[0].id]: standFast.id },
    behaviourTemplates: [standFast],
    ...over,
  };
}

describe("real-time scenario files", () => {
  it("reads back what was saved", () => {
    const saved = JSON.parse(JSON.stringify(scenario()));
    const { scenario: read, errors } = parseScenario(saved);
    expect(errors).toEqual([]);
    expect(read).toEqual(saved);
    expect(read?.forces.length).toBeGreaterThan(1);
  });

  it("refuses other files and versions outright", () => {
    expect(parseScenario([]).errors[0]).toMatch(/JSON object/);
    expect(parseScenario({ format: "something" }).errors[0]).toMatch(/Not a real-time scenario/);
    expect(parseScenario({ ...scenario(), version: 4 }).errors[0]).toMatch(/version 4 is not supported/);
  });

  it("lists every problem at once", () => {
    const bad = {
      ...scenario(),
      rules: { comms: "smoke signals", breakpointPct: 5 },
      forces: [{ ...forces[0], side: "green", position: { lat: 999, lng: 0 } }, forces[0]],
      behaviours: { ghost: "core/default", [forces[0].id]: "custom/missing" },
      behaviourTemplates: [],
    };
    const { scenario: read, errors } = parseScenario(bad);
    expect(read).toBeUndefined();
    const text = errors.join("\n");
    for (const expected of [/rules.comms/, /breakpointPct/, /"side"/, /"position"/, /duplicate unit id/, /"ghost" is not one of the forces/, /unknown template "custom\/missing"/])
      expect(text).toMatch(expected);
  });

  it("reads a version 1 file as a Skirmish, its time limit as the duration", () => {
    const { scope, ...v2 } = scenario();
    void scope;
    const v1 = { ...v2, version: 1, rules: { comms: "radio", breakpointPct: 50, timeLimitMin: 45 } };
    const { scenario: read, errors } = parseScenario(JSON.parse(JSON.stringify(v1)));
    expect(errors).toEqual([]);
    expect(read?.version).toBe(SCENARIO_VERSION);
    expect(read?.scope).toEqual({ ...scopeFor("skirmish"), durationMin: 45 });
    expect(read?.rules).toEqual({ comms: "radio", breakpointPct: 50 });
  });

  it("checks the scope, and refuses one the engine cannot run yet", () => {
    const broken = parseScenario({ ...scenario(), scope: { ...scopeFor("skirmish"), durationMin: -1, domains: { land: "maybe" } } });
    expect(broken.errors.join("\n")).toMatch(/durationMin/);
    expect(broken.errors.join("\n")).toMatch(/scope.domains.land/);
    const landless = parseScenario({ ...scenario(), scope: { ...scopeFor("skirmish"), domains: { ...scopeFor("skirmish").domains, land: "off" } } });
    expect(landless.scenario).toBeUndefined();
    expect(landless.errors.join("\n")).toMatch(/scope: Land cannot be off/);
  });

  it("rejects an invalid bundled template", () => {
    const broken: Template = { ...standFast, remove: ["keep"] }; // removes fallbacks
    expect(parseScenario({ ...scenario(), behaviourTemplates: [broken] }).errors.join("\n")).toMatch(/templates in the file are invalid/);
  });

  it("bundles only the templates in use, with their custom parents", () => {
    const child: Template = { id: "custom/child", version: 1, parent: standFast.id };
    const unused: Template = { id: "custom/unused", version: 1, parent: "core/default" };
    expect(referencedTemplates({ a: child.id, b: "core/armour" }, [standFast, child, unused]).map((t) => t.id)).toEqual([
      "custom/child",
      "custom/stand-fast",
    ]);
  });

  it("names files after the scenario", () => {
    expect(scenarioFileName("Meeting engagement: Narva!")).toBe("meeting-engagement-narva.bgws-scenario.json");
    expect(scenarioFileName("  ")).toBe("scenario.bgws-scenario.json");
  });
});
