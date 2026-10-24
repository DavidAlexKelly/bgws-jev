import { describe, expect, it } from "vitest";
import { DECISION_POINTS, type DecisionPoint } from "../../engine/decisions";
import { buildRegistry, bundledTemplateFiles } from "../load";
import { resolveTemplate, walk } from "../walk";
import { vocabulary } from "../vocabulary";
import { LAND_ACTIONS, landPlan } from "./land";

describe("the default land template", () => {
  const points = Object.keys(DECISION_POINTS) as DecisionPoint[];

  it("validates against the land action library", () => {
    expect(() => buildRegistry(bundledTemplateFiles(), vocabulary())).not.toThrow();
  });

  it("runs, at every decision point, the actions that point has always used, in order", () => {
    expect(points).toHaveLength(13);
    const split: Record<DecisionPoint, string[]> = {
      D0: ["land.complyWithOrders", "land.breakOffCovered", "land.fireAndBack", "land.finishFightFirst", "land.breakContactFirst"],
      D1: ["land.carryOn", "land.engageNow", "land.waitFor", "land.betterPosition", "land.observe", "land.pullBackToFriends"],
      D2: ["land.returnFire", "land.hullDownThenFire", "land.outOfSight", "land.askFriendForHelp", "land.assault", "land.goQuietInCover"],
      D3: ["land.searchBearing", "land.coverFromBearing", "land.pullBackFromBearing", "land.carryOn"],
      D4: ["land.fireNow", "land.waitCloser", "land.letPass"],
      D5: ["land.fireFirst", "land.keepHolding", "land.relocateQuietly", "land.slipAway"],
      D6: ["land.keepFiring", "land.fireAndMove", "land.shiftFire", "land.goQuiet", "land.outOfSight"],
      D7: ["land.resumeOrCarryOn", "land.watchLastSeen", "land.regainContact", "land.shiftFireFromLost"],
      D8: ["land.betterShotOnTarget", "land.shiftFireFromTarget", "land.stopFiring", "land.outOfTargetSight", "land.keepAtIt"],
      D9: ["land.helpFriend", "land.moveToSupport", "land.carryOn"],
      D10: ["land.resumeOrders", "land.skipToNextPhase", "land.holdHere", "land.overwatchHere", "land.keepHoldingHere"],
      D11: ["land.rejoin", "land.holdAndRecover", "land.joinNearestFriends"],
      D12: ["land.complyWithRequest", "land.partlyOverwatch", "land.cannotComply"],
    };
    for (const point of points) expect(landPlan(point).map((p) => p.action)).toEqual(split[point]);
  });

  it("lets a variant drop one option at D1 — 'stand fast' removes pulling back", () => {
    const registry = buildRegistry(
      {
        ...bundledTemplateFiles(),
        "./templates/custom/stand-fast.json": { id: "custom/stand-fast", version: 1, parent: "core/default", remove: ["pullBack"] },
      },
      vocabulary(),
    );
    const plan = (id: string) => {
      const r = walk(resolveTemplate(id, registry), "D1", {}, null);
      return r.kind === "choose" ? r.options.map((o) => o.action) : [];
    };
    expect(plan("core/default")).toContain("land.pullBackToFriends");
    expect(plan("custom/stand-fast")).toEqual(plan("core/default").filter((a) => a !== "land.pullBackToFriends"));
  });

  it("names only actions that exist", () => {
    for (const point of points) for (const { action } of landPlan(point)) expect(LAND_ACTIONS).toContain(action);
  });
});
