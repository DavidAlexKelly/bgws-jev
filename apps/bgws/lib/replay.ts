// ── bgws/lib/replay.ts ─────────────────────────────────────────────────────
// A game, recorded turn by turn, so it can be watched.
//
// `runGame` plays to a conclusion and returns the final state. That is what
// the harness wants — it is running thousands of games and only cares who
// won. It is useless for watching one: by the time it returns, every
// intermediate position is gone.
//
// This runs the same loop and keeps a SNAPSHOT after every turn, with the
// events that happened during it. A scrubber can then step through the
// engagement, and the map can show where everything actually was.
//
// The rules are untouched. This calls the same `runTurn` under the same
// ruleset with the same seeded generators, so a recorded game and a harness
// game from the same seed are the same game. If they ever diverge, the replay
// is lying and this file is the bug.
//
// Pure: no map, no React, no Foundry. Testable.

import type { GameState, Side } from "./state";
import { forceElementsOf } from "./state";
import type { TerrainSampler } from "./lineOfSight";
import type { Commander } from "../rules/commander";
import { heuristicCommander } from "../rules/commander";
import { createRng } from "../rules/dice";
import { EventLog, type GameEvent, type ResolutionEvent } from "../rules/events";
import type { RuleSet } from "../rules/ruleset";
import { runTurn } from "../rules/turnLoop";

export interface TurnSnapshot {
  /** 1-based. The state below is how things stood at the END of this turn. */
  turn: number;
  state: GameState;
  /** Everything logged during this turn, in order. */
  events: GameEvent[];
  /** Combat Strength remaining per side, for the casualty graph. */
  strength: Record<Side, number>;
}

export interface Replay {
  /** Before a shot is fired — snapshot 0, so the scrubber can start at setup. */
  setup: GameState;
  turns: TurnSnapshot[];
  winner: Side | null;
  reason: "annihilation" | "turnLimit";
  /** Everything needed to reproduce this exact game. */
  provenance: {
    rulesetId: string;
    scenarioId: string;
    seed: string;
    groundSeed: string;
  };
}

export interface RecordOptions {
  initial: GameState;
  ruleset: RuleSet;
  terrain: TerrainSampler;
  seed: string;
  /** Named on the replay so a viewer can see which ground was used. */
  groundSeed: string;
  maxTurns?: number;
  /** Defaults to the heuristic commander on both sides. */
  commanderFor?: (side: Side, seed: string) => Commander;
}

function strengthOf(state: GameState, side: Side): number {
  return forceElementsOf(state, side).reduce((sum, fe) => sum + fe.combatStrength, 0);
}

function aliveOn(state: GameState, side: Side): boolean {
  return forceElementsOf(state, side).some((fe) => fe.combatStrength > 0);
}

/**
 * Play a game and keep every turn.
 *
 * The victory conditions are copied from `runGame` deliberately rather than
 * shared: `runGame` checks at the TOP of its loop, which means it reports the
 * turn count BEFORE the killing turn. A viewer needs the snapshot after it.
 * Keeping the two separate and identical in effect is clearer than
 * generalising `runGame` and quietly changing what `turns` means for the
 * harness, whose reports would then not compare with last month's.
 */
export async function recordGame(options: RecordOptions): Promise<Replay> {
  const maxTurns = options.maxTurns ?? 40;
  const makeCommander =
    options.commanderFor ??
    ((side: Side, seed: string) => heuristicCommander(side, createRng(`${seed}:commander:${side}`)));

  const log = new EventLog();
  const config = {
    ruleset: options.ruleset,
    terrain: options.terrain,
    // Same stream naming as the harness, so the same seed is the same game.
    rng: createRng(`${options.seed}:dice`),
    commanders: {
      blue: makeCommander("blue", options.seed),
      red: makeCommander("red", options.seed),
    },
    log,
    maxTurns,
  };

  const setup = options.initial;
  let state = setup;
  const turns: TurnSnapshot[] = [];
  let winner: Side | null = null;
  let reason: Replay["reason"] = "turnLimit";
  let seen = 0;

  for (let turn = 1; turn <= maxTurns; turn += 1) {
    state = await runTurn(state, config);

    const all = log.all();
    turns.push({
      turn,
      state,
      events: all.slice(seen),
      strength: { blue: strengthOf(state, "blue"), red: strengthOf(state, "red") },
    });
    seen = all.length;

    const blueAlive = aliveOn(state, "blue");
    const redAlive = aliveOn(state, "red");
    if (!blueAlive || !redAlive) {
      winner = blueAlive ? "blue" : redAlive ? "red" : null;
      reason = "annihilation";
      break;
    }
  }

  if (reason === "turnLimit") {
    // A turn limit is not a draw: whoever has more combat power left has, in
    // any sense a training audience would recognise, won the engagement.
    const blue = strengthOf(state, "blue");
    const red = strengthOf(state, "red");
    winner = blue === red ? null : blue > red ? "blue" : "red";
  }

  return {
    setup,
    turns,
    winner,
    reason,
    provenance: {
      rulesetId: options.ruleset.id,
      scenarioId: setup.scenarioId,
      seed: options.seed,
      groundSeed: options.groundSeed,
    },
  };
}

/** The state at a scrubber position. 0 is setup, 1 is after turn one. */
export function stateAt(replay: Replay, index: number): GameState {
  if (index <= 0) return replay.setup;
  return replay.turns[Math.min(index, replay.turns.length) - 1].state;
}

/**
 * Narrative lines for a turn, in order.
 *
 * Resolutions carry a written narrative from the resolver; decisions do not,
 * because a list of every option a commander considered is a debugging tool
 * and would bury the engagement. `decisionsAt` exposes them separately.
 */
export function narrativeAt(replay: Replay, index: number): string[] {
  if (index <= 0 || index > replay.turns.length) return [];
  return replay.turns[index - 1].events
    .filter((event): event is ResolutionEvent => event.type === "resolution")
    // A resolver with no narrative is a resolver someone forgot to describe;
    // the line is dropped rather than rendered blank, and the resolution is
    // still in `events` for anyone debugging.
    .map((event) => event.narrative)
    .filter((line): line is string => Boolean(line));
}

/** What each side chose this turn, for the decision panel. */
export function decisionsAt(
  replay: Replay,
  index: number,
): { side: Side; summary: string; from: number; rationale?: string }[] {
  if (index <= 0 || index > replay.turns.length) return [];
  return replay.turns[index - 1].events
    .filter((event) => event.type === "decision")
    .map((event) => {
      const decision = event as Extract<GameEvent, { type: "decision" }>;
      const chosen = decision.options.find((option) => option.id === decision.chosenId);
      return {
        side: decision.side,
        summary: chosen?.summary ?? decision.chosenId,
        from: decision.options.length,
        rationale: decision.rationale,
      };
    });
}

/** Per-turn strength, for a casualty curve. Index 0 is setup. */
export function strengthSeries(replay: Replay): { blue: number; red: number }[] {
  return [
    { blue: strengthOf(replay.setup, "blue"), red: strengthOf(replay.setup, "red") },
    ...replay.turns.map((snapshot) => snapshot.strength),
  ];
}

/** A one-line result, for the header. */
export function describeResult(replay: Replay): string {
  const turns = replay.turns.length;
  const outcome =
    replay.winner === null
      ? "drawn"
      : `${replay.winner} wins${replay.reason === "turnLimit" ? " on remaining strength" : " by annihilation"}`;
  return `${outcome} after ${turns} turn${turns === 1 ? "" : "s"}`;
}
