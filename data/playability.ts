// ── bgws/data/playability.ts ───────────────────────────────────────────────
// Can this catalogue entry be fielded, and if not, why not?
//
// THE ASSET EXPLORER AND THE GAME WANT DIFFERENT THINGS, and pretending
// otherwise is how a scenario ends up containing a unit that cannot fire.
//
// The Explorer's job is to show the whole catalogue: 1,239 platforms, however
// thin the data. The game's job is to field things that work. Of those 1,239,
// 819 are playable and 420 are not — including every single soft-skin and
// every single foot platform, which carry a name, an id and nothing else.
//
// So a platform is not "missing data" in some vague sense; it fails a
// specific test, and the test names which one. A greyed-out row in the
// Explorer that says "no weapon ranges" is a fact about the source. A greyed
// out row with no reason is a bug report waiting to be filed.

import type { PlatformProfile } from "./profiles";

export type UnplayableReason =
  | "noMoveType"
  | "noTargetClass"
  | "noCombatStrength"
  | "noCapabilities"
  | "noWeaponRanges";

export const UNPLAYABLE_EXPLANATION: Record<UnplayableReason, string> = {
  noMoveType: "no movement type, so it cannot be placed or moved",
  noTargetClass: "no target class, so nothing knows what may shoot at it (2.1.8)",
  noCombatStrength: "combat strength index of zero, so it cannot fight or be hurt",
  noCapabilities: "no weapons listed at all",
  noWeaponRanges: "no weapon has a range, so it could never fire a shot",
};

export interface Playability {
  playable: boolean;
  /** Every test it fails, not just the first — the Explorer shows them all. */
  reasons: UnplayableReason[];
}

/**
 * The gate.
 *
 * Deliberately about CAPABILITY rather than completeness score. A platform
 * with a low confidence score but real ranges is playable and its figures are
 * soft; a platform with no ranges is not playable at any confidence. Those
 * are different problems and conflating them would hide the second behind the
 * first.
 *
 * (In this catalogue the two happen to coincide exactly — all 819 playable
 * rows score 1.0 and every unplayable one scores 0.3-0.4 — but that is a fact
 * about today's data, not a rule, and the gate should not depend on it.)
 */
export function playabilityOf(profile: {
  moveType?: PlatformProfile["moveType"] | null;
  targetClass?: PlatformProfile["targetClass"] | null;
  csIndex?: number | null;
  capabilities?: { maxRangeM?: number | null }[] | null;
}): Playability {
  const reasons: UnplayableReason[] = [];

  if (!profile.moveType) reasons.push("noMoveType");
  if (!profile.targetClass) reasons.push("noTargetClass");
  if (profile.csIndex == null || profile.csIndex <= 0) reasons.push("noCombatStrength");

  const armed = (profile.capabilities ?? []).some(
    (capability) => capability.maxRangeM != null && capability.maxRangeM > 0,
  );
  if (!armed) reasons.push("noWeaponRanges");

  return { playable: reasons.length === 0, reasons };
}

/**
 * The same question asked of a CATALOGUE row rather than a game platform.
 *
 * `PlatformProfile` carries capability CLASSES — "atk", "apers" — and not the
 * ranges, which live in the capability profile dataset. So the catalogue-side
 * check can only ask whether a platform has any weapon at all, and says so in
 * its own words rather than borrowing "no weapon has a range" and implying a
 * check it did not make.
 *
 * In this data the two agree exactly: every row with no capability classes
 * also has no ranges and a zero combat strength. That is a fact about today's
 * catalogue, not a guarantee, which is why they are separate functions.
 */
export function playabilityOfProfile(profile: {
  moveType?: PlatformProfile["moveType"] | null;
  targetClass?: PlatformProfile["targetClass"] | null;
  csIndex?: number | null;
  capabilities?: readonly string[] | null;
}): Playability {
  const reasons: UnplayableReason[] = [];

  if (!profile.moveType) reasons.push("noMoveType");
  if (!profile.targetClass) reasons.push("noTargetClass");
  if (profile.csIndex == null || profile.csIndex <= 0) reasons.push("noCombatStrength");
  if ((profile.capabilities ?? []).length === 0) reasons.push("noCapabilities");

  return { playable: reasons.length === 0, reasons };
}

/** One line for the Asset Explorer. */
export function describePlayability(playability: Playability): string {
  if (playability.playable) return "Playable";
  return `Catalogue only — ${playability.reasons
    .map((reason) => UNPLAYABLE_EXPLANATION[reason])
    .join("; ")}`;
}
