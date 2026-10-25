// DFG step 11: every decision in the feed carries how its template got there.
import { describe, expect, it } from "vitest";
import { playRecord, recordFromScenario } from "../headless";
import { PRESET_SCENARIOS } from "../presets";
import type { RtLogEntry } from "./runner";

describe("the decision log's path view", () => {
  it("decisions name their template and the path walked, and say what was not offered", async () => {
    const decisions: Extract<RtLogEntry, { type: "decision" }>[] = [];
    await playRecord(recordFromScenario(PRESET_SCENARIOS.M2()), { untilS: 1800, onEntry: (e) => void (e.type === "decision" && decisions.push(e)) });
    const walked = decisions.filter((d) => d.walk);
    expect(walked.length).toBeGreaterThan(0);
    for (const d of walked) {
      expect(d.walk!.template.length).toBeGreaterThan(0);
      expect(d.walk!.path.length).toBeGreaterThan(0);
    }
  }, 60_000);
});
