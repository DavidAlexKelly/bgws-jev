// Level of detail (§4.2): a formation far from the enemy moves as one under
// the rules, and comes apart into its units as it closes.
import { describe, expect, it } from "vitest";
import { distanceM } from "./geo";
import { rulesOnly } from "./lod";
import { at, kitGame, runKit } from "./testkit";

describe("aggregation (§4.2)", () => {
  it("aggregates away from contact, keeps the formation's shape on the move, and disaggregates near the enemy", async () => {
    const game = kitGame({
      seed: "lod",
      units: [
        { id: "b-hq", side: "blue", platform: "generic/cp-battalion", count: 3, east: 0, north: -1000, commandRating: 3 },
        { id: "b-1", side: "blue", platform: "generic/mbt", count: 4, east: -800, north: 0, order: { kind: "move", to: at(-800, 25_000), mode: "march" } },
        { id: "b-2", side: "blue", platform: "generic/ifv", count: 3, east: 800, north: 0, order: { kind: "hold" } },
        { id: "r-1", side: "red", platform: "generic/infantrySection", count: 3, east: 0, north: 30_000, order: { kind: "hold" } },
      ],
      hqs: [{ id: "blue-bn", side: "blue", echelon: "battalion", postId: "b-hq", subordinates: ["b-1", "b-2"], seat: "heuristic" }],
      scope: { detail: { aggregateAwayFromContact: true, jev: "nearContact" } },
      maxDurationS: 6 * 3600,
    });
    const early = await runKit(game, 120);
    expect(early.state.world!.land["b-1"].lod).toBe("aggregated");
    expect(rulesOnly(early.state, "b-2")).toBe(true);
    // The member follows the lead, keeping its place 1.6 km to the east.
    const mid = await runKit(game, 3600);
    const lead = mid.state.game.forceElements["b-1"].position;
    const member = mid.state.game.forceElements["b-2"].position;
    expect(distanceM(member, at(800, 0))).toBeGreaterThan(3000);
    expect(Math.abs(distanceM(lead, member) - 1600)).toBeLessThan(800);
    // Close to the enemy: every unit its own again, and Jev's to lead.
    const feed: string[] = [];
    const late = await runKit(game, 3 * 3600);
    for (const l of late.state.world!.log) feed.push(l.kind);
    expect(late.state.world!.land["b-1"].lod).not.toBe("aggregated");
    expect(feed).toContain("disaggregate");
  }, 60_000);
});
