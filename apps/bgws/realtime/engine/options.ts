// ── bgws/realtime/engine/options.ts ────────────────────────────────────────
// The honest odds behind every option, and the places code picks: what fire
// from here (or from there) would really do, and where to close to effective
// range. The options themselves are the decision points' (decisions.ts).

import { bearingDeg, distanceM, type LatLng } from "../../lib/board";
import { lineOfSight } from "../../lib/lineOfSight";
import type { ForceElement, Side } from "../../lib/state";
import { inCover } from "../../lib/proceduralTerrain";
import { weaponFor } from "../../rules/turnLoop";
import { hitConditions, hullDownFrom, vehiclesIn } from "./engine";
import { aimedIntervalS, hitChance } from "./fire";
import { describeStrike, strikeOdds, type StrikeOdds } from "./lethality";
import { allowanceAt, offsetBy } from "./geometry";
import type { RtConfig, RtState } from "./types";

/**
 * The chance each round from `self` hits `enemy` right now (fire.ts), and how
 * many hits a shot should land — one round per fit vehicle.
 */
export function oddsAgainst(
  self: ForceElement,
  enemy: ForceElement,
  state: RtState,
  config: RtConfig,
  from: LatLng = self.position,
): { pHit: number; expectedHits: number; rounds: number } | null {
  const rangeM = distanceM(from, enemy.position);
  const weapon = weaponFor(self, enemy, rangeM);
  if (!weapon) return null;
  const pHit = hitChance(weapon, enemy, rangeM, hitConditions(state, self, enemy, config, from), config.timing);
  const rounds = Math.max(1, state.units[self.id]?.vehicles.fit ?? vehiclesIn(self));
  return { pHit, expectedHits: pHit * rounds, rounds };
}

/** Strength this side has seen itself take off an enemy: it watched the rounds land. */
export function knownDamage(state: RtState, side: Side, enemyId: string): number {
  return Object.entries(state.units)
    .filter(([id]) => state.game.forceElements[id]?.side === side)
    .reduce((sum, [, unit]) => sum + (unit.engagement?.targetId === enemyId ? unit.engagement.damage : 0), 0);
}

/** What fire from `firer` would really do to `target`, in real time. */
export interface Effect {
  /** Chance one shot knocks out a vehicle. */
  perShot: number;
  /** Chance of knocking out at least one vehicle within a minute. */
  perMinute: number;
  /** Expected minutes of steady fire to knock out every vehicle still fighting. */
  minutesToKnockOut: number | null;
  /** The face it would strike, and whether the round gets through. */
  strike?: StrikeOdds;
  /** Vehicles still fighting, as far as this side can see. */
  vehiclesLeft: number;
}

/**
 * The honest odds, by the same chain the engine rolls: the fire table's
 * hits (with range bands), the range-scaled chance a hit is a round on
 * target, and then — from where the firer would be — the face it strikes,
 * the round's penetration there against that armour, and the chance a
 * penetration knocks the vehicle out. `from` is for "close to" options.
 */
export function damageEffect(
  firer: ForceElement,
  target: ForceElement,
  state: RtState,
  config: RtConfig,
  from: LatLng = firer.position,
): Effect | null {
  const odds = oddsAgainst(firer, target, state, config, from);
  if (!odds) return null;
  const rangeM = distanceM(from, target.position);
  const weapon = weaponFor(firer, target, rangeM);
  if (!weapon) return null;
  const strike = strikeOdds(weapon, target, from, rangeM, config.ruleset, hullDownFrom(state, target, from, config));
  const { timing } = config;
  // Per round: hits and knocks out. Per shot: any of its rounds does.
  const perRound = odds.pHit * strike.pKnockOut;
  const perShot = 1 - Math.pow(1 - perRound, odds.rounds);
  const expectedPerShot = perRound * odds.rounds;
  const shotsPerMinute = 60 / aimedIntervalS(weapon, timing.shotIntervalS);
  const perMinute = 1 - Math.pow(1 - perShot, shotsPerMinute);
  // Knocked-out vehicles are seen to be knocked out.
  const vehiclesLeft = state.units[target.id]?.vehicles.fit ?? 1;
  return {
    perShot,
    perMinute,
    minutesToKnockOut: expectedPerShot > 0 ? vehiclesLeft / (expectedPerShot * shotsPerMinute) : null,
    strike,
    vehiclesLeft,
  };
}

/** "≈4%/min to knock out one of its vehicles (side, 620 vs 140 mm: 99% penetrate), ~35 min to finish all 4". */
export function describeEffect(effect: Effect | null, it = "it"): string {
  const whose = it === "you" ? "your" : "its";
  if (!effect || effect.perShot <= 0) {
    const s = effect?.strike;
    return s && s.pPenetrate < 0.01
      ? `cannot hurt ${it} from there (${describeStrike(s)}: rounds do not penetrate)`
      : `no chance of hurting ${it} from there`;
  }
  const pct = effect.perMinute < 0.01 ? "<1" : String(Math.round(effect.perMinute * 100));
  const s = effect.strike;
  const pen = s ? ` (${describeStrike(s)}: ${Math.round(s.pPenetrate * 100)}% penetrate)` : "";
  const mins = effect.minutesToKnockOut;
  const all = effect.vehiclesLeft > 1 ? `all ${effect.vehiclesLeft}` : "the last one";
  const ko = mins == null ? "" : mins > 120 ? `, over 2 h to finish ${all}` : `, ~${Math.max(1, Math.round(mins))} min to finish ${all}`;
  return `≈${pct}%/min to knock out one of ${whose} vehicles${pen}${ko}`;
}

/** Own units already firing on this enemy (within the last minute), besides `except`. */
export function engagedBy(state: RtState, side: Side, enemyId: string, except?: string): string[] {
  return Object.entries(state.units)
    .filter(([id, unit]) => {
      const fe = state.game.forceElements[id];
      return (
        id !== except &&
        fe?.side === side &&
        fe.combatStrength > 0 &&
        unit.engagement?.targetId === enemyId &&
        state.time - unit.engagement.lastShotAt <= 60
      );
    })
    .map(([id]) => id)
    .sort();
}

/**
 * A place to close to effective range of `enemy` from: on the near side of
 * it, inside 1 km (where the range bands make fire far more accurate and
 * lethal) and inside the weapon's short range and half its maximum, with a
 * line of sight to shoot from, and
 * preferring cover, fewer known enemies watching, and a shorter move.
 */
export function closePosition(
  self: ForceElement,
  enemy: ForceElement,
  known: readonly ForceElement[],
  config: RtConfig,
): { at: LatLng; rangeM: number; cover: boolean; seenBy: number } | null {
  const weapon = weaponFor(self, enemy, 500);
  if (!weapon) return null;
  // Inside 1 km, where fire gets markedly more accurate and more lethal
  // (fire.ts), and inside the weapon's short range and half its maximum.
  const rangeM = Math.max(400, Math.min(900, weapon.shortRangeM * 0.9, (weapon.maxRangeM / 2) * 0.9));
  if (distanceM(self.position, enemy.position) <= rangeM + 250) return null;
  const bearing = bearingDeg(enemy.position, self.position);
  const seenBy = (at: LatLng) =>
    known.filter((other) => distanceM(other.position, at) <= 3000 && lineOfSight(config.terrain, { from: other.position, to: at }).visible)
      .length;
  const best = [-60, -40, -20, 0, 20, 40, 60]
    .map((delta) => offsetBy(enemy.position, (bearing + delta + 360) % 360, rangeM))
    .filter((at) => allowanceAt(self, at, config) > 0)
    .filter((at) => lineOfSight(config.terrain, { from: at, to: enemy.position }).visible)
    .map((at) => ({ at, cover: inCover(config.terrain, at), seenBy: seenBy(at) }))
    .sort(
      (a, b) =>
        distanceM(self.position, a.at) / 1000 - (a.cover ? 1 : 0) + a.seenBy * 0.5 -
        (distanceM(self.position, b.at) / 1000 - (b.cover ? 1 : 0) + b.seenBy * 0.5),
    )[0];
  return best ? { ...best, rangeM } : null;
}
