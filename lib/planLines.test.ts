import { describe, expect, it } from "vitest";

import { createRng } from "../rules/dice";
import { EventLog } from "../rules/events";
import { MEETING_ENGAGEMENT_V1 } from "../rules/forceList";
import {
  heuristicOrdersCommander,
  planOrdersTurn,
  type OrdersCommander,
  type PlannedTurn,
} from "../rules/orders";
import { HOUSE_V1, withModules } from "../rules/ruleset";
import { scenarioFactory } from "./forceBuilder";
import { describePlan, planGeometryFor, unorderedIn } from "./planLines";
import { proceduralTerrain, STANDARD_GROUND } from "./proceduralTerrain";

const terrain = proceduralTerrain(STANDARD_GROUND);

function config(commanders: Record<"blue" | "red", OrdersCommander>) {
  return {
    ruleset: HOUSE_V1,
    terrain,
    commanders,
    rng: createRng("plan-lines-test"),
    log: new EventLog(),
    maxTurns: 40,
  };
}

const setup = () => scenarioFactory(MEETING_ENGAGEMENT_V1, HOUSE_V1)();

const both = () => ({
  blue: heuristicOrdersCommander("blue"),
  red: heuristicOrdersCommander("red"),
});

async function planned(): Promise<PlannedTurn> {
  return planOrdersTurn(setup(), config(both()));
}

describe("orders that were refused", () => {
  // The command cap is the only thing in the game that makes an element sit
  // out a turn it was chosen for, so this is the one case where "nothing is
  // drawn here" has to mean something other than "nothing was planned".
  const capped = withModules(HOUSE_V1, { commandActivations: true });

  async function cappedPlan(): Promise<PlannedTurn> {
    const cfg = { ...config(both()), ruleset: capped };
    return planOrdersTurn(scenarioFactory(MEETING_ENGAGEMENT_V1, capped)(), cfg);
  }

  it("refuses nothing once the commander budgets for itself", async () => {
    // ⚠ THE BUG THIS PAIR OF TESTS EXISTS FOR. The heuristic commander used
    // to order every element it was offered and let `validateOrders` truncate
    // to the budget — so four of seven orders a turn came back "beyond this
    // turn's command capacity", and which four was decided by the order
    // `Object.entries` happened to return.
    const plan = await cappedPlan();
    for (const side of ["blue", "red"] as const) {
      expect(plan.accepted[side].length).toBeLessThanOrEqual(
        plan.requests[side].activationBudget,
      );
      expect(
        plan.rejected[side].filter(
          (r) => r.reason === "beyond this turn's command capacity",
        ),
      ).toEqual([]);
    }
  }, 40_000);

  it("accounts for every element it was offered", async () => {
    // ⚠ THE INVARIANT THE WHOLE OVERLAY IS FOR, AND IT DID NOT HOLD.
    //
    // Reported from a real game: "still only seeing 2 ordered on each side
    // even though there is 4". There were no refusals — the model had simply
    // committed two elements and left two in hand, saying so in its plan text
    // ("fe-1 and fe-2 hold pending contact reports"). Those two got no line,
    // no badge and no mention, so a deliberate decision to keep force in hand
    // was drawn exactly like an oversight.
    //
    // Every element that was offered a decision must now appear as SOMETHING:
    // a line, a hold, or an explicit uncommitted marker. Silence is the one
    // thing the map may not do.
    const plan = await cappedPlan();
    for (const side of ["blue", "red"] as const) {
      const geometry = planGeometryFor(plan, side);
      const shown = new Set([
        ...geometry.lines.map((line) => line.actorId),
        ...geometry.badges.map((badge) => badge.actorId),
      ]);
      const offered = Object.keys(plan.requests[side].optionsByElement);
      expect(offered.length).toBeGreaterThan(plan.requests[side].activationBudget);
      for (const actorId of offered) {
        expect(shown.has(actorId), `${side} ${actorId} is invisible`).toBe(true);
      }
      // And the uncommitted ones are exactly those with no line.
      const lined = new Set(geometry.lines.map((line) => line.actorId));
      for (const badge of geometry.badges.filter((b) => b.kind === "uncommitted")) {
        expect(lined.has(badge.actorId)).toBe(false);
      }
    }
  }, 40_000);

  it("draws a refusal as a ghost rather than as nothing", async () => {
    // Synthesised rather than provoked: the commander above no longer
    // overruns, which is the point of it, so the only honest way to test the
    // drawing is to hand it a refusal directly.
    const plan = await cappedPlan();
    const victim = plan.accepted.blue[0];
    expect(victim).toBeDefined();
    const withRefusal: PlannedTurn = {
      ...plan,
      accepted: { ...plan.accepted, blue: plan.accepted.blue.slice(1) },
      rejected: {
        ...plan.rejected,
        blue: [
          ...plan.rejected.blue,
          { intent: victim, reason: "beyond this turn's command capacity" },
        ],
      },
    };

    const geometry = planGeometryFor(withRefusal, "blue");
    const ghost = geometry.lines.find((line) => line.actorId === victim.actorId);
    expect(ghost).toBeDefined();
    expect(ghost?.refused).toBe(true);
    expect(ghost?.refusedReason).toBe("beyond this turn's command capacity");
    // And the ones that were accepted are not marked.
    for (const line of geometry.lines.filter((l) => l.actorId !== victim.actorId)) {
      expect(line.refused).toBeFalsy();
    }
  }, 40_000);
});

describe("a plan, as geometry", () => {
  it("draws something for a turn that was ordered", async () => {
    const geometry = planGeometryFor(await planned(), "both");
    expect(geometry.lines.length).toBeGreaterThan(0);
  }, 40_000);

  it("starts every line where the element actually stands", async () => {
    // A line that began anywhere else would be drawing a plan for a unit that
    // is not there — the commonest way a plan overlay goes quietly wrong.
    const plan = await planned();
    for (const line of planGeometryFor(plan, "both").lines) {
      const actor = plan.state.forceElements[line.actorId];
      expect(actor).toBeDefined();
      expect(line.points[0]).toEqual(actor.position);
      expect(line.points.length).toBeGreaterThanOrEqual(2);
    }
  }, 40_000);

  it("only draws lines for orders that were accepted", async () => {
    // A rejected order is not going to happen. Drawing it would show a plan
    // the engine has already declined to honour.
    const plan = await planned();
    const geometry = planGeometryFor(plan, "both");
    const acceptedIds = new Set(
      (["blue", "red"] as const).flatMap((side) =>
        plan.accepted[side].map((intent) => intent.optionId),
      ),
    );
    for (const line of geometry.lines) expect(acceptedIds.has(line.optionId)).toBe(true);
    // Only `hold` badges trace to an option. An `uncommitted` badge is the
    // absence of one, so demanding an accepted id for it would be demanding
    // an order that by definition was never given.
    for (const badge of geometry.badges) {
      if (badge.kind !== "hold") {
        expect(badge.optionId).toBeUndefined();
        continue;
      }
      expect(acceptedIds.has(badge.optionId!)).toBe(true);
    }
  }, 40_000);

  it("gives a held element a badge rather than a line", async () => {
    // Silently dropping a hold would make an element that was deliberately
    // stood still indistinguishable from one that was never ordered.
    const plan = await planned();
    const geometry = planGeometryFor(plan, "both");
    for (const badge of geometry.badges) {
      expect(geometry.lines.some((line) => line.optionId === badge.optionId)).toBe(false);
    }
  }, 40_000);
});

describe("fog of war applies to plans", () => {
  it("shows only its own side's orders to a single viewpoint", async () => {
    // ⚠ THE LOAD-BEARING CASE. Both sides are asked for orders before either
    // is executed precisely so that neither has the other's intentions. An
    // overlay that drew both would hand them over for free.
    const plan = await planned();

    const blue = planGeometryFor(plan, "blue");
    expect(blue.lines.length).toBeGreaterThan(0);
    expect(blue.lines.every((line) => line.side === "blue")).toBe(true);
    expect(blue.badges.every((badge) => badge.side === "blue")).toBe(true);

    const red = planGeometryFor(plan, "red");
    expect(red.lines.every((line) => line.side === "red")).toBe(true);
  }, 40_000);

  it("shows both sides to the umpire", async () => {
    const plan = await planned();
    const umpire = planGeometryFor(plan, "both");
    const sides = new Set(umpire.lines.map((line) => line.side));
    expect(sides.has("blue")).toBe(true);
    expect(sides.has("red")).toBe(true);
  }, 40_000);

  it("never leaks an enemy element id into a side's geometry", async () => {
    const plan = await planned();
    for (const line of planGeometryFor(plan, "blue").lines) {
      expect(plan.state.forceElements[line.actorId].side).toBe("blue");
    }
  }, 40_000);
});

describe("describing a plan in one line", () => {
  it("counts what was ordered and what was refused", async () => {
    const plan = await planned();
    expect(describePlan(plan, "blue")).toMatch(/\d+ ordered/);
  }, 40_000);

  it("reports a commander that never answered, rather than a count", async () => {
    // A trial has to separate "played badly" from "never answered", and so
    // does the screen — a model that failed must not read as one that held.
    const plan = await planned();
    const failed: PlannedTurn = {
      ...plan,
      orders: {
        ...plan.orders,
        blue: { side: "blue", intents: [], failure: "unreachable: 403" },
      },
      accepted: { ...plan.accepted, blue: [] },
      rejected: { ...plan.rejected, blue: [] },
    };
    expect(describePlan(failed, "blue")).toMatch(/unreachable/);
  }, 40_000);
});

describe("elements nobody mentioned", () => {
  it("lists those offered a decision and given no order", async () => {
    const plan = await planned();
    const silent: PlannedTurn = {
      ...plan,
      accepted: { ...plan.accepted, blue: [] },
    };
    const unordered = unorderedIn(silent, silent.state, "blue");
    expect(unordered.length).toBeGreaterThan(0);
    // Everything listed had something it could have been told to do.
    for (const actorId of unordered) {
      expect(plan.requests.blue.optionsByElement[actorId]).toBeDefined();
    }
  }, 40_000);

  it("is empty when every offered element was ordered", async () => {
    const plan = await planned();
    const ordered = new Set(plan.accepted.blue.map((intent) => intent.actorId));
    const offered = Object.keys(plan.requests.blue.optionsByElement);
    // The heuristic orders everything it is offered, so this is the normal case.
    if (offered.every((actorId) => ordered.has(actorId))) {
      expect(unorderedIn(plan, plan.state, "blue")).toEqual([]);
    }
  }, 40_000);
});
