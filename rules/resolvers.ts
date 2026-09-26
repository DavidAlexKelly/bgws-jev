// ── bgws/rules/resolvers.ts ────────────────────────────────────────────────
// Direct fire, morale, sighting, initiative.
//
// Each is the same shape: gather NAMED modifiers, roll, read a table, return
// the deltas and an event that explains itself. No resolver holds a number —
// every one comes from the RuleSet, so a mechanic can be retuned or switched
// off without touching this file. That is what makes the platform able to
// answer "does this rule matter?".
//
// Nothing here mutates state. A resolver returns effects; the engine applies
// them. That keeps resolution replayable and makes a resolver testable with a
// literal.

import { distanceM } from "../lib/board";
import type { ForceElement, Morale, MunitionKind, Side } from "../lib/state";
import { degradeMorale, hasMarker, improveMorale } from "../lib/state";
import type { Rng } from "./dice";
import type { Modifier, ResolutionEvent, StateDelta } from "./events";
import {
  fireColumnFor,
  fireResultFor,
  hitsFor,
  type FireResult,
  type RuleSet,
} from "./ruleset";

/** Everything a resolver may need to know about the situation. */
export interface FireContext {
  /** Target is in woods or urban ground. */
  targetInCover?: boolean;
  /** Smoke between firer and target. */
  smoke?: boolean;
  /** Fire is coming from a flank or the rear. */
  flank?: boolean;
  /** Range in metres, for the long-range modifier. */
  rangeM?: number;
  /** The firing capability's maximum range, for the same. */
  maxRangeM?: number;
  /** Armour the firing capability defeats at 1 km. Undefined is unknown. */
  penetrationMm?: number;
  /**
   * How the round defeats armour. Undefined behaves as kinetic, which is what
   * the engine assumed for everything before munitions were modelled.
   */
  munition?: MunitionKind;
  /** The round attacks the roof rather than the aspect it was fired from. */
  topAttack?: boolean;
  /** Reactive Fire: interrupting an enemy move, outside your own activation (7.1.3). */
  snapShot?: boolean;
  /** Fired in the Counteraction Round rather than the Action-Reaction Round (7.2.2). */
  counteractionFire?: boolean;
  /** Fired by a defender at the people assaulting it (9.3.2). */
  defensiveFire?: boolean;
  /**
   * Indirect fire onto a target that is only Partially Sighted (9.2.2).
   *
   * Direct fire cannot do this at all; indirect fire can, at a penalty,
   * because somebody has reported a contact rather than identified a target.
   */
  partialContact?: boolean;
  /**
   * Modifiers a caller adds on top of the rulebook's — the real-time mode's
   * range bands. Absent in the turn game, which is therefore unchanged.
   */
  extraModifiers?: readonly Modifier[];
}

export interface ResolutionOutcome {
  event: Omit<ResolutionEvent, "seq">;
  effects: StateDelta[];
}

/**
 * Direct fire, from one or more firers at one target.
 *
 * Combined fire sums Combat Strength and takes the WORST modifier among the
 * firers — BGWS's rule, kept because it is a good one: it stops a commander
 * stacking a suppressed unit's fire in for free, and it makes "who fires
 * together" a real decision rather than always-everyone.
 */
export function resolveDirectFire(
  firers: readonly ForceElement[],
  target: ForceElement,
  context: FireContext,
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
  phase: ResolutionEvent["phase"],
): ResolutionOutcome {
  const modifiers: Modifier[] = [];
  const combinedStrength = firers.reduce((sum, fe) => sum + fe.combatStrength, 0);

  if (context.targetInCover) {
    modifiers.push({ source: "targetInCover", value: ruleset.drms.targetInCover });
  }
  if (hasMarker(target, "moved")) {
    modifiers.push({ source: "targetMoved", value: ruleset.drms.targetMoved });
  }
  if (target.morale !== "good") {
    modifiers.push({ source: "targetSuppressed", value: ruleset.drms.targetSuppressed });
  }
  // The worst firer's state applies to the whole group, not the best.
  if (firers.some((fe) => hasMarker(fe, "moved"))) {
    modifiers.push({ source: "firerMoved", value: ruleset.drms.firerMoved });
  }
  if (firers.some((fe) => fe.morale !== "good")) {
    modifiers.push({ source: "firerSuppressed", value: ruleset.drms.firerSuppressed });
  }
  if (context.smoke) {
    modifiers.push({ source: "smoke", value: ruleset.drms.smoke });
  }
  if (context.flank) {
    modifiers.push({ source: "flank", value: ruleset.drms.flank });
  }
  if (context.snapShot) {
    modifiers.push({ source: "snapShot", value: ruleset.reaction.snapShotDrm });
  }
  // A second-round shot is a worse shot than a first-round one. Without this
  // there is no reason to fire in the Action-Reaction Round at all — you
  // would always hold, see what the enemy did, and shoot in the Counteraction
  // Round with better information and no penalty.
  if (context.counteractionFire) {
    modifiers.push({ source: "counteractionFire", value: ruleset.counteraction.fireDrm });
  }
  // 9.3.2, and the -2 is the rulebook's own. Firing at people who are already
  // on top of you is not the same shot as firing at people a kilometre away.
  if (context.defensiveFire) {
    modifiers.push({ source: "defensiveFire", value: ruleset.assault.defensiveFireDrm });
  }
  if (context.partialContact) {
    modifiers.push({
      source: "partialContact",
      value: ruleset.indirectFire.partialSightingDrm,
    });
  }
  if (
    context.rangeM != null &&
    context.maxRangeM != null &&
    context.rangeM > context.maxRangeM / 2
  ) {
    modifiers.push({ source: "longRange", value: ruleset.drms.longRange });
  }
  if (context.extraModifiers) modifiers.push(...context.extraModifiers);

  const roll = rng.d66();
  const total = roll.total + modifiers.reduce((sum, m) => sum + m.value, 0);
  const column = fireColumnFor(ruleset, combinedStrength);
  const result = fireResultFor(column, total);

  // Can the round hurt what it hit? A hit that cannot get through still
  // shakes the crew, so it degrades to suppression rather than nothing: being
  // bounced off is not the same as being missed.
  const facedArmourMm = armourFacing(target, context, ruleset);
  const defeated = canPenetrate(
    context.penetrationMm,
    facedArmourMm,
    context.rangeM,
    ruleset,
    context.munition,
  );
  const gated =
    !defeated && ruleset.penetration.onFailure === "hardGate" && hitsFor(result) > 0
      ? "suppress"
      : result;

  const effects = fireEffects(target, gated, ruleset);

  return {
    event: {
      type: "resolution",
      turn,
      phase,
      kind: "directFire",
      rulesetId: ruleset.id,
      actorIds: firers.map((fe) => fe.id),
      targetIds: [target.id],
      roll,
      modifiers,
      total,
      table: `fire:${column.label}`,
      // The GATED result, not the rolled one. Reporting "two hits" for a round
      // that bounced would make the log disagree with the board, and the log
      // is what anybody debugging a game reads first.
      result: gated,
      effects,
      narrative: defeated
        ? describeFire(firers, target, gated, total, column.label)
        : `${describeFire(firers, target, gated, total, column.label)} ` +
          `Rounds failed to defeat ${facedArmourMm} mm of ` +
          `${aspectName(context)} armour.`,
    },
    effects,
  };
}

/**
 * Can this shot actually hurt that target?
 *
 * Returns true when the module is off, when either figure is unknown, or when
 * the round gets through. UNKNOWN FAILS OPEN — see PenetrationRule. A rule
 * that punishes gaps in the data rather than weaknesses in the gun would have
 * silently disabled the Challenger 2, whose penetration the source does not
 * record.
 */
export function canPenetrate(
  penetrationMm: number | undefined,
  armourMm: number | undefined,
  rangeM: number | undefined,
  ruleset: RuleSet,
  munition?: MunitionKind,
): boolean {
  if (!ruleset.modules.penetration) return true;
  if (penetrationMm == null || armourMm == null) return true;

  // ⚠ A SHAPED CHARGE DOES NOT WEAKEN WITH RANGE. The jet is formed on
  // impact, so a Kornet defeats the same armour at 5 km as at 500 m. Falloff
  // is a kinetic phenomenon and applying it to everything — which is what
  // this did before munitions were modelled — quietly nerfed every guided
  // anti-tank weapon in the game at exactly the ranges they are built for.
  const kinetic = munition == null || munition === "ke";
  if (!kinetic) return penetrationMm >= armourMm;

  // Penetration is quoted at 1 km. Falloff only applies beyond that; nothing
  // gains penetration by being closer than the figure was measured at.
  const beyondKm = Math.max(0, ((rangeM ?? 1000) - 1000) / 1000);
  const effective =
    penetrationMm * Math.max(0, 1 - beyondKm * ruleset.penetration.falloffPerKmBeyond1Km);

  return effective >= armourMm;
}

/**
 * Which armour this shot actually meets.
 *
 * ⚠ THIS IS WHAT MAKES MANOEUVRE MECHANICAL RATHER THAN DECORATIVE. The
 * engine has computed a flank aspect for a long time, but it only ever spent
 * it on a dice modifier: the shot still met the glacis, so going round the
 * side improved the roll and nothing else. A Challenger 2 is 700 mm at the
 * front and 140 mm at the side, and a Javelin arriving through a 30 mm roof
 * is fighting a different vehicle again.
 *
 * Falls back to the single frontal figure when the element has no facings,
 * which is every element built from the L6 profile and every hand-declared
 * platform. Unknown still fails open, for the reason in PenetrationRule.
 */
export function armourFacing(
  target: ForceElement,
  context: FireContext,
  ruleset: RuleSet,
): number | undefined {
  const armour = target.armour;
  if (!ruleset.modules.facingArmour || !armour) return target.armourMm;

  const shaped = context.munition === "ce" || context.munition === "ceTandem";
  const aspect = context.topAttack ? "roof" : context.flank ? "side" : "front";

  const value = shaped
    ? ({ front: armour.frontCeMm, side: armour.sideCeMm, roof: armour.roofCeMm } as const)[aspect]
    : ({ front: armour.frontKeMm, side: armour.sideKeMm, roof: armour.roofKeMm } as const)[aspect];

  const resolved = value ?? target.armourMm;
  if (resolved == null) return undefined;

  // A tandem warhead's precursor strips reactive armour, which is the entire
  // reason tandem warheads exist. Without this a Kornet and an RPG-7 meet the
  // same protection.
  if (context.munition === "ceTandem" && target.eraFitted) {
    return resolved * (1 - ruleset.penetration.tandemEraDefeatFraction);
  }
  return resolved;
}

/** How to name the aspect in a narrative line. */
function aspectName(context: FireContext): string {
  if (context.topAttack) return "roof";
  return context.flank ? "side" : "frontal";
}

function fireEffects(
  target: ForceElement,
  result: FireResult,
  ruleset: RuleSet,
): StateDelta[] {
  const hits = hitsFor(result);
  const effects: StateDelta[] = [];

  if (hits > 0) {
    // Strength and morale scale separately from the ruleset: being shaken is
    // not the same as being destroyed, and tying them to one number made
    // game length impossible to tune without also changing how units break.
    const lost = hits * ruleset.lethality.strengthPerHit;
    effects.push({ kind: "combatStrength", feId: target.id, delta: -lost });
    effects.push({
      kind: "morale",
      feId: target.id,
      to: degradeMorale(target.morale, hits * ruleset.lethality.moraleStepsPerHit),
    });
    if (target.combatStrength - lost <= 0) {
      effects.push({ kind: "eliminated", feId: target.id });
    }
  } else if (result === "suppress") {
    effects.push({ kind: "morale", feId: target.id, to: degradeMorale(target.morale, 1) });
  }

  return effects;
}

function describeFire(
  firers: readonly ForceElement[],
  target: ForceElement,
  result: FireResult,
  total: number,
  column: string,
): string {
  const who =
    firers.length === 1 ? firers[0].label : `${firers.length} elements together`;
  const what =
    result === "miss"
      ? "no effect"
      : result === "suppress"
        ? "suppressed"
        : `${hitsFor(result)} hit${hitsFor(result) > 1 ? "s" : ""}`;
  return `${who} engaged ${target.label}: ${what} (${column}, modified ${total}).`;
}

/**
 * RALLY (5.2). The Command Sub-phase's first step, and the only thing on the
 * board that moves morale UPWARDS on purpose.
 *
 * Three things distinguish it from a Morale Check, and all three are the
 * rulebook's:
 *
 *   1. It CANNOT make things worse. 5.2 is an attempt "to recover one or two
 *      levels"; a failed attempt recovers nothing. A check that could break
 *      the element would make being shaken a death spiral with no exit, which
 *      is exactly the state this rule exists to prevent.
 *   2. An automatic level comes first, for an element co-located with an
 *      un-Suppressed HQ, or out of line of sight of every enemy. Both are
 *      free of the dice, so pulling back into dead ground is a RELIABLE way
 *      to recover — which is what makes it a decision rather than a prayer.
 *   3. The roll is a single D6 against a 4+. That is the rulebook's own
 *      number, unusually: most resolution figures in BGWS live on Player Aids
 *      that are not in the box we have.
 *
 * Together they cap recovery at two levels a turn, which is 5.2's own play
 * note: "an FE can recover two levels of Morale Status with one Rally".
 */
export function resolveRally(
  fe: ForceElement,
  options: { hqPresent?: boolean; outOfContact?: boolean },
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
): ResolutionOutcome {
  const modifiers: Modifier[] = [];
  if (options.hqPresent) modifiers.push({ source: "hqRally", value: 0 });
  if (options.outOfContact) modifiers.push({ source: "outOfContact", value: 0 });

  // Troop Quality, compressed onto a D6. See RallyRule.qualityDrm.
  if (fe.troopQuality >= ruleset.rally.goodAt) {
    modifiers.push({ source: "rallyQuality", value: ruleset.rally.qualityDrm });
  } else if (fe.troopQuality <= ruleset.rally.poorAt) {
    modifiers.push({ source: "rallyQuality", value: -ruleset.rally.qualityDrm });
  }

  const automatic = options.hqPresent || options.outOfContact ? 1 : 0;
  const roll = rng.d6();
  const total = roll.total + modifiers.reduce((sum, m) => sum + m.value, 0);
  const passed = total >= ruleset.rally.passTarget;
  const levels = automatic + (passed ? 1 : 0);

  const effects: StateDelta[] =
    levels > 0 ? [{ kind: "morale", feId: fe.id, to: improveMorale(fe.morale, levels) }] : [];

  const reason = [
    automatic > 0 ? (options.hqPresent ? "steadied by its HQ" : "out of contact") : null,
    passed ? "rallied" : "did not rally",
  ]
    .filter(Boolean)
    .join(", ");

  return {
    event: {
      type: "resolution",
      turn,
      phase: "command",
      kind: "rally",
      rulesetId: ruleset.id,
      actorIds: [fe.id],
      targetIds: [],
      roll,
      modifiers,
      total,
      table: "rally",
      result: levels > 0 ? "recovered" : "held",
      effects,
      narrative:
        `${fe.label} rally: ${reason} (${total} vs ${ruleset.rally.passTarget}); ` +
        `${levels > 0 ? `${fe.morale} → ${improveMorale(fe.morale, levels)}` : `still ${fe.morale}`}.`,
    },
    effects,
  };
}

/**
 * A morale check.
 *
 * Passing leaves the unit where it is; failing steps it down. Troop Quality is
 * added to the roll rather than being a target number, so a good unit is
 * better at everything rather than merely luckier — which is the behaviour a
 * training audience expects from "quality".
 */
export function resolveMoraleCheck(
  fe: ForceElement,
  options: { multipleDirections?: boolean; hqPresent?: boolean },
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
  phase: ResolutionEvent["phase"],
): ResolutionOutcome {
  const modifiers: Modifier[] = [{ source: "troopQuality", value: fe.troopQuality }];

  const lost = fe.combatStrengthStart - fe.combatStrength;
  if (lost > 0) {
    // Proportional to how much of the sub-unit is gone, not to an absolute
    // number of points — so it means the same thing whatever a hit is worth.
    // See MoraleTable.penaltyAtTotalLoss.
    const fraction = lost / Math.max(1, fe.combatStrengthStart);
    modifiers.push({
      source: "strengthLost",
      value: -Math.round(fraction * ruleset.morale.penaltyAtTotalLoss),
    });
  }
  if (options.multipleDirections) {
    modifiers.push({ source: "multipleDirections", value: -ruleset.morale.multipleDirections });
  }
  if (options.hqPresent) {
    modifiers.push({ source: "hqPresent", value: ruleset.morale.hqPresent });
  }

  const roll = rng.d66();
  const total = roll.total + modifiers.reduce((sum, m) => sum + m.value, 0);
  const passed = total >= ruleset.morale.passTarget;

  // A pass RALLIES when the ruleset says so, rather than merely not costing a
  // step. See MoraleTable.rallyOnPass: without it Troop Quality measured
  // exactly 50% over 400 games, because fire damage had already saturated
  // morale and a downside-only check had nothing left to protect.
  const effects: StateDelta[] =
    passed
      ? ruleset.morale.rallyOnPass && fe.morale !== "good"
        ? [{ kind: "morale", feId: fe.id, to: improveMorale(fe.morale, 1) }]
        : []
      : [{ kind: "morale", feId: fe.id, to: degradeMorale(fe.morale, 1) }];

  return {
    event: {
      type: "resolution",
      turn,
      phase,
      kind: "morale",
      rulesetId: ruleset.id,
      actorIds: [fe.id],
      targetIds: [],
      roll,
      modifiers,
      total,
      table: "morale",
      result: passed ? "passed" : "failed",
      effects,
      narrative: `${fe.label} morale check: ${passed ? "held" : "shaken"} (${total} vs ${ruleset.morale.passTarget}).`,
    },
    effects,
  };
}

export type SightingOutcomeLevel = "none" | "partial" | "full";

/**
 * A sighting attempt.
 *
 * Geometry decides whether a line exists; this decides whether anyone noticed.
 * Splitting the two is what lets `partialSighting` be switched off without
 * touching line of sight: with the module off, anything that is not `none`
 * becomes `full`.
 */
export function resolveSighting(
  observer: ForceElement,
  target: ForceElement,
  options: { targetInCover?: boolean; observerIsRecce?: boolean; throughSmoke?: boolean },
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
  phase: ResolutionEvent["phase"],
  viewer: Side,
): ResolutionOutcome {
  const rangeKm = distanceM(observer.position, target.position) / 1000;
  const modifiers: Modifier[] = [
    {
      source: "range",
      value: Math.round(rangeKm) * ruleset.sighting.perKilometre,
    },
  ];

  if (options.targetInCover) {
    modifiers.push({ source: "targetInCover", value: ruleset.sighting.targetInCover });
  }
  if (hasMarker(target, "moved")) {
    modifiers.push({ source: "targetMoved", value: ruleset.sighting.targetMoved });
  }
  if (target.concealed && ruleset.modules.concealment) {
    modifiers.push({ source: "targetConcealed", value: ruleset.sighting.targetConcealed });
  }
  // 9.2.2.4: "any FE that Attempts Sighting from, through or into Smoke
  // suffers a -2 DRM". Smoke blinds as well as protects, which is what makes
  // laying it a decision rather than a free buff.
  if (options.throughSmoke) {
    modifiers.push({ source: "throughSmoke", value: ruleset.sighting.throughSmoke });
  }
  if (options.observerIsRecce) {
    modifiers.push({ source: "observerIsRecce", value: ruleset.sighting.observerIsRecce });
  }

  const roll = rng.d66();
  const total = roll.total + modifiers.reduce((sum, m) => sum + m.value, 0);

  let level: SightingOutcomeLevel = "none";
  if (total >= ruleset.sighting.fullAt) level = "full";
  else if (total >= ruleset.sighting.partialAt) level = "partial";

  // With three-state contact switched off, a contact is a contact.
  if (!ruleset.modules.partialSighting && level === "partial") level = "full";

  const effects: StateDelta[] =
    level === "none" ? [] : [{ kind: "sighting", viewer, feId: target.id, to: level }];

  return {
    event: {
      type: "resolution",
      turn,
      phase,
      kind: "sighting",
      rulesetId: ruleset.id,
      actorIds: [observer.id],
      targetIds: [target.id],
      roll,
      modifiers,
      total,
      table: "sighting",
      result: level,
      effects,
      narrative: `${observer.label} scanning: ${level} contact at ${rangeKm.toFixed(1)} km.`,
    },
    effects,
  };
}

/**
 * The initiative test.
 *
 * Both sides roll; the higher takes the initiative. The transmission advantage
 * is inverted on purpose — the side that talked LESS is harder to find and
 * reacts faster, which is the lesson the mechanic exists to teach.
 */
export function resolveInitiative(
  transmissions: Record<Side, number>,
  lossesLastTurn: Record<Side, number>,
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
): ResolutionOutcome & { winner: Side } {
  const scores: Record<Side, number> = { blue: 0, red: 0 };
  const modifiers: Modifier[] = [];
  const dice: number[] = [];

  const talkDelta = transmissions.red - transmissions.blue;
  const quieter: Side | null = talkDelta === 0 ? null : talkDelta > 0 ? "blue" : "red";

  for (const side of ["blue", "red"] as const) {
    const roll = rng.d6();
    dice.push(roll.total);
    let score = roll.total;

    const losses = lossesLastTurn[side] ?? 0;
    if (losses > 0) {
      const penalty = losses * ruleset.initiative.perLossLastTurn;
      score += penalty;
      modifiers.push({ source: `${side}:lossesLastTurn`, value: penalty });
    }
    scores[side] = score;
  }

  // A tie goes to the side that transmitted less. If that is also tied it is
  // decided by a die, NOT by a fixed side.
  //
  // It used to fall to blue. That is a small bias per turn and a large one
  // over a game, and it showed up the moment a symmetric control was run with
  // every module on: identical forces, identical troop quality, and blue won
  // 24 of 40 against red's 10. A control that is not actually fair silently
  // corrupts every measurement taken against it — which is most of them.
  const tieBreak: Side = quieter ?? (rng.d6().total % 2 === 0 ? "blue" : "red");
  const winner: Side =
    scores.blue > scores.red ? "blue" : scores.red > scores.blue ? "red" : tieBreak;

  const effects: StateDelta[] = [{ kind: "initiative", side: winner }];

  return {
    winner,
    effects,
    event: {
      type: "resolution",
      turn,
      phase: "initiative",
      kind: "initiative",
      rulesetId: ruleset.id,
      actorIds: [],
      targetIds: [],
      roll: { dice, total: dice.reduce((a, b) => a + b, 0), cursor: 0 },
      modifiers,
      total: Math.max(scores.blue, scores.red),
      table: "initiative",
      result: winner,
      effects,
      narrative: `Initiative: ${winner} (blue ${scores.blue}, red ${scores.red}).`,
    },
  };
}

/** Morale levels at which a force element may not be ordered to advance. */
export function canAdvance(morale: Morale): boolean {
  return morale === "good" || morale === "suppressed1";
}

/**
 * Can this Force Element engage at all?
 *
 * A BROKEN sub-unit cannot. It is not destroyed — it is still on the board,
 * still occupying ground, still able to rally — but it is not fighting.
 *
 * Adding this is what finally gave Troop Quality a measurable effect. Broken
 * units used to keep firing at a -2 penalty, which is a rounding error on
 * 2D6, so breaking a unit cost it almost nothing and the difference between
 * veterans and conscripts came out at 50% over 400 games. Morale has to have
 * a consequence before quality can have a use.
 */
export function canEngage(morale: Morale): boolean {
  return morale !== "broken";
}
