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

  it("with every domain built, every type of scenario can be played", () => {
    expect(SCENARIO_TYPES.filter(playable)).toEqual([...SCENARIO_TYPES]);
  });

  it("a Skirmish loads as the plan sets it: land simulated, drones and fires abstracted, the rest off", () => {
    const { scope, errors } = effectiveScope(scopeFor("skirmish"));
    expect(errors).toEqual([]);
    for (const d of DOMAINS) expect(scope.domains[d], d).toBe(d === "land" ? "simulated" : d === "drones" || d === "fires" ? "abstracted" : "off");
  });

  it("refuses, with reasons, a scope the engine cannot run; turns off an abstraction it does not have, saying so", () => {
    expect(effectiveScope({ ...scopeFor("skirmish"), domains: { ...scopeFor("skirmish").domains, land: "off" } }).errors.join(" ")).toMatch(
      /Land cannot be off/,
    );
    expect(effectiveScope({ ...scopeFor("skirmish"), durationMin: 30_000 }).errors.join(" ")).toMatch(/5–20160 minutes/);
    // Air defence is simulated or off: there is no abstracted air defence.
    const abstracted = effectiveScope({ ...scopeFor("skirmish"), domains: { ...scopeFor("skirmish").domains, airDefence: "abstracted" } });
    expect(abstracted.errors).toEqual([]);
    expect(abstracted.scope.domains.airDefence).toBe("off");
    expect(abstracted.notes.join(" ")).toMatch(/Air defence/);
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
    expect(() => configFromSpec(spec({ scope: { ...scopeFor("skirmish"), domains: { ...scopeFor("skirmish").domains, land: "off" } } }))).toThrow(/cannot be played yet/);
  });
});
