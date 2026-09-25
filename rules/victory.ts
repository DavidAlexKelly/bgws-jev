// ── bgws/rules/victory.ts ──────────────────────────────────────────────────
// Who won, and how well.
//
// ⚠ THE GAME USED TO BE DECIDED BY ATTRITION ALONE, AND BGWS IS ABOUT GROUND.
//
// `runGame` compared total Combat Strength at the turn limit and called the
// larger number the winner. That is a defensible tie-break and a poor
// definition of victory: a side that traded well while ignoring its objective
// scored as a winner, and every "outcomes changed" figure in every report was
// therefore a measure of who did more damage rather than of who achieved
// anything. The SSIs define victory against OBJECTIVES, in three grades —
// Decisive, Substantive, Marginal — and this is that, modelled.
//
// WHAT IS THE RULEBOOK'S AND WHAT IS OURS
// ---------------------------------------
// The grades and the idea that both sides can score something are the
// rulebook's (3.1: "A side may be able to achieve a Decisive, Substantive or
// Marginal victory, or no success. It is possible for both sides to achieve
// some form of success."). The THRESHOLDS are ours: per-scenario victory
// conditions live in the SSIs, which we do not have, so a general rule stands
// in — hold your objective, and how badly you hurt the enemy decides the
// grade. Every number is in VictoryRule where it can be argued with.

import { distanceM } from "../lib/board";
import type { GameState, Side } from "../lib/state";
import { forceElementsOf } from "../lib/state";
import type { RuleSet } from "./ruleset";

export type VictoryLevel = "decisive" | "substantive" | "marginal" | "none";

/** What the verdict was drawn from, so a screen or a report can show its work. */
export interface SideStanding {
  strength: number;
  strengthStart: number;
  /** Fraction of starting Combat Strength still in the field, 0-1. */
  effectiveness: number;
  /** Holding its own objective: present in force, with no enemy contesting. */
  holdsObjective: boolean;
  /** How close its nearest living element is to its objective, in metres. */
  nearestToObjectiveM: number | null;
  combatEffective: boolean;
}

export interface Verdict {
  winner: Side | null;
  level: VictoryLevel;
  /** Which clause decided it, in the words of this file. */
  basis:
    | "annihilation"
    | "objectiveAndEnemyBroken"
    | "objectiveHeld"
    | "enemyBroken"
    | "attrition"
    | "stalemate";
  standing: Record<Side, SideStanding>;
}

function standingOf(state: GameState, side: Side, ruleset: RuleSet): SideStanding {
  const mine = forceElementsOf(state, side);
  const living = mine.filter((fe) => fe.combatStrength > 0);
  const strength = living.reduce((sum, fe) => sum + fe.combatStrength, 0);
  const strengthStart = mine.reduce((sum, fe) => sum + fe.combatStrengthStart, 0);
  const objective = state.objectives?.[side];

  let nearest: number | null = null;
  let holds = false;
  if (objective) {
    const distances = living.map((fe) => distanceM(fe.position, objective));
    nearest = distances.length > 0 ? Math.min(...distances) : null;

    // HOLDING MEANS HOLDING IT AGAINST SOMEBODY. An element parked on the
    // objective with an enemy troop beside it has not taken anything, and a
    // rule that said otherwise would reward driving past the enemy rather
    // than beating them.
    const contested = forceElementsOf(state, side === "blue" ? "red" : "blue")
      .filter((fe) => fe.combatStrength > 0)
      .some((fe) => distanceM(fe.position, objective) <= ruleset.victory.holdWithinM);
    holds = nearest !== null && nearest <= ruleset.victory.holdWithinM && !contested;
  }

  const effectiveness = strengthStart > 0 ? strength / strengthStart : 0;
  return {
    strength,
    strengthStart,
    effectiveness,
    holdsObjective: holds,
    nearestToObjectiveM: nearest,
    combatEffective: effectiveness > ruleset.victory.combatIneffectiveBelow,
  };
}

/**
 * Judge the engagement.
 *
 * Read in order, and the order IS the rule: taking the ground you were sent
 * for outranks hurting the enemy, and hurting the enemy outranks having more
 * left over. A game decided on the last clause is close to a draw and is
 * reported as `marginal` or `none` so that nobody mistakes it for a result.
 */
export function judgeVictory(state: GameState, ruleset: RuleSet): Verdict {
  const standing = {
    blue: standingOf(state, "blue", ruleset),
    red: standingOf(state, "red", ruleset),
  };

  const alive = {
    blue: standing.blue.strength > 0,
    red: standing.red.strength > 0,
  };

  // 1. Somebody has nothing left.
  if (!alive.blue || !alive.red) {
    const winner = alive.blue ? "blue" : alive.red ? "red" : null;
    return { winner, level: winner ? "decisive" : "none", basis: "annihilation", standing };
  }

  // 2. Objective held against a broken enemy: the whole mission, achieved.
  for (const side of ["blue", "red"] as const) {
    const enemy = side === "blue" ? "red" : "blue";
    if (standing[side].holdsObjective && !standing[enemy].combatEffective) {
      return { winner: side, level: "decisive", basis: "objectiveAndEnemyBroken", standing };
    }
  }

  // 3. Objective held, enemy still in the field. Substantive: the ground is
  //    taken and the enemy can still fight for it.
  const holders = (["blue", "red"] as const).filter((side) => standing[side].holdsObjective);
  if (holders.length === 1) {
    return { winner: holders[0], level: "substantive", basis: "objectiveHeld", standing };
  }
  // Both on their objectives is a real BGWS outcome — the objectives are in
  // different places — and neither has denied the other anything.
  if (holders.length === 2) {
    return { winner: null, level: "substantive", basis: "objectiveHeld", standing };
  }

  // 4. Nobody took anything, but one side is no longer a force.
  const broken = (["blue", "red"] as const).filter((side) => !standing[side].combatEffective);
  if (broken.length === 1) {
    const winner = broken[0] === "blue" ? "red" : "blue";
    return { winner, level: "marginal", basis: "enemyBroken", standing };
  }

  // 5. Attrition, and only if the gap is wide enough to mean anything. This is
  //    the clause the old engine used for everything.
  const gap = standing.blue.effectiveness - standing.red.effectiveness;
  if (Math.abs(gap) >= ruleset.victory.marginalEffectivenessGap) {
    return {
      winner: gap > 0 ? "blue" : "red",
      level: "marginal",
      basis: "attrition",
      standing,
    };
  }

  return { winner: null, level: "none", basis: "stalemate", standing };
}

/** One line for a screen or a report. */
export function describeVerdict(verdict: Verdict): string {
  if (!verdict.winner) {
    return verdict.level === "substantive"
      ? "both sides hold their objectives"
      : verdict.level === "none"
        ? "no result: neither side achieved anything decisive"
        : `drawn (${verdict.basis})`;
  }
  const reason = {
    annihilation: "the enemy was eliminated",
    objectiveAndEnemyBroken: "objective held and the enemy broken",
    objectiveHeld: "objective held",
    enemyBroken: "the enemy was broken",
    attrition: "more combat power left in the field",
    stalemate: "nothing decisive",
  }[verdict.basis];
  return `${verdict.winner} — ${verdict.level}: ${reason}`;
}
