// Quiet-period skipping: the clock runs faster only while nothing happens, and never changes the game.
import { describe, expect, it } from "vitest";
import { configFromSpec } from "../engine/configSpec";
import { InThreadRunner } from "../engine/runnerHandle";
import { recordFromScenario } from "../headless";
import { PRESET_SCENARIOS } from "../presets";
import { clockSpeed, quietPeriod } from "./quiet";

describe("quiet periods", () => {
  it("is quiet before contact, not once the fighting starts; the speed follows", async () => {
    const r = recordFromScenario(PRESET_SCENARIOS.M3());
    const runner = new InThreadRunner(r.initial, configFromSpec(r.spec));
    const skip = { maxCompression: 600, skipQuiet: true };
    if (quietPeriod(runner.state)) expect(clockSpeed(runner.state, 10, skip)).toBe(600);
    let wasLoud = false;
    for (let t = 0; t < 3600 && !wasLoud; t += 60) {
      await runner.advance(60);
      wasLoud = !quietPeriod(runner.state);
    }
    expect(wasLoud).toBe(true);
    expect(clockSpeed(runner.state, 10, skip)).toBe(10);
    expect(clockSpeed(runner.state, 300, { maxCompression: 60, skipQuiet: false })).toBe(60);
  }, 60_000);
});
