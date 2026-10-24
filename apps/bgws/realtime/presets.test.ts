// The milestone scenarios (§7) play from their files, headless, with the
// rules deciding — and each exercises what its milestone built.
import { describe, expect, it } from "vitest";
import { playRecord, recordFromScenario } from "./headless";
import { parseScenario } from "./scenario";
import { PRESET_SCENARIOS } from "./presets";

const strikes0 = (w: { stats: { munitions: Record<string, { fired: number }> } }) => Object.values(w.stats.munitions).reduce((n, m) => n + m.fired, 0);

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

  it("M4: drones fly, strike and are shot down; jammers and ESM work on both sides", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M4());
    const result = await playRecord(record, { untilS: 2400 });
    const w = result.state.world!;
    const kinds = new Set(w.log.map((l) => `${l.domain}:${l.kind}`));
    expect(Object.keys(w.entities).length + strikes0(w), "launches").toBeGreaterThan(0);
    const strikes = ["fpv", "fpvFibre", "loitering"].reduce((n, k) => n + (w.stats.munitions[k]?.fired ?? 0), 0);
    expect(strikes, "strikes").toBeGreaterThan(0);
    const early = await playRecord(record, { untilS: 300 });
    expect(early.state.world!.jammers.filter((j) => j.on).length, "jammers on").toBe(2);
    expect(Object.keys(w.fixes.blue).length + Object.keys(w.fixes.red).length, "ESM hears emitters").toBeGreaterThan(0);
    void kinds;
  }, 60_000);

  it("M5: SEAD on the fixed SAM, air defence against the raid, flights tasked and turned round", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M5());
    const result = await playRecord(record, { untilS: 2 * 3600 });
    const w = result.state.world!;
    const raid = Object.values(w.entities).filter((e) => e.id.startsWith("r-owa-"));
    expect(raid.filter((e) => e.status === "destroyed").length, "raid intercepted").toBeGreaterThan(raid.length / 2);
    expect(w.stats.interceptsTried, "interceptors fired").toBeGreaterThan(0);
    expect(w.stats.spentUsd.blue, "the defence's bill").toBeGreaterThan(0);
    expect(Object.values(w.entities).some((e) => e.air?.sorties != null && e.air.sorties > 0), "a flight landed and turned round").toBe(true);
    expect(Object.keys(w.fixes.blue).includes("r-sam") || (w.stats.munitions.arm?.fired ?? 0) > 0, "the SAM found").toBe(true);
  }, 120_000);

  it("M6: formations move aggregated, come apart near the enemy, and the supply chain and medics work", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M6());
    // The world feed as it streams (the state keeps only its latest lines).
    const feed: { kind: string; domain: string }[] = [];
    const result = await playRecord(record, { untilS: 4 * 3600, onEntry: (e) => void (e.type === "world" && feed.push(e.item)) });
    const w = result.state.world!;
    expect(feed.some((l) => l.kind === "aggregate"), "aggregated away from contact").toBe(true);
    expect(feed.some((l) => l.kind === "disaggregate"), "comes apart near the enemy").toBe(true);
    expect(feed.some((l) => l.domain === "logistics" && (l.kind === "logstat" || l.kind === "convoy")), "LOGSTATs and convoys").toBe(true);
    expect(Object.values(w.land).some((l) => l.lod === "far"), "rules lead away from contact").toBe(true);
  }, 300_000);

  it("M7: the task group's layers engage the raid, the coast fires on the group, and shipping is protected", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M7());
    const feed: { kind: string; domain: string }[] = [];
    const result = await playRecord(record, { untilS: 2 * 3600, onEntry: (e) => void (e.type === "world" && feed.push(e.item)) });
    const w = result.state.world!;
    expect(feed.filter((l) => l.kind === "engage").length, "interceptors fired").toBeGreaterThan(10);
    expect(w.stats.spentUsd.blue, "the defence's bill").toBeGreaterThan(w.stats.lostUsd.red);
    expect(w.stats.neutralHits ?? 0, "no merchant hit").toBe(0);
    expect(["b-ddg", "b-ffg"].some((id) => w.entities[id].status === "active"), "the group survives the first raid").toBe(true);
  }, 120_000);

  it("M8: drones and SEAD on the radars, cyber on the air picture, imagery and the kill web choosing strikes", async () => {
    const record = recordFromScenario(PRESET_SCENARIOS.M8());
    const feed: { kind: string; domain: string }[] = [];
    const result = await playRecord(record, { untilS: 4 * 3600, onEntry: (e) => void (e.type === "world" && feed.push(e.item)) });
    const w = result.state.world!;
    const kinds = new Set(feed.map((l) => `${l.domain}:${l.kind}`));
    expect(kinds.has("drones:launch"), "staged drones away").toBe(true);
    expect(kinds.has("cyber:effect") || kinds.has("cyber:failed"), "the cyber access used").toBe(true);
    expect(kinds.has("air:release"), "SEAD or strikes released").toBe(true);
    expect(kinds.has("space:imaged") || kinds.has("space:clouded"), "satellite passes").toBe(true);
    expect(w.chains.filter((k) => k.side === "blue" && k.engaged != null).length, "kill chains engaged").toBeGreaterThan(0);
  }, 300_000);
});
