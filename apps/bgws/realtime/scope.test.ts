import { describe, expect, it } from "vitest";
import { DEFAULT_ORIGIN } from "../lib/forceBuilder";
import { HOUSE_V1 } from "../rules/ruleset";
import { configFromSpec, RT_CONFIG_SPEC_VERSION, timingSpec, type RtConfigSpec } from "./engine/configSpec";
import { domainMode } from "./engine/engine";
import { DEFAULT_TIMING } from "./engine/timing";
import { DOMAINS, effectiveScope, playable, SCENARIO_TYPE_INFO, SCENARIO_TYPES, scopeErrors, scopeFor } from "./scope";

describe("scenario scope", () => {
  it("every preset is a well-formed scope", () => {
    for (const type of SCENARIO_TYPES) expect(scopeErrors(scopeFor(type)), type).toEqual([]);
  });

  it("only what the engine has built can be played: the land, drone, sustained and custom types today", () => {
    expect(SCENARIO_TYPES.filter(playable)).toEqual(["skirmish", "combinedArms", "droneLine", "sustainedOffensive", "custom"]);
  });

  it("a Skirmish loads as the plan sets it: land simulated, drones and fires abstracted, the rest off", () => {
    const { scope, errors } = effectiveScope(scopeFor("skirmish"));
    expect(errors).toEqual([]);
    for (const d of DOMAINS) expect(scope.domains[d], d).toBe(d === "land" ? "simulated" : d === "drones" || d === "fires" ? "abstracted" : "off");
  });

  it("refuses, with reasons, a scope that needs what is not built", () => {
    const { errors } = effectiveScope(scopeFor("strike"));
    expect(errors.join(" ")).toMatch(/Cyber cannot be simulated yet \(planned for M8\)/);
    expect(effectiveScope({ ...scopeFor("skirmish"), domains: { ...scopeFor("skirmish").domains, land: "off" } }).errors.join(" ")).toMatch(
      /Land cannot be off/,
    );
    expect(effectiveScope({ ...scopeFor("skirmish"), durationMin: 20_000 }).errors.join(" ")).toMatch(/5–14400 minutes/);
  });

  it("every preset names a victory and a milestone", () => {
    for (const type of SCENARIO_TYPES) {
      expect(SCENARIO_TYPE_INFO[type].victory.length).toBeGreaterThan(0);
      expect(SCENARIO_TYPE_INFO[type].milestone.length).toBeGreaterThan(0);
    }
  });
});

describe("scope in the engine's config", () => {
  const spec = (over: Partial<RtConfigSpec> = {}): RtConfigSpec => ({
    version: RT_CONFIG_SPEC_VERSION,
    rulesetId: HOUSE_V1.id,
    seed: "scope",
    timing: timingSpec(DEFAULT_TIMING),
    ground: { source: "generated", seed: "g", origin: DEFAULT_ORIGIN },
    ...over,
  });

  it("carries the runnable scope; the engine reads each domain's mode", () => {
    const config = configFromSpec(spec({ scope: scopeFor("skirmish") }));
    expect(domainMode(config, "land")).toBe("simulated");
    expect(domainMode(config, "drones")).toBe("abstracted");
    expect(domainMode(config, "naval")).toBe("off");
  });

  it("no scope is a skirmish, as every game was before", () => {
    const config = configFromSpec(spec());
    expect(config.scope).toBeUndefined();
    expect(domainMode(config, "land")).toBe("simulated");
    expect(domainMode(config, "air")).toBe("off");
  });

  it("refuses to build a config for a scope it cannot run", () => {
    expect(() => configFromSpec(spec({ scope: scopeFor("joint") }))).toThrow(/cannot be played yet/);
  });
});
