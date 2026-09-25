/**
 * Testing the MEASURING INSTRUMENT, not the model.
 *
 * A trial of a language model cannot be a unit test — it needs a network, a
 * token and a few pounds of inference. But everything around the model can
 * be, and must be, because the trial's whole value is telling three things
 * apart that look identical from outside:
 *
 *   the model played and lost
 *   the model asked for things that were not on the menu
 *   the model never answered
 *
 * Every one of those ends with the challenger losing. If the harness cannot
 * distinguish them it will report the third as the first, and "the bot beat
 * the model" will go in a slide deck when the truth was a 403.
 *
 * The model call is injected, so each of the three is reproduced here with a
 * stub and asserted on.
 */

import { describe, expect, it } from "vitest";

import { scenarioFactory } from "../lib/forceBuilder";
import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import type { Side } from "../lib/state";
import { describeTrial, runTrial, type TrialOptions } from "./commanderTrial";
import { SYMMETRIC_CONTROL_V1 } from "./forceList";
import { llmCommander, type ModelCall } from "./llmCommander";
import { heuristicOrdersCommander } from "./orders";
import { HOUSE_V1 } from "./ruleset";

const terrain = proceduralTerrain(STANDARD_GROUND);

function trialWith(call: ModelCall, seeds = ["t0", "t1"]): TrialOptions {
  return {
    ruleset: HOUSE_V1,
    terrain,
    scenario: scenarioFactory(SYMMETRIC_CONTROL_V1, HOUSE_V1),
    seeds,
    maxTurns: 12,
    challenger: (side: Side) => llmCommander({ side, call, name: `stub-${side}` }),
    baseline: (side: Side) => heuristicOrdersCommander(side),
  };
}

/**
 * A stub that plays legally: it takes the first option offered to each
 * element, which it can do because the prompt lists them by id.
 */
const playsLegally: ModelCall = async (prompt) => {
  const ids = [...prompt.matchAll(/^ {2}(\S+:\S+)\s{2}/gm)].map((match) => match[1]);
  const byElement = new Map<string, string>();
  for (const id of ids) {
    const actorId = id.split(":")[0];
    if (!byElement.has(actorId)) byElement.set(actorId, id);
  }
  return JSON.stringify({
    plan: "stub: first legal option for each element",
    orders: [...byElement].map(([actorId, optionId]) => ({ actorId, optionId })),
  });
};

/** A stub that asks for things that were never offered. */
const asksForNonsense: ModelCall = async () =>
  JSON.stringify({
    plan: "stub: illegal orders",
    orders: [
      { actorId: "fe-1", optionId: "fe-1:teleport:everywhere" },
      { actorId: "fe-2", optionId: "fe-2:fire:the-moon" },
    ],
  });

/** A stub that is simply not there. */
const unreachable: ModelCall = async () => {
  throw new Error("403 no access to the commander query");
};

/** A stub that answers with prose instead of JSON. */
const babbles: ModelCall = async () => "I would advance on the left, I think.";

describe("the trial plays a fair fight", () => {
  it("plays every seed in both orientations", async () => {
    // Blue and red are not symmetric in any force list except the control,
    // and this codebase has already been caught by a side bias it did not
    // know about. One orientation would measure the force list.
    const result = await runTrial(trialWith(playsLegally, ["a", "b", "c"]));

    expect(result.games.length).toBe(6);
    expect(result.games.filter((game) => game.challengerSide === "blue").length).toBe(3);
    expect(result.games.filter((game) => game.challengerSide === "red").length).toBe(3);
  }, 60_000);

  it("accounts for every game as a win, a loss or a draw", async () => {
    const result = await runTrial(trialWith(playsLegally));
    expect(result.challengerWins + result.baselineWins + result.draws).toBe(result.games.length);
  }, 60_000);
});

describe("the three ways to lose are told apart", () => {
  it("a commander that plays legally records no failures and few rejections", async () => {
    const result = await runTrial(trialWith(playsLegally));

    expect(result.failedTurns).toBe(0);
    expect(result.ordersIssued).toBeGreaterThan(0);
    // It picks the first option offered, which is always legal by
    // construction, so nothing should be refused.
    expect(result.ordersRejected).toBe(0);
  }, 60_000);

  it("an unreachable model is counted as unanswered, NOT as bad play", async () => {
    // The distinction the whole file exists for. Without it a 403 reads as a
    // cautious general who held position every turn and lost narrowly.
    const result = await runTrial(trialWith(unreachable));

    expect(result.failedTurns).toBe(result.totalTurns);
    expect(result.ordersIssued).toBe(0);
    expect(result.games[0].firstFailure).toContain("unreachable");
  }, 60_000);

  it("an unreadable reply is counted as unanswered too", async () => {
    const result = await runTrial(trialWith(babbles));

    expect(result.failedTurns).toBe(result.totalTurns);
    expect(result.games[0].firstFailure).toContain("unreadable");
  }, 60_000);

  it("illegal orders are counted as rejections, and the model still played", async () => {
    // This one DID answer. It answered wrongly, which is a prompt problem and
    // a different fix from an outage.
    const result = await runTrial(trialWith(asksForNonsense));

    expect(result.failedTurns).toBe(0);
    expect(result.ordersIssued).toBeGreaterThan(0);
    expect(result.ordersRejected).toBe(result.ordersIssued);
  }, 60_000);
});

describe("the verdict refuses to be quotable when it should not be", () => {
  it("says NO VERDICT when the challenger mostly did not answer", async () => {
    const result = await runTrial(trialWith(unreachable));
    expect(describeTrial(result, "stub")).toContain("NO VERDICT");
  }, 60_000);

  it("warns when most orders were not legal options", async () => {
    const result = await runTrial(trialWith(asksForNonsense));
    expect(describeTrial(result, "stub")).toContain("TREAT WITH CARE");
  }, 60_000);

  it("says neither when the commander answered legally", async () => {
    const summary = describeTrial(await runTrial(trialWith(playsLegally)), "stub");
    expect(summary).not.toContain("NO VERDICT");
    expect(summary).not.toContain("TREAT WITH CARE");
  }, 60_000);
});
