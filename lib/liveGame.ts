// ── bgws/lib/liveGame.ts ───────────────────────────────────────────────────
// A game played one turn at a time, where the next turn depends on what the
// commanders just decided.
//
// Different from lib/replay.ts on purpose, and both are needed:
//
//   replay.ts   plays all 40 turns up front and keeps snapshots. Right for a
//               bot, which answers instantly, and for reviewing a finished
//               engagement.
//   liveGame.ts computes ONE turn per call. Required the moment a commander
//               is a model: you cannot pre-compute turn 5 without knowing
//               what was ordered on turn 4, and you cannot hold a user
//               waiting for eighty model calls before showing them anything.
//
// Pure: no React, no map, no network. The model call is injected.

import type { GameState, Side } from "./state";
import { forceElementsOf } from "./state";
import type { TerrainSampler } from "./lineOfSight";
import type { RoutePlanner } from "./routePlan";
import type { TurnStep } from "../rules/turnLoop";
import {
  EventLog,
  type DecisionEvent,
  type GameEvent,
  type ResolutionEvent,
} from "../rules/events";
import type { TacticalDecider } from "../rules/tactical";
import type { Rng } from "../rules/dice";
import {
  executePlannedTurn,
  planOrdersTurn,
  type CounteractionOrders,
  type Intent,
  type Orders,
  type OrdersCommander,
  type PlannedTurn,
} from "../rules/orders";
import type { RuleSet } from "../rules/ruleset";
import { judgeVictory, type Verdict } from "../rules/victory";

export interface TurnRecord {
  turn: number;
  /** How things stood at the END of this turn. */
  state: GameState;
  /** What each side intended, before validation. */
  orders: Record<Side, Orders>;
  /** Orders that were dropped, and why. Shown, not hidden — see below. */
  rejected: Record<Side, { intent: Intent; reason: string }[]>;
  /** What each side did with the Counteraction Round, where it was played. */
  counteraction?: Record<Side, CounteractionOrders>;
  narrative: string[];
  /**
   * Every decision taken during the turn, in order — including the ones made
   * at the moment by a TacticalDecider, which appear nowhere else. Carried so
   * the play screen can show WHY an element held fire, not just that it did.
   */
  decisions: DecisionEvent[];
  strength: Record<Side, number>;
  /**
   * Every moment inside the turn, in order, each with the state it left behind.
   *
   * This is what makes a turn playable back rather than merely reported: the
   * narrative says what happened, and these say what the board looked like
   * while it was happening. Always ends with the turn's own end state, so
   * scrubbing to the end and stepping to the next turn agree.
   */
  steps: TurnStep[];
}

/**
 * A turn whose orders have been generated but not yet fought.
 *
 * Holds the two bookkeeping values the turn record needs, which used to be
 * locals inside `advanceTurn` and now have to survive the gap between the two
 * button presses:
 *
 *   steps    the moments recorded while PLANNING (rally, initiative,
 *            sighting). Without them the scrubber would start at the first
 *            shot and the turn would appear to begin mid-fight.
 *   logFrom  how long the event log was before planning began, so the turn's
 *            own events can still be sliced out of it afterwards.
 */
export interface PendingTurn {
  planned: PlannedTurn;
  steps: TurnStep[];
  logFrom: number;
}

export interface LiveGame {
  setup: GameState;
  current: GameState;
  turns: TurnRecord[];
  /**
   * Orders generated and awaiting execution, or null between turns.
   *
   * This is what makes the play screen's two-press turn possible: "generate
   * orders" fills it, the map draws it, "execute orders" consumes it.
   */
  pending: PendingTurn | null;
  /**
   * Where the engagement stands, by the victory rule rather than by body count.
   *
   * Present while the game is running as well as at the end: "blue holds its
   * objective and red is still a force" is the most useful single sentence on
   * the screen, and it used to be unavailable until somebody was wiped out.
   */
  verdict?: Verdict;
  /** Null while the game is still running. */
  winner: Side | null;
  over: boolean;
  reason: "annihilation" | "turnLimit" | null;
}

export interface LiveGameConfig {
  ruleset: RuleSet;
  terrain: TerrainSampler;
  /**
   * How a long march is planned. Absent means the offline bearing planner.
   *
   * Carried through rather than dropped because the browser has the terrain
   * raster's A* and the harness does not — and the live game is the browser's
   * entry point, so this is the one place the real router can get in.
   */
  routePlanner?: RoutePlanner;
  /** Optional listener for live progress; the turn record keeps its own copy. */
  onStep?: (step: TurnStep) => void;
  commanders: Record<Side, OrdersCommander>;
  /** Per-side in-the-moment decisions. Absent means the declared rules. */
  tactical?: Partial<Record<Side, TacticalDecider>>;
  rng: Rng;
  log: EventLog;
  maxTurns: number;
}

function strengthOf(state: GameState, side: Side): number {
  return forceElementsOf(state, side).reduce((sum, fe) => sum + fe.combatStrength, 0);
}

export function startGame(setup: GameState): LiveGame {
  return {
    setup,
    current: setup,
    turns: [],
    pending: null,
    winner: null,
    over: false,
    reason: null,
  };
}

/**
 * The board to put on screen right now.
 *
 * ⚠ NOT ALWAYS `game.current`, and getting this wrong is subtle.
 *
 * While orders are pending, `current` is still the state at the START of the
 * turn — before rally and before sighting. But the orders were planned
 * against the post-sighting board, so drawing `current` would show a plan
 * that references contacts not yet on the map, and units still marked broken
 * that have in fact rallied. The planned state is the honest one.
 */
export function displayState(game: LiveGame): GameState {
  return game.pending?.planned.state ?? game.current;
}

/**
 * Play exactly one turn.
 *
 * Returns a NEW LiveGame rather than mutating, so a caller holding the
 * previous value still has a valid board to render while the next turn is
 * being decided — which matters when deciding takes a model call and several
 * seconds.
 */
export async function advanceTurn(
  game: LiveGame,
  config: LiveGameConfig,
): Promise<LiveGame> {
  return executePending(await planNextTurn(game, config), config);
}

/**
 * Ask both commanders for this turn's orders, and stop there.
 *
 * Costs the model calls; changes nothing on the board. Idempotent while a
 * plan is already pending — pressing "generate orders" twice must not buy a
 * second opinion and throw the first away, because the first one is what is
 * currently drawn on the map.
 */
export async function planNextTurn(
  game: LiveGame,
  config: LiveGameConfig,
): Promise<LiveGame> {
  if (game.over || game.pending) return game;

  const logFrom = config.log.all().length;
  // Collected here rather than in the engine: the rules must not hold a
  // growing list of past states, and the caller is the only one that knows how
  // long it wants to keep them.
  const steps: TurnStep[] = [];
  const planned = await planOrdersTurn(game.current, {
    ...config,
    onStep: (step) => {
      steps.push(step);
      config.onStep?.(step);
    },
  });

  return { ...game, pending: { planned, steps, logFrom } };
}

/**
 * Throw a planned turn away, and un-write what planning wrote.
 *
 * ⚠ PLANNING IS NOT FREE OF SIDE EFFECTS, WHICH IS WHY THIS EXISTS.
 *
 * `planNextTurn` resolves rally, initiative and sighting before either
 * commander is asked — they decide what is on offer — and all three append to
 * the event log. Simply setting `pending` to null left those entries behind,
 * so a discarded-and-regenerated turn logged "Turn N begins" twice and an
 * initiative roll that never governed anything. The board was always correct;
 * the log was not, and the log is what a reader trusts afterwards.
 *
 * ⚠ THE DICE ARE NOT REWOUND, DELIBERATELY. The rng has advanced and stays
 * advanced, so re-planning gives a genuinely different initiative rather than
 * the same one again. That is the point of discarding: against a heuristic
 * commander and an unchanged board, rewinding the dice too would hand back
 * the identical plan and the button would appear to do nothing.
 */
export function discardPending(game: LiveGame, config: LiveGameConfig): LiveGame {
  if (!game.pending) return game;
  config.log.rewindTo(game.pending.logFrom);
  return { ...game, pending: null };
}

/**
 * Fight the pending turn.
 *
 * A no-op with nothing pending, rather than an error: the play screen's
 * button is disabled in that state, and a double-click that slipped through
 * should do nothing rather than throw away a turn.
 */
export async function executePending(
  game: LiveGame,
  config: LiveGameConfig,
): Promise<LiveGame> {
  if (game.over || !game.pending) return game;

  const { planned, logFrom } = game.pending;
  // Seeded with the steps recorded while planning, so a played-back turn
  // begins where the turn began rather than at the first shot.
  const steps: TurnStep[] = [...game.pending.steps];
  const result = await executePlannedTurn(planned, {
    ...config,
    onStep: (step) => {
      steps.push(step);
      config.onStep?.(step);
    },
  });
  const events: GameEvent[] = config.log.all().slice(logFrom);

  const turn: TurnRecord = {
    turn: planned.turn,
    state: result.state,
    orders: result.orders,
    rejected: result.rejected,
    counteraction: result.counteraction,
    narrative: events
      .filter((event): event is ResolutionEvent => event.type === "resolution")
      .map((event) => event.narrative)
      .filter((line): line is string => Boolean(line)),
    decisions: events.filter((event): event is DecisionEvent => event.type === "decision"),
    strength: {
      blue: strengthOf(result.state, "blue"),
      red: strengthOf(result.state, "red"),
    },
    // The end state is a step in its own right, so the scrubber's last stop
    // and the turn record agree about where the turn left things.
    steps: [
      ...steps,
      {
        turn: planned.turn,
        phase: "cleanup" as const,
        label: "End of turn",
        state: result.state,
      },
    ],
  };

  const turns = [...game.turns, turn];
  // Judged, not counted: the ground first, then whether the enemy is still a
  // force, and only then what is left in the field. See rules/victory.ts.
  const verdict = judgeVictory(result.state, config.ruleset);
  const annihilated =
    strengthOf(result.state, "blue") <= 0 || strengthOf(result.state, "red") <= 0;

  // `pending: null` on every path — the plan has been spent. Leaving it set
  // would leave the screen offering to execute a turn that has already been
  // fought, against a board that has moved on underneath it.
  if (annihilated) {
    return {
      ...game,
      current: result.state,
      turns,
      pending: null,
      winner: verdict.winner,
      verdict,
      over: true,
      reason: "annihilation",
    };
  }

  if (turns.length >= config.maxTurns) {
    return {
      ...game,
      current: result.state,
      turns,
      pending: null,
      winner: verdict.winner,
      verdict,
      over: true,
      reason: "turnLimit",
    };
  }

  // Running games carry the standing verdict too, so the screen can show who
  // is ahead and why WHILE it is being played rather than only at the end.
  return {
    ...game,
    current: result.state,
    turns,
    pending: null,
    winner: null,
    verdict,
    over: false,
    reason: null,
  };
}

/** The latest turn, or null before the first. */
export function lastTurn(game: LiveGame): TurnRecord | null {
  return game.turns.length === 0 ? null : game.turns[game.turns.length - 1];
}

/** State at a scrubber position. 0 is setup. */
export function stateAtTurn(game: LiveGame, index: number): GameState {
  if (index <= 0) return game.setup;
  return game.turns[Math.min(index, game.turns.length) - 1].state;
}

/**
 * Every order a side gave this turn, accepted or not, as readable lines.
 *
 * REJECTED ORDERS ARE SHOWN. When a commander is a model this is the most
 * important diagnostic on the screen: an order dropped as illegal is the
 * difference between "the model played badly" and "the model asked for
 * something impossible and we silently did nothing". Hiding it would make a
 * hallucinating commander look merely passive.
 */
export function orderLines(
  turn: TurnRecord,
  side: Side,
): { text: string; accepted: boolean; reason?: string }[] {
  const rejected = turn.rejected[side];
  const rejectedIds = new Set(rejected.map((entry) => entry.intent.optionId));

  const lines = turn.orders[side].intents.map((intent) => {
    const failure = rejected.find((entry) => entry.intent.optionId === intent.optionId);
    return {
      text: intent.optionId,
      accepted: !rejectedIds.has(intent.optionId),
      reason: failure?.reason,
    };
  });

  return lines;
}
