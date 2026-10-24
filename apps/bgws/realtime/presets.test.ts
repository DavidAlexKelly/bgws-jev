// The milestone scenarios (§7) play from their files, headless, with the
// rules deciding — and each exercises what its milestone built.
import { describe, expect, it } from "vitest";
import { playRecord, recordFromScenario } from "./headless";
import { parseScenario } from "./scenario";
import { PRESET_SCENARIOS } from "./presets";

describe("milestone scenarios", () => {
  for (const [milestone, make] of Object.entries(PRESET_SCENARIOS)) {
    it(`${milestone} is a valid scenario file`, () => {
      const { errors } = parseScenario(JSON.parse(JSON.stringify(make())));
      expect(errors).toEqual([]);
    });
  }

  it("M2: the mech company's attack uses dismounts, the mortar, the minefield and its stocks", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M2());
    const result = await playRecord(record, { untilS: 3600 });
    const w = result.state.world!;
    const ids = Object.keys(result.state.game.forceElements);
    expect(ids.some((id) => id.includes("-dis")), "dismounts").toBe(true);
    expect(w.missions.filter((m) => m.state !== "refused").length, "fire missions").toBeGreaterThan(0);
    expect(Object.values(w.stats.munitions).reduce((s, m) => s + m.fired, 0), "rounds in flight").toBeGreaterThan(0);
    const fuelUsed = ["b-ifv-1", "b-ifv-2", "b-tanks"].some((id) => w.land[id].stores.fuelL < record.initial.world!.land[id].stores.fuelL);
    expect(fuelUsed, "fuel drawn down").toBe(true);
  }, 60_000);

  it("M3: the battalion plans, its companies plan to its guidance, and FRAGOs go down the nets", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M3());
    const result = await playRecord(record, { untilS: 2400 });
    const w = result.state.world!;
    expect(Object.keys(w.hqs).sort()).toEqual(["blue-a", "blue-b", "blue-bn", "blue-c", "red-hq"]);
    expect(w.log.some((l) => l.text.startsWith("blue-bn issues its plan")), "battalion plan").toBe(true);
    expect(w.messages.some((m) => m.kind === "frago" && m.from === "b-a-hq"), "company FRAGOs").toBe(true);
    expect(w.stats.reportLatencyByHq["blue-bn"]?.length ?? 0, "reports reach the battalion").toBeGreaterThan(0);
  }, 60_000);
});
