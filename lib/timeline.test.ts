/**
 * A turn, played back.
 *
 * The property that matters is that the timeline is RECORDED rather than
 * reconstructed. The event log carries the effects of every resolution, but
 * plenty of state changes are not resolutions — a MOVED marker, a facing, a
 * route being trimmed, clean-up wiping markers — so a playback rebuilt from
 * the log alone would drift from what actually happened, and would look right
 * while doing it.
 */

import { describe, expect, it } from "vitest";

import { proceduralTerrain, STANDARD_GROUND } from "./proceduralTerrain";
import { scenarioFactory } from "./forceBuilder";
import { advanceTurn, startGame, type LiveGame, type LiveGameConfig } from "./liveGame";
import { createRng } from "../rules/dice";
import { EventLog } from "../rules/events";
import { FORCE_LISTS } from "../rules/forceList";
import { heuristicOrdersCommander } from "../rules/orders";
import { HOUSE_V1 } from "../rules/ruleset";

const terrain = proceduralTerrain(STANDARD_GROUND);

function game(): { game: LiveGame; config: LiveGameConfig } {
  const list = FORCE_LISTS["advance-to-contact-v1"];
  return {
    game: startGame(scenarioFactory(list, HOUSE_V1)()),
    config: {
      ruleset: HOUSE_V1,
      terrain,
      commanders: {
        blue: heuristicOrdersCommander("blue"),
        red: heuristicOrdersCommander("red"),
      },
      rng: createRng("timeline"),
      log: new EventLog(),
      maxTurns: 10,
    },
  };
}

describe("the turn timeline", () => {
  it("records more than one moment in a turn", async () => {
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    const [record] = played.turns;

    expect(record.steps.length).toBeGreaterThan(1);
  });

  it("opens with the turn and closes with its end state", async () => {
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    const [record] = played.turns;

    expect(record.steps[0].label).toContain("Turn 1");
    const last = record.steps[record.steps.length - 1];
    expect(last.label).toBe("End of turn");
    // Identity, not equality: scrubbing to the end of a turn and stepping to
    // the next one must show the same board, not two boards that agree.
    expect(last.state).toBe(record.state);
  });

  it("shows the board CHANGING, not the same board repeated", async () => {
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    const [record] = played.turns;

    const distinct = new Set(record.steps.map((step) => step.state));
    expect(distinct.size).toBeGreaterThan(1);
  });

  it("labels each step with something a person can read", async () => {
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    const [record] = played.turns;

    for (const step of record.steps) {
      expect(step.label.length).toBeGreaterThan(0);
      expect(step.turn).toBe(1);
    }
    // At least one step is an element doing something, rather than a phase.
    expect(record.steps.some((step) => step.actorId !== undefined)).toBe(true);
  });

  it("attributes an action to the side that took it", async () => {
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    const [record] = played.turns;

    const sided = record.steps.filter((step) => step.side !== undefined);
    expect(sided.length).toBeGreaterThan(0);
    for (const step of sided) {
      expect(["blue", "red"]).toContain(step.side);
    }
  });

  it("keeps a timeline per turn rather than one for the game", async () => {
    const { game: start, config } = game();
    const one = await advanceTurn(start, config);
    const two = await advanceTurn(one, config);

    expect(two.turns).toHaveLength(2);
    expect(two.turns[0].steps.every((step) => step.turn === 1)).toBe(true);
    expect(two.turns[1].steps.every((step) => step.turn === 2)).toBe(true);
  });

  it("passes steps to a live listener as well as recording them", async () => {
    // The play screen wants both: a running commentary while the turn is being
    // resolved, and a timeline to scrub afterwards.
    const { game: start, config } = game();
    const seen: string[] = [];
    const played = await advanceTurn(start, {
      ...config,
      onStep: (step) => seen.push(step.label),
    });
    const [record] = played.turns;

    // The record has one more: the end state, which the engine does not emit.
    expect(seen).toEqual(record.steps.slice(0, -1).map((step) => step.label));
  });

  it("costs nothing when nobody is listening", async () => {
    // The harness plays tens of thousands of games and wants none of this, so
    // the recorder has to be genuinely optional rather than merely ignorable.
    const { game: start, config } = game();
    const played = await advanceTurn(start, config);
    expect(played.turns[0].state.turn).toBe(2);
  });
});
