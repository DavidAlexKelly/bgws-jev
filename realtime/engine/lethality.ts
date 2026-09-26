// ── bgws/realtime/engine/lethality.ts ──────────────────────────────────────
// What happens when a round arrives: hit → penetrate → kill, vehicle by vehicle.
//
// The fire table still says how many rounds STRIKE (its "hits", with the range
// bands in fire.ts). This file says what each strike does, from the data:
//
//   1. INTERCEPT   an active protection system may stop a missile or rocket
//   2. PENETRATE   the round's penetration AT THIS RANGE (the munition's
//                  curve) against the armour on the face it strikes — front,
//                  turret front when hull-down, side, rear, or roof for top
//                  attack — worked out from where the firer is and which way
//                  the target faces. A soft curve around the margin, not the
//                  turn game's hard gate, which deadlocked peer tanks.
//   3. KILL        a penetration knocks the vehicle out with a probability
//                  by target class. DECLARED: the L7 survivability band is for
//                  aircraft and is null for every land platform.
//
// Every figure that is ours rather than the source's is marked DECLARED.

import { bearingDeg, bearingDeltaDeg, type LatLng } from "../../lib/board";
import type { Capability, ForceElement } from "../../lib/state";
import type { Rng } from "../../rules/dice";
import type { RuleSet } from "../../rules/ruleset";

export type Aspect = "front" | "turret" | "side" | "rear" | "roof";

/** DECLARED. Kinetic penetration lost per km beyond the last known point — the L27A1 curve's own slope (~6%/km). */
const KE_FALLOFF_PER_KM = 0.06;
/** DECLARED. Where the source gives only frontal armour: side, rear and roof as fractions of it (Challenger 2's ratios). */
const ASPECT_FRACTION: Record<Exclude<Aspect, "front" | "turret">, number> = { side: 0.2, rear: 0.06, roof: 0.045 };
/** DECLARED. How soft the penetration margin is: at ±12% of the armour the chance is ~27% / ~73%. */
const PEN_SPREAD = 0.12;
/** DECLARED. Chance a penetration knocks the vehicle out, by what it is. */
const P_KILL: Record<string, number> = { armoured_vehicle: 0.6, default: 0.85 };
/** DECLARED. Chance an active protection system defeats one missile or rocket. */
const P_APS_INTERCEPT = 0.5;
/** As the rules have it (tandemEraDefeatFraction): a tandem warhead strips this much of ERA-fitted armour. */
const TANDEM_ERA_STRIP = 0.4;
/** A shot this far round from the target's facing arrives on the rear. */
const REAR_ARC_DEG = 150;

/** Penetration at a range: the munition's curve where there is one, else the 1 km figure. */
export function penetrationAt(capability: Capability, rangeM: number): number | undefined {
  const kinetic = capability.munition == null || capability.munition === "ke";
  const curve = capability.penetrationCurveMm;
  if (curve && curve.length) {
    if (rangeM <= curve[0].rangeM) return curve[0].mm;
    for (let i = 1; i < curve.length; i += 1) {
      const b = curve[i];
      if (rangeM <= b.rangeM) {
        const a = curve[i - 1];
        return a.mm + ((b.mm - a.mm) * (rangeM - a.rangeM)) / (b.rangeM - a.rangeM);
      }
    }
    const last = curve[curve.length - 1];
    return kinetic ? last.mm * Math.max(0, 1 - ((rangeM - last.rangeM) / 1000) * KE_FALLOFF_PER_KM) : last.mm;
  }
  if (capability.penetrationMm == null) return undefined;
  // A shaped charge's jet does not weaken with range; a long rod does.
  if (!kinetic) return capability.penetrationMm;
  return capability.penetrationMm * Math.max(0, 1 - ((rangeM - 1000) / 1000) * KE_FALLOFF_PER_KM);
}

/** Which face of the target a round from `from` strikes. */
export function aspectOf(
  target: ForceElement,
  from: LatLng,
  ruleset: RuleSet,
  options: { topAttack?: boolean; hullDown?: boolean } = {},
): Aspect {
  if (options.topAttack) return "roof";
  if (target.facing == null) return options.hullDown ? "turret" : "front";
  const delta = bearingDeltaDeg(target.facing, bearingDeg(target.position, from));
  if (delta <= ruleset.frontArcDeg) return options.hullDown ? "turret" : "front";
  if (delta >= REAR_ARC_DEG) return "rear";
  return "side";
}

/** The armour on that face, against that kind of round. Undefined: nothing known. */
export function armourOn(target: ForceElement, aspect: Aspect, shapedCharge: boolean): number | undefined {
  const a = target.armour;
  const pick = (ke?: number, ce?: number) => (shapedCharge ? (ce ?? ke) : ke);
  if (a) {
    const front = pick(a.frontKeMm, a.frontCeMm);
    switch (aspect) {
      case "front":
        return front ?? target.armourMm;
      case "turret":
        return pick(a.turretFrontKeMm, a.turretFrontCeMm) ?? front ?? target.armourMm;
      case "side":
        return pick(a.sideKeMm, a.sideCeMm);
      case "rear":
        return pick(a.rearKeMm, a.rearCeMm);
      case "roof":
        return pick(a.roofKeMm, a.roofCeMm);
    }
  }
  if (target.armourMm == null) return undefined;
  return aspect === "front" || aspect === "turret" ? target.armourMm : target.armourMm * ASPECT_FRACTION[aspect];
}

export interface StrikeOdds {
  aspect: Aspect;
  penetrationMm?: number;
  armourMm?: number;
  pIntercept: number;
  pPenetrate: number;
  pKill: number;
  /** Chance one strike knocks a vehicle out. */
  pKnockOut: number;
}

/** The odds of one round that strikes this target, from `from`, with this weapon. */
export function strikeOdds(
  capability: Capability,
  target: ForceElement,
  from: LatLng,
  rangeM: number,
  ruleset: RuleSet,
  hullDown = false,
): StrikeOdds {
  const shaped = capability.munition === "ce" || capability.munition === "ceTandem";
  const aspect = aspectOf(target, from, ruleset, { topAttack: capability.topAttack, hullDown });
  const penetration = penetrationAt(capability, rangeM);
  let armour = target.targetClass === "armoured_vehicle" ? armourOn(target, aspect, shaped) : undefined;
  if (armour != null && capability.munition === "ceTandem" && target.eraFitted) armour *= 1 - TANDEM_ERA_STRIP;
  // Unknown on either side fails open, as the turn game's penetration rule does.
  const pPenetrate =
    penetration == null || armour == null || armour <= 0
      ? 1
      : 1 / (1 + Math.exp(-(penetration - armour) / (PEN_SPREAD * armour)));
  const pIntercept = shaped && target.apsFitted ? P_APS_INTERCEPT : 0;
  const pKill = P_KILL[target.targetClass] ?? P_KILL.default;
  return {
    aspect,
    ...(penetration != null ? { penetrationMm: Math.round(penetration) } : {}),
    ...(armour != null ? { armourMm: Math.round(armour) } : {}),
    pIntercept,
    pPenetrate,
    pKill,
    pKnockOut: (1 - pIntercept) * pPenetrate * pKill,
  };
}

export type StrikeResult = "intercepted" | "noPenetration" | "survived" | "knockedOut";

/** Roll one strike. Always three draws, in a fixed order, so a replay is exact. */
export function rollStrike(odds: StrikeOdds, rng: Rng): StrikeResult {
  const u = () => rng.int(1_000_000) / 1_000_000;
  const intercept = u();
  const penetrate = u();
  const kill = u();
  if (intercept < odds.pIntercept) return "intercepted";
  if (penetrate >= odds.pPenetrate) return "noPenetration";
  return kill < odds.pKill ? "knockedOut" : "survived";
}

/** "front, 620 vs 700 mm" — for the feed and for Jev. */
export function describeStrike(odds: StrikeOdds): string {
  const mm = odds.penetrationMm != null && odds.armourMm != null ? `, ${odds.penetrationMm} vs ${odds.armourMm} mm` : "";
  return `${odds.aspect === "turret" ? "turret front (hull-down)" : odds.aspect}${mm}`;
}
