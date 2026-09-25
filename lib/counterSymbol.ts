// ── bgws/lib/counterSymbol.ts ──────────────────────────────────────────────
// A counter for a platform that has no symbol of its own.
//
// THIS IS A PLACEHOLDER, AND THE COMMENT MATTERS MORE THAN THE CODE.
//
// The [SIM] L6 profiles describe EQUIPMENT: a hull, its protection, what it
// can shoot and how far. A SIDC describes a UNIT: an affiliation, an echelon
// and a role. Those are different things, and the equipment cannot answer the
// unit's questions — a Challenger 2 profile does not know whether it is one
// tank, a troop of four or a squadron of fourteen, and echelon is most of what
// a counter communicates.
//
// So a real Force Element's SIDC comes from the ORBAT (src/shared/orbat), and
// this module exists only so a platform picked out of the equipment list can
// be dropped on the map and seen during the spike. When the ORBAT is wired in,
// this should be deleted rather than improved.
//
// Pure: takes a profile-shaped value, returns a SIDC string.

import type { MoveType, PlatformProfile } from "../data/profiles";

export type Side = "blue" | "red";

/** MIL-STD-2525C affiliation: F = friend, H = hostile. */
function affiliation(side: Side): string {
  return side === "red" ? "H" : "F";
}

/**
 * Function ID for the icon, chosen from what the platform can do rather than
 * from its name — the source's names are a game tech tree ("Uk M1a2 Sep3
 * Abrams") and cannot be trusted to say what something is.
 */
function functionId(platform: PlatformProfile): string {
  const capabilities = new Set(platform.capabilities);

  // Order matters: a vehicle with an ATGM is an anti-tank asset first, whatever
  // else it carries.
  if (capabilities.has("atm")) return "UCAA--"; // anti-tank
  if (capabilities.has("aa")) return "UCD---"; // air defence
  if (platform.moveType === "F") return "UCI---"; // infantry
  if (capabilities.has("atk") && platform.moveType === "T") return "UCA---"; // armour
  if (platform.moveType === "W") return "UCR---"; // reconnaissance / wheeled
  return "UC----"; // combat, unspecified
}

/**
 * A 15-character MIL-STD-2525C SIDC, which is what milsymbol renders.
 *
 * Echelon (position 11) is left as '-'. A platform has no echelon, and
 * guessing one would put a company's bar over a single vehicle.
 */
export function counterSidc(platform: PlatformProfile, side: Side): string {
  return `S${affiliation(side)}G*${functionId(platform)}-----`.slice(0, 15).padEnd(15, "-");
}

/** Short label for the counter. The source's names are long and noisy. */
export function counterLabel(platform: PlatformProfile): string {
  // "Uk Challenger 2 Tes" → "Challenger 2 Tes": the nation prefix is a game
  // tech-tree marker, not part of the equipment's name, and it is wrong often
  // enough ("Uk M1a2 Abrams") to be worth dropping.
  const withoutNationPrefix = platform.displayName.replace(
    /^(Uk|Us|Usa|Germ|Ussr|Cn|Jp|It|Fr|Sw|Il|Au)\s+/i,
    "",
  );
  return withoutNationPrefix.length > 22
    ? `${withoutNationPrefix.slice(0, 21)}…`
    : withoutNationPrefix;
}

/** Move Type, spelled out for a tooltip or a chart. */
export function moveTypeLabel(moveType: MoveType): string {
  if (moveType === "F") return "Foot";
  if (moveType === "W") return "Wheeled";
  return "Tracked";
}
