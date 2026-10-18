// ── bgws/rules/assault.ts ──────────────────────────────────────────────────
// Closing with the enemy.
//
// Assault is the one resolution where the odds matter more than the dice, and
// that is the whole point of modelling it separately from fire: a commander
// who masses three-to-one and a commander who attacks at parity should be
// playing visibly different games.
//
// ONE SIMPLIFICATION, STATED
// --------------------------
// BGWS shifts COLUMNS for circumstances (cover, suppression, surprise) and
// then rolls on the column the shifts land on. Here the odds ladder is itself
// expressed as a modifier to the roll, and the shifts add to the same
// modifier. One mechanism instead of two, with the same shape: better odds and
// favourable circumstances both push the same number up.
//
// It is a simplification and it is reversible — if play shows that column
// moves and roll modifiers need to behave differently (they do interact
// differently with the tails of 2D6), the ladder is data and can be split.

import type { ForceElement } from "../lib/state";
import { degradeMorale, hasMarker } from "../lib/state";
import type { Rng } from "./dice";
import type { Modifier, ResolutionEvent, StateDelta } from "./events";
import type { ResolutionOutcome } from "./resolvers";
import type { RuleSet } from "./ruleset";

export type AssaultResult = "repulsed" | "melee" | "defenderBreaks";

export interface AssaultContext {
  defenderInCover?: boolean;
  /** The attacker achieved surprise — see the surprise roll in the turn loop. */
  surprise?: boolean;
  /** Defender is vehicles only, with no dismounts to hold the ground. */
  defenderIsVehicleOnly?: boolean;
}

/**
 * Which odds column a strength ratio falls in, as an index.
 *
 * The lowest column catches everything below it: attacking at one-to-four is
 * not better than attacking at one-to-two, and the table should not pretend to
 * resolve the difference.
 */
export function oddsColumnIndex(ruleset: RuleSet, ratio: number): number {
  let index = 0;
  ruleset.assault.oddsColumns.forEach((odds, i) => {
    if (ratio >= odds) index = i;
  });
  return index;
}

/** The index of the even-odds column, which is the ladder's zero point. */
function evenOddsIndex(ruleset: RuleSet): number {
  const at = ruleset.assault.oddsColumns.indexOf(1);
  return at === -1 ? 0 : at;
}

export function resolveAssault(
  attackers: readonly ForceElement[],
  defenders: readonly ForceElement[],
  context: AssaultContext,
  ruleset: RuleSet,
  rng: Rng,
  turn: number,
  phase: ResolutionEvent["phase"],
): ResolutionOutcome & { result: AssaultResult } {
  const attackStrength = attackers.reduce((sum, fe) => sum + fe.combatStrength, 0);
  const defendStrength = defenders.reduce((sum, fe) => sum + fe.combatStrength, 0);

  // A defenceless position is taken, not assaulted. Guarding the division is
  // not pedantry: an empty objective is a normal thing to advance onto.
  const ratio = defendStrength <= 0 ? Number.POSITIVE_INFINITY : attackStrength / defendStrength;

  const index = oddsColumnIndex(ruleset, ratio);
  const modifiers: Modifier[] = [
    { source: `odds:${formatRatio(ratio)}`, value: index - evenOddsIndex(ruleset) },
  ];

  if (context.defenderInCover) {
    modifiers.push({ source: "defenderInCover", value: ruleset.assault.shifts.defenderInCover });
  }
  if (defenders.some((fe) => fe.morale !== "good")) {
    modifiers.push({
      source: "defenderSuppressed",
      value: ruleset.assault.shifts.defenderSuppressed,
    });
  }
  if (context.surprise) {
    modifiers.push({ source: "attackerSurprise", value: ruleset.assault.shifts.attackerSurprise });
  }
  if (context.defenderIsVehicleOnly) {
    modifiers.push({
      source: "defenderIsVehicleOnly",
      value: ruleset.assault.shifts.defenderIsVehicleOnly,
    });
  }
  // An attacker who has already fought this turn goes in tired.
  if (attackers.some((fe) => hasMarker(fe, "melee"))) {
    modifiers.push({ source: "attackerAlreadyInMelee", value: -1 });
  }

  const roll = rng.d66();
  const total = roll.total + modifiers.reduce((sum, m) => sum + m.value, 0);

  const result: AssaultResult =
    total >= ruleset.assault.defenderBreaksAt
      ? "defenderBreaks"
      : total <= ruleset.assault.attackRepulsedAt
        ? "repulsed"
        : "melee";

  const effects = assaultEffects(attackers, defenders, result, ruleset);

  return {
    result,
    effects,
    event: {
      type: "resolution",
      turn,
      phase,
      kind: "assault",
      rulesetId: ruleset.id,
      actorIds: attackers.map((fe) => fe.id),
      targetIds: defenders.map((fe) => fe.id),
      roll,
      modifiers,
      total,
      table: `assault:${formatRatio(ratio)}`,
      result,
      effects,
      narrative: describeAssault(attackers, defenders, result, formatRatio(ratio), total),
    },
  };
}

/**
 * Where a retreating element ends up (9.3.6).
 *
 * "FEs that Retreat must Move a minimum of 500m, and may move up to 1,000m,
 * away from the enemy. Retreating attacking FEs must Move in the direction
 * they Assaulted from. Defending FEs must move away from the attacking FE,
 * towards their side's starting area."
 *
 * We do not track a Forming Up Point or a start line, so "the direction they
 * assaulted from" and "towards their side's starting area" both become
 * directly away from the other side — which is the same bearing in every
 * case a single assault produces, and is the part of the rule that matters:
 * a retreat breaks contact.
 */
export function retreatTo(
  fe: ForceElement,
  awayFrom: readonly ForceElement[],
  metres: number,
): { lat: number; lng: number } {
  const centre = {
    lat: awayFrom.reduce((sum, other) => sum + other.position.lat, 0) / (awayFrom.length || 1),
    lng: awayFrom.reduce((sum, other) => sum + other.position.lng, 0) / (awayFrom.length || 1),
  };

  // Degrees per metre, near enough at this latitude and over 500 m.
  const latPerM = 1 / 111_320;
  const lngPerM = 1 / (111_320 * Math.cos((fe.position.lat * Math.PI) / 180) || 1);

  const dLat = fe.position.lat - centre.lat;
  const dLng = fe.position.lng - centre.lng;
  const norm = Math.hypot(dLat, dLng);

  // Standing exactly on top of each other: fall back to due south, so a
  // retreat still breaks contact rather than silently not happening.
  if (norm === 0) return { lat: fe.position.lat - metres * latPerM, lng: fe.position.lng };

  return {
    lat: fe.position.lat + (dLat / norm) * metres * latPerM,
    lng: fe.position.lng + (dLng / norm) * metres * lngPerM,
  };
}

function assaultEffects(
  attackers: readonly ForceElement[],
  defenders: readonly ForceElement[],
  result: AssaultResult,
  ruleset: RuleSet,
): StateDelta[] {
  const effects: StateDelta[] = [];
  const closeCombat = ruleset.modules.closeCombat;
  const retreatM = ruleset.assault.retreatMinM;

  const hurt = (fe: ForceElement, strengthLoss: number, moraleSteps: number) => {
    if (strengthLoss > 0) {
      effects.push({ kind: "combatStrength", feId: fe.id, delta: -strengthLoss });
      if (fe.combatStrength - strengthLoss <= 0) {
        effects.push({ kind: "eliminated", feId: fe.id });
      }
    }
    if (moraleSteps > 0) {
      effects.push({ kind: "morale", feId: fe.id, to: degradeMorale(fe.morale, moraleSteps) });
    }
  };

  /**
   * A Retreat result (9.3.6): break contact, and one further step of morale.
   *
   * "Any Retreating FE(s) drops one additional level of Morale Status" — on
   * top of whatever the assault already cost, which is why a retreat can
   * finish a unit that the fighting did not.
   */
  const retreat = (
    retreating: readonly ForceElement[],
    awayFrom: readonly ForceElement[],
    alreadyLostSteps: number,
  ) => {
    for (const fe of retreating) {
      if (fe.combatStrength <= 0) continue;
      const to = retreatTo(fe, awayFrom, retreatM);
      effects.push({ kind: "position", feId: fe.id, lat: to.lat, lng: to.lng });
      effects.push({ kind: "marker", feId: fe.id, marker: "moved", added: true });
      effects.push({
        kind: "morale",
        feId: fe.id,
        to: degradeMorale(fe.morale, alreadyLostSteps + 1),
      });
    }
  };

  /** 9.3.10: everyone who was in it reorganises, and that takes a full turn. */
  const reorg = (participants: readonly ForceElement[]) => {
    for (const fe of participants) {
      if (fe.combatStrength <= 0) continue;
      effects.push({ kind: "marker", feId: fe.id, marker: "reorg", added: true });
      effects.push({ kind: "marker", feId: fe.id, marker: "reorgPlacedThisTurn", added: true });
    }
  };

  if (result === "defenderBreaks") {
    // The position falls. Defenders are broken rather than annihilated —
    // a wargame in which every assault kills teaches nothing about withdrawal.
    for (const fe of defenders) hurt(fe, 1, closeCombat ? 0 : 2);
    if (closeCombat) {
      // The defender took the Retreat result, so it goes: 9.3.6 rather than
      // standing on the objective it just lost.
      retreat(defenders, attackers, 2);
      reorg(attackers);
    } else {
      for (const fe of attackers) {
        effects.push({ kind: "marker", feId: fe.id, marker: "reorg", added: true });
      }
    }
  } else if (result === "repulsed") {
    for (const fe of attackers) hurt(fe, 1, closeCombat ? 0 : 1);
    if (closeCombat) {
      // Thrown back IS a Retreat result for the attacker.
      retreat(attackers, defenders, 1);
      reorg(defenders);
    }
  } else {
    // Melee: both sides bleed and neither holds cleanly. The marker survives
    // clean-up (9.3.8), so the two forces stay fixed until one of them
    // breaks — which is the whole of close combat and never used to happen.
    for (const fe of attackers) {
      hurt(fe, 1, 0);
      effects.push({ kind: "marker", feId: fe.id, marker: "melee", added: true });
    }
    for (const fe of defenders) {
      hurt(fe, 1, 1);
      effects.push({ kind: "marker", feId: fe.id, marker: "melee", added: true });
    }
  }

  return effects;
}

function formatRatio(ratio: number): string {
  if (!Number.isFinite(ratio)) return "unopposed";
  return `${ratio.toFixed(1)}:1`;
}

function describeAssault(
  attackers: readonly ForceElement[],
  defenders: readonly ForceElement[],
  result: AssaultResult,
  ratio: string,
  total: number,
): string {
  const who = attackers.map((fe) => fe.label).join(", ");
  const whom = defenders.map((fe) => fe.label).join(", ");
  const outcome =
    result === "defenderBreaks"
      ? "carried the position"
      : result === "repulsed"
        ? "was thrown back"
        : "is locked in close combat with";
  return `${who} assaulted ${whom} at ${ratio}: ${outcome} (modified ${total}).`;
}
