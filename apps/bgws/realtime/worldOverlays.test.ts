// The world on the map (W13): what each picture shows, by layer.
import { describe, expect, it } from "vitest";
import { at, kitGame } from "./world/testkit";
import { WORLD_LAYERS, worldOverlay } from "./worldOverlays";

const game = () =>
  kitGame({
    units: [
      { id: "b-sam", side: "blue", platform: "generic/samMedium", count: 2, east: 0, north: 0 },
      { id: "r-x", side: "red", platform: "generic/infantrySection", count: 1, east: 0, north: 30_000 },
    ],
    entities: [
      { id: "b-jet", owner: "blue", archetype: "generic/fighter4", east: 0, north: 5000, altM: 8000, status: "active" },
      { id: "r-jet", owner: "red", archetype: "generic/fighter4", east: 0, north: 25_000, altM: 8000, status: "active" },
      { id: "b-sat", owner: "blue", archetype: "generic/satSar", east: 0, north: 0 },
    ],
    domains: { air: "simulated", airDefence: "simulated", space: "simulated" },
  });

describe("the world overlay", () => {
  it("the truth shows every side's aircraft; a side's picture shows its own and the tracks it holds", () => {
    const g = game();
    const all = new Set(WORLD_LAYERS);
    const truth = worldOverlay(g.state, { seenBy: "truth", layers: all });
    expect(truth.entities.features.map((f) => f.properties.id).sort()).toEqual(["b-jet", "b-sat", "r-jet"]);
    const blue = worldOverlay(g.state, { seenBy: "blue", layers: all });
    expect(blue.entities.features.map((f) => f.properties.id)).not.toContain("r-jet");
    g.state.world!.tracks.blue["r-jet"] = { id: "r-jet", truthId: "r-jet", at: at(0, 25_000), altM: 8000, seenAt: 0, quality: 10, classification: "class", thought: "aircraft", domain: "air", by: "b-sam", errorM: 100 };
    expect(worldOverlay(g.state, { seenBy: "blue", layers: all }).entities.features.some((f) => f.properties.id === "r-jet" && f.properties.known === "class")).toBe(true);
  });

  it("layers toggle; the air-defence ring and the satellite's ground track are drawn", () => {
    const g = game();
    const none = worldOverlay(g.state, { seenBy: "truth", layers: new Set() });
    expect(none.entities.features.length).toBe(0);
    const air = worldOverlay(g.state, { seenBy: "truth", layers: new Set(["air", "space"] as const) });
    expect(air.areas.features.some((f) => f.properties.kind === "adRing")).toBe(true);
    expect(air.lines.features.some((f) => f.properties.kind === "groundTrack")).toBe(true);
  });
});
