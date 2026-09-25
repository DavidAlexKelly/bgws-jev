// ── bgws/rules/harness.ts ──────────────────────────────────────────────────
// Running many games to find out whether a rule earns its place.
//
// This is the point of the platform. A wargame played once tells you a story;
// a wargame played two hundred times with one mechanic switched off tells you
// whether that mechanic was doing anything.
//
// THREE QUESTIONS IT ANSWERS
//
//   Did the rule ever FIRE?     A modifier that never applied is a rule with
//                               nothing to say. Cheapest possible finding.
//   Did it change an OUTCOME?   Same seeds, module off, different winners?
//   Did it change a CHOICE?     Did commanders decide differently? A rule can
//                               alter outcomes by luck alone; altering
//                               decisions is what makes it interesting.
//
// SEPARATE RANDOM STREAMS, AND WHY IT MATTERS HERE
// ------------------------------------------------
// Dice and commander tie-breaks draw from DIFFERENT generators. If they shared
// one, swapping the heuristic commander for a model would shift every
// subsequent die roll, and a comparison between them would be measuring the
// dice as much as the decisions. Two streams from one seed keeps a comparison
// about the thing being compared.
//
// ⚠ WHAT THESE NUMBERS ARE NOT
// Until the board runs on real terrain and real force management charts, a
// batch result is evidence about THE RULES, not about combat. "Massing wins
// more often" is a statement about the fire table; it is not a claim about
// warfare. Every report carries that caveat.

import { flatTerrain, type TerrainSampler } from "../lib/lineOfSight";
import type { GameState, Side } from "../lib/state";
import { forceElementsOf } from "../lib/state";
import type { Commander } from "./commander";
import { heuristicCommander } from "./commander";
import { createRng } from "./dice";
import { EventLog, decisionBreadth, modifierUsage } from "./events";
import type { ModuleFlags, RuleSet } from "./ruleset";
import { withModules } from "./ruleset";
import type { Verdict, VictoryLevel } from "./victory";
import { runGame } from "./turnLoop";

export type CommanderFactory = (side: Side, seed: string) => Commander;

export interface BatchOptions {
  /** A fresh starting state per game — never a shared mutable one. */
  scenario: () => GameState;
  ruleset: RuleSet;
  seeds: string[];
  terrain?: TerrainSampler;
  maxTurns?: number;
  commander?: CommanderFactory;
}

export interface GameSummary {
  seed: string;
  winner: Side | null;
  turns: number;
  reason: "annihilation" | "turnLimit";
  /** How well it was won (3.1's grades). See rules/victory.ts. */
  level: VictoryLevel;
  /** Which clause of the judgement decided it. */
  basis: Verdict["basis"];
  decisions: number;
  meanOptions: number;
  survivingStrength: Record<Side, number>;
  /** Every modifier that fired at least once, and how often. */
  modifiersFired: Record<string, number>;
  /** The sequence of chosen option ids — the game's decisions, in order. */
  decisionTrace: string[];
}

export interface BatchResult {
  rulesetId: string;
  games: GameSummary[];
  wins: Record<"blue" | "red" | "draw", number>;
  meanTurns: number;
  meanDecisionBreadth: number;
  modifierUsage: Record<string, number>;
  /** Declared in the ruleset, never once applied. Candidates for deletion. */
  neverFired: string[];
}

/**
 * The baseline, and it now keeps formation.
 *
 * Without it the baseline scattered within two turns and could never use
 * Combined Fire, so the sweep's reading of `combinedFire` was a measurement
 * of the commander rather than of the rule. See FormationPreference.
 *
 * This changes every number the harness produces, because the baseline IS the
 * measuring instrument. That is the right trade — a sharper instrument is
 * worth a renumbered report — but it means any comparison against a report
 * generated before this must be thrown away rather than reconciled.
 */
const defaultCommander =
  (ruleset: RuleSet): CommanderFactory =>
  (side, seed) =>
    heuristicCommander(
      side,
      createRng(`${seed}:commander:${side}`),
      // ⚠ ONLY WHEN MASSING IS ACTUALLY POSSIBLE.
      //
      // A commander concentrates because Combined Fire pays. With the module
      // off it buys nothing, and a baseline that huddled for no reason was
      // measurably worse: symmetric-control went from 38 to 52 games out of
      // 200 hitting the turn limit, drifting back towards the unresolved-game
      // problem house-v2 was tuned to fix.
      //
      // It also keeps the control arm of every OTHER module's comparison
      // exactly the game it was.
      ruleset.modules.combinedFire ? { coLocatedM: ruleset.coLocatedM } : undefined,
    );

/** Run one scenario over many seeds under one ruleset. */
export async function runBatch(options: BatchOptions): Promise<BatchResult> {
  const terrain = options.terrain ?? flatTerrain();
  const makeCommander = options.commander ?? defaultCommander(options.ruleset);
  const games: GameSummary[] = [];

  for (const seed of options.seeds) {
    const log = new EventLog();
    const outcome = await runGame(options.scenario(), {
      ruleset: options.ruleset,
      terrain,
      // Dice and decisions draw from different streams; see the note above.
      rng: createRng(`${seed}:dice`),
      commanders: {
        blue: makeCommander("blue", seed),
        red: makeCommander("red", seed),
      },
      log,
      maxTurns: options.maxTurns ?? 20,
    });

    const breadth = decisionBreadth(log);
    const usage = modifierUsage(log);

    games.push({
      seed,
      winner: outcome.winner,
      turns: outcome.turns,
      reason: outcome.reason,
      level: outcome.verdict.level,
      basis: outcome.verdict.basis,
      decisions: breadth.total,
      meanOptions: breadth.meanOptions,
      survivingStrength: {
        blue: strengthOf(outcome.state, "blue"),
        red: strengthOf(outcome.state, "red"),
      },
      modifiersFired: Object.fromEntries(
        Object.entries(usage).map(([name, entry]) => [name, entry.count]),
      ),
      decisionTrace: log.decisions().map((d) => d.chosenId),
    });
  }

  return summarise(options.ruleset, games);
}

function strengthOf(state: GameState, side: Side): number {
  return forceElementsOf(state, side).reduce((sum, fe) => sum + fe.combatStrength, 0);
}

function summarise(ruleset: RuleSet, games: GameSummary[]): BatchResult {
  const wins = { blue: 0, red: 0, draw: 0 };
  const usage: Record<string, number> = {};

  for (const game of games) {
    if (game.winner === "blue") wins.blue += 1;
    else if (game.winner === "red") wins.red += 1;
    else wins.draw += 1;

    for (const [name, count] of Object.entries(game.modifiersFired)) {
      usage[name] = (usage[name] ?? 0) + count;
    }
  }

  const neverFired = declaredModifiers(ruleset).filter((name) => !(name in usage));

  return {
    rulesetId: ruleset.id,
    games,
    wins,
    meanTurns: mean(games.map((g) => g.turns)),
    meanDecisionBreadth: mean(games.map((g) => g.meanOptions)),
    modifierUsage: usage,
    neverFired,
  };
}

/**
 * Every modifier the ruleset declares, by the name a resolver would log it
 * under.
 *
 * Kept in step with the resolvers by hand, which is a real maintenance burden
 * and worth it: without it "never fired" cannot distinguish a rule that did
 * nothing from a rule nobody wired up, and those need opposite fixes.
 */
export function declaredModifiers(ruleset: RuleSet): string[] {
  return [
    ...Object.keys(ruleset.drms),
    "troopQuality",
    "strengthLost",
    "multipleDirections",
    "hqPresent",
    // Rally (5.2). Two of these carry a value of zero on purpose: the HQ and
    // out-of-contact effects are an automatic LEVEL of recovery rather than a
    // DRM, and they are logged as modifiers so that a rally can be explained
    // — and so this guard can prove they are reachable at all.
    "hqRally",
    "outOfContact",
    "rallyQuality",
    "range",
    "snapShot",
    "counteractionFire",
    "defensiveFire",
    "partialContact",
    "targetConcealed",
    "observerIsRecce",
    "throughSmoke",
    ...Object.keys(ruleset.assault.shifts),
    "attackerAlreadyInMelee",
  ];
}

function mean(values: number[]): number {
  if (values.length === 0) return 0;
  return Math.round((values.reduce((a, b) => a + b, 0) / values.length) * 100) / 100;
}

// ── Comparison ─────────────────────────────────────────────────────────────

export interface Comparison {
  a: BatchResult;
  b: BatchResult;
  /** Seeds where the winner differed. */
  outcomesChanged: string[];
  /** Seeds where commanders made at least one different choice. */
  decisionsChanged: string[];
  meanTurnsDelta: number;
  verdict: string;
}

/**
 * Compare two rulesets over the same seeds.
 *
 * Paired by seed, which is the strongest comparison available: both runs get
 * the same dice for as long as they stay in step. They will diverge once an
 * outcome differs — that is not a flaw, it is the measurement.
 */
export function compare(a: BatchResult, b: BatchResult, label: string): Comparison {
  const bySeed = new Map(b.games.map((g) => [g.seed, g]));
  const outcomesChanged: string[] = [];
  const decisionsChanged: string[] = [];

  for (const gameA of a.games) {
    const gameB = bySeed.get(gameA.seed);
    if (!gameB) continue;
    if (gameA.winner !== gameB.winner) outcomesChanged.push(gameA.seed);
    if (gameA.decisionTrace.join("|") !== gameB.decisionTrace.join("|")) {
      decisionsChanged.push(gameA.seed);
    }
  }

  const total = a.games.length || 1;
  const meanTurnsDelta = Math.round((b.meanTurns - a.meanTurns) * 100) / 100;

  return {
    a,
    b,
    outcomesChanged,
    decisionsChanged,
    meanTurnsDelta,
    verdict: verdictFor(label, outcomesChanged.length, decisionsChanged.length, total, meanTurnsDelta),
  };
}

function verdictFor(
  label: string,
  outcomes: number,
  decisions: number,
  total: number,
  turnsDelta: number,
): string {
  const outcomePct = Math.round((outcomes / total) * 100);
  const decisionPct = Math.round((decisions / total) * 100);

  // Decisions are weighted above outcomes on purpose. A rule can flip a winner
  // through the dice alone; changing what a commander CHOOSES is the thing
  // that makes a mechanic worth its complexity.
  let strength: string;
  if (decisionPct === 0 && outcomePct === 0) {
    strength = "no measurable effect — a candidate for deletion";
  } else if (decisionPct === 0) {
    strength = "changed outcomes but never a decision — it is moving the dice, not the play";
  } else if (decisionPct < 20) {
    strength = "marginal";
  } else if (decisionPct < 50) {
    strength = "real";
  } else {
    strength = "substantial";
  }

  return (
    `${label}: decisions changed in ${decisions}/${total} games (${decisionPct}%), ` +
    `outcomes in ${outcomes}/${total} (${outcomePct}%), ` +
    `mean game length ${turnsDelta >= 0 ? "+" : ""}${turnsDelta} turns. ` +
    `Effect: ${strength}. ` +
    `NOTE: evidence about the rules, not about combat — the board is flat and ` +
    `force management values are provisional.`
  );
}

/**
 * Does one module earn its place?
 *
 * Runs the same scenario and seeds with the module off and on, and reports
 * what changed. This is the function the whole platform exists to offer.
 */
export async function moduleImpact(
  module: keyof ModuleFlags,
  options: BatchOptions,
): Promise<Comparison> {
  const off = withModules(options.ruleset, { [module]: false } as Partial<ModuleFlags>);
  const on = withModules(options.ruleset, { [module]: true } as Partial<ModuleFlags>);

  const withoutIt = await runBatch({ ...options, ruleset: off });
  const withIt = await runBatch({ ...options, ruleset: on });

  return compare(withoutIt, withIt, String(module));
}

/** A short, readable report for a batch. */
export function describeBatch(result: BatchResult): string {
  const total = result.games.length;
  const lines = [
    `ruleset ${result.rulesetId} over ${total} games`,
    `  blue ${result.wins.blue} · red ${result.wins.red} · drawn ${result.wins.draw}`,
    `  mean length ${result.meanTurns} turns · mean options per decision ${result.meanDecisionBreadth}`,
  ];
  if (result.neverFired.length > 0) {
    lines.push(`  never fired: ${result.neverFired.join(", ")}`);
  }
  return lines.join("\n");
}
