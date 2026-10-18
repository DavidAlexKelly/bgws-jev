/**
 * THE CALIBRATION GUARD. Fails when a number takes the game out of the shape
 * the rest of the numbers were tuned in.
 *
 * WHY THIS FILE EXISTS
 * --------------------
 * HOUSE_V1's fire ladder, morale table and troop-quality DRMs were calibrated
 * by sweep against games that ran about 12 turns and resolved by annihilation.
 * Several other numbers — the movement allowance, the ground's woodland and
 * wet fractions, the turn limit — change how long a game runs and therefore
 * silently change what all of the calibrated numbers mean. A game that times
 * out is a draw, and a batch of draws measures nothing.
 *
 * Every assertion here is an ENVELOPE, not a target: the bounds are wide
 * enough that a sane change passes and only a change that breaks the game's
 * shape fails. They are deterministic — fixed dice seeds, fixed ground — so a
 * failure is a real regression and never flake.
 *
 * ⚠ THIS GUARD IS ALSO A STAND-IN FOR A SWEEP THAT COULD NOT BE RUN. The
 * container the movement allowance was priced in had no node_modules and no
 * egress, so the value was pinned by argument rather than chosen by batch.
 * This is how it is held to account until `scripts/bgwsMovementTune.ts` can
 * actually be run. If you can run batches, run that instead and bring numbers.
 *
 * WHEN THIS FAILS, the fix is not to widen the envelope. It is either to
 * change the number that broke it, or — if the new shape is genuinely better —
 * to re-run the ruleset tuner (scripts/bgwsTune.ts) and move the envelope with
 * evidence attached.
 */

import { describe, expect, it } from "vitest";

import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import { scenarioFactory } from "../lib/forceBuilder";
import { FORCE_LISTS } from "./forceList";
import { moduleImpact, runBatch } from "./harness";
import { HOUSE_V1, withModules } from "./ruleset";

const terrain = proceduralTerrain(STANDARD_GROUND);
const seeds = Array.from({ length: 40 }, (_, index) => `calib${index}`);

/**
 * Three lists, not six: one all-tracked advance, one symmetric control, one
 * mixed. Enough to catch a broken tempo, cheap enough to run on every commit.
 */
const LISTS = ["advance-to-contact-v1", "symmetric-control-v1", "combined-arms-v1"] as const;

const MAX_TURNS = 40;

function batchFor(listId: (typeof LISTS)[number]) {
  const list = FORCE_LISTS[listId];
  return runBatch({
    scenario: scenarioFactory(list, HOUSE_V1),
    ruleset: HOUSE_V1,
    seeds,
    terrain,
    maxTurns: MAX_TURNS,
  });
}

describe("games still resolve", () => {
  it.each(LISTS)("%s finishes well inside the turn limit", async (listId) => {
    const result = await batchFor(listId);

    // Calibrated at ~12 turns with 1 game in 80 hitting the limit. 20 is the
    // point at which a third of the ruleset's tuning stops applying.
    expect(result.meanTurns, `${listId} mean game length`).toBeLessThan(20);

    // Tightened from 0.25 once Rally (5.2) was implemented. With a way back up
    // the morale ladder, unresolved games fell from 14 in 160 to 1 in 160
    // across four lists — so the envelope moves to hold that gain rather than
    // leaving room for it to be lost again unnoticed. Moving an envelope is
    // legitimate when evidence moves with it; widening one to make a failure
    // go away is not.
    const timedOut = result.games.filter((game) => game.reason === "turnLimit").length;
    expect(timedOut / seeds.length, `${listId} share hitting the turn limit`).toBeLessThan(0.15);

    // A draw is an unmeasurable game. A few are tolerable; many are not.
    expect(result.wins.draw / seeds.length, `${listId} share drawn`).toBeLessThan(0.1);
  }, 60_000);
});

describe("neither side is structurally doomed", () => {
  /**
   * ⚠ A SYMMETRIC FORCE LIST IS NOT A SYMMETRIC GAME, AND THIS GUARD USED TO
   * CONFUSE THE TWO.
   *
   * It asserted that symmetric-control-v1 — the same troops on both sides —
   * came out near even, and read any skew as a rules problem. It is not: both
   * sides field the same forces but they fight over DIFFERENT GROUND, and on
   * the standard ground one approach is wetter and more overlooked than the
   * other. Measured, that list runs 27/72 to red... and 54/46 the other way
   * once the deployments are swapped. The advantage follows the ground.
   *
   * So the question the guard should ask is not "is blue level with red" but
   * "does the advantage belong to the POSITION or to the COLOUR" — and the
   * only way to ask it is to play both orientations. Over three lists and both
   * orientations the current rules come out blue 294, red 298.
   *
   * The old form let a real rules bias hide behind the map, and cried wolf
   * about the map as though it were a rules bias. Both failures at once.
   */
  it("gives neither COLOUR an advantage, once the ground is controlled for", async () => {
    const list = FORCE_LISTS["symmetric-control-v1"];
    const options = { ruleset: HOUSE_V1, seeds, terrain, maxTurns: MAX_TURNS };

    const normal = await runBatch({
      ...options,
      scenario: scenarioFactory(list, HOUSE_V1),
    });
    const mirrored = await runBatch({
      ...options,
      scenario: scenarioFactory(list, HOUSE_V1, { mirrored: true }),
    });

    const blue = normal.wins.blue + mirrored.wins.blue;
    const red = normal.wins.red + mirrored.wins.red;
    const decided = blue + red;
    expect(decided).toBeGreaterThan(0);

    const blueShare = blue / decided;
    expect(blueShare, `blue ${blue} v red ${red} across both orientations`).toBeGreaterThan(
      0.35,
    );
    expect(blueShare, `blue ${blue} v red ${red} across both orientations`).toBeLessThan(0.65);
  }, 60_000);

  it("shows that the ground, not the rules, is what is lopsided", async () => {
    // The evidence for the test above, kept as a test so it stays true: the
    // same list, the same seeds, deployments swapped, and the winner changes
    // sides. If this ever stops holding, the guard above is measuring
    // something else and should not be trusted.
    const list = FORCE_LISTS["symmetric-control-v1"];
    const options = { ruleset: HOUSE_V1, seeds, terrain, maxTurns: MAX_TURNS };

    const normal = await runBatch({ ...options, scenario: scenarioFactory(list, HOUSE_V1) });
    const mirrored = await runBatch({
      ...options,
      scenario: scenarioFactory(list, HOUSE_V1, { mirrored: true }),
    });

    const blueNormal = normal.wins.blue / Math.max(1, normal.wins.blue + normal.wins.red);
    const blueMirrored =
      mirrored.wins.blue / Math.max(1, mirrored.wins.blue + mirrored.wins.red);
    expect(
      blueMirrored,
      `blue wins ${Math.round(blueNormal * 100)}% normally and ` +
        `${Math.round(blueMirrored * 100)}% mirrored`,
    ).toBeGreaterThan(blueNormal);
  }, 60_000);
});

describe("movement is priced so that the ground can bind", () => {
  it("an element cannot cross the board in a couple of bounds", () => {
    // The failure this catches: allowances taken from a route planner's march
    // speeds. At 20 km/h cross-country a tracked element covers 5 km in a
    // 15-minute turn, crosses a 10 km board in two, and arrives before the
    // 3 km sighting cap can matter — so no terrain and no bound can ever
    // constrain it. Half the board in one turn is the line.
    for (const moveType of ["F", "W", "T"] as const) {
      const open = HOUSE_V1.movement[moveType].open ?? 0;
      expect(open, `${moveType} open-ground allowance`).toBeGreaterThan(0);
      expect(open, `${moveType} open-ground allowance`).toBeLessThan(5000);
    }
  });

  it("keeps the tempo the fire and morale tables were tuned at", () => {
    // The loop's own stand-in bounds measure 629-2,033 m across the six force
    // lists, and HOUSE_V1 was calibrated with games played at that pace. The
    // tracked open-ground allowance is the closest single number to it.
    const open = HOUSE_V1.movement.T.open ?? 0;
    expect(open).toBeGreaterThanOrEqual(600);
    expect(open).toBeLessThanOrEqual(2500);
  });

  it("leaves impassable ground impassable", () => {
    // Scaling must never turn a 0 into a small positive number: "cannot" and
    // "slowly" are different rules, and a fraction that rounds one into the
    // other would quietly let trucks ford rivers.
    expect(HOUSE_V1.movement.W.marsh).toBe(0);
    expect(HOUSE_V1.movement.W.water).toBe(0);
  });

  it("still earns its place once priced", async () => {
    // The whole point. If pricing movement tactically made the rule stop
    // mattering, that would be worth knowing immediately.
    const comparison = await moduleImpact("terrainMovement", {
      scenario: scenarioFactory(FORCE_LISTS["advance-to-contact-v1"], HOUSE_V1),
      ruleset: HOUSE_V1,
      seeds,
      terrain,
      maxTurns: MAX_TURNS,
    });
    expect(comparison.decisionsChanged.length / seeds.length).toBeGreaterThan(0.5);
  }, 60_000);

  it("does not make the off arm nonsense", async () => {
    // With the module off the loop falls back to its blind 40%/25% bounds
    // rather than to no limit at all. If it fell back to "go the whole way",
    // the control arm would be a game in which everything teleports into
    // contact on turn one, and every comparison against it would be junk.
    const off = await runBatch({
      scenario: scenarioFactory(FORCE_LISTS["advance-to-contact-v1"], HOUSE_V1),
      ruleset: withModules(HOUSE_V1, { terrainMovement: false }),
      seeds,
      terrain,
      maxTurns: MAX_TURNS,
    });
    expect(off.meanTurns).toBeGreaterThan(3);
    expect(off.meanTurns).toBeLessThan(20);
  }, 60_000);
});
