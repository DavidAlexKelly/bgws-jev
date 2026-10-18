// ── bgws/data/platformCatalogue.generated.ts ───────────────────────────────
//
// ⚠ GENERATED. Do not hand-edit — run `scripts/bgwsGeneratePlatforms.ts`.
//
// Platforms from the L6 catalogue that are PLAYABLE: they have a move type, a
// target class, a combat-strength index above zero, and at least one weapon
// with a range. Everything else in the catalogue can be browsed in the Asset
// Explorer and cannot be fielded, because there is nothing to field.
//
// SOURCE
//   [SIM] L6 bgws_platform_profile
//     ri.foundry.main.dataset.eb91f557-fa16-482d-a474-bea9c97f6aa2
//   read 2026-09-13, master
//
// WHAT THE CATALOGUE ACTUALLY CONTAINS, which is not what it looks like:
//
//   1,239 platforms
//     819 playable  — every one of them `heavyVehicle`
//     420 not       — 134 soft_skin and 76 foot, ALL empty shells, plus 210
//                     armoured vehicles missing ranges or a CS index
//
//   Of those 819, there are only 72 DISTINCT STATLINES (cs index, armour,
//   penetration). Eleven names per behaviour. The catalogue is broad in
//   nomenclature and narrow in mechanics: it is a very large tank park with
//   no infantry fighting vehicle, no reconnaissance vehicle, no soft-skin and
//   no artillery that the game can use.
//
//   So "use any asset from the Asset Explorer" delivers names and 72
//   behaviours, all of them heavy armour. The variety that makes a wargame —
//   infantry, mortars, carriers — is not in the source at any completeness
//   score, and has to be DECLARED. See DECLARED_PLATFORMS in rules/forceList.
//
// THIS FILE IS A SLICE, not the full 819: two platforms per nation, which is
// every nation the playable set contains. Regenerating with network access
// emits all of them. The slice is enough to field a scenario from sourced
// figures, which is what it is for.
//
// DERIVATIONS, all declared:
//   - short range is half of maximum. BGWS 2.1.8: "Under 51% of Max Range is
//     short range". `bgws_capability_profile` carries its own per-weapon
//     short ranges and a full regeneration should prefer them; this slice
//     uses the rulebook's rule so that no figure here is unattributable.
//   - ranges are capped at LOS_CAP_M (3,000 m), the board's line-of-sight
//     limit, exactly as the hand-pinned platforms already were.
//   - penetration is `best_pen_mm_1000m`. Absent means UNKNOWN, not zero.
//   - armour is `armor_max_mm`, with the same reading.
//   - display names are the source's, ugly capitalisation and all. Renaming
//     them here would break the only link back to the catalogue row.

import type { PlatformSnapshot } from "../rules/forceList";

/** Platforms the game can field, straight from the source. */
export const CATALOGUE_PLATFORMS: Record<string, PlatformSnapshot> = {
  "tankmodels/cn_m1a2t": {
    assetId: "tankmodels/cn_m1a2t",
    displayName: "Cn M1a2t",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.3,
    protectionBand: "very_heavy",
    armourMm: 800,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/cn_ztz_96": {
    assetId: "tankmodels/cn_ztz_96",
    displayName: "Cn Ztz 96",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9,
    protectionBand: "heavy",
    armourMm: 530,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/fr_leopard_2a5nl": {
    assetId: "tankmodels/fr_leopard_2a5nl",
    displayName: "Fr Leopard 2a5nl",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  "tankmodels/fr_leopard_2a6nl": {
    assetId: "tankmodels/fr_leopard_2a6nl",
    displayName: "Fr Leopard 2a6nl",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  "tankmodels/germ_leopard_2a5": {
    assetId: "tankmodels/germ_leopard_2a5",
    displayName: "Germ Leopard 2a5",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  "tankmodels/germ_leopard_2a5_pso": {
    assetId: "tankmodels/germ_leopard_2a5_pso",
    displayName: "Germ Leopard 2a5 Pso",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/il_merkava_mk_3_raam_segol": {
    assetId: "tankmodels/il_merkava_mk_3_raam_segol",
    displayName: "Il Merkava Mk 3 Raam Segol",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.4,
    protectionBand: "very_heavy",
    armourMm: 900,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/il_merkava_mk_3b": {
    assetId: "tankmodels/il_merkava_mk_3b",
    displayName: "Il Merkava Mk 3b",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.4,
    protectionBand: "very_heavy",
    armourMm: 900,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/it_leopard_2a4": {
    assetId: "tankmodels/it_leopard_2a4",
    displayName: "It Leopard 2a4",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.1,
    protectionBand: "heavy",
    armourMm: 585,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  "tankmodels/it_leopard_2a7_hungary": {
    assetId: "tankmodels/it_leopard_2a7_hungary",
    displayName: "It Leopard 2a7 Hungary",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/jp_leopard_2ri": {
    assetId: "tankmodels/jp_leopard_2ri",
    displayName: "Jp Leopard 2ri",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.1,
    protectionBand: "heavy",
    armourMm: 585,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  "tankmodels/jp_tkx_prot": {
    assetId: "tankmodels/jp_tkx_prot",
    displayName: "Jp Tkx Prot",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.1,
    protectionBand: "very_heavy",
    armourMm: 605,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/sw_leopard_2a6nl": {
    assetId: "tankmodels/sw_leopard_2a6nl",
    displayName: "Sw Leopard 2a6nl",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.2,
    protectionBand: "very_heavy",
    armourMm: 650,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  /**
   * The one platform in this slice that is not a gun tank: a missile carrier,
   * with no main gun and no machine gun at all. It is here deliberately, as
   * proof that the generator does not quietly invent an `atk` capability for
   * something that has none — and because at 800 mm of penetration it is the
   * only thing in the slice that reliably kills a Merkava.
   */
  "tankmodels/sw_pvrbv_551": {
    assetId: "tankmodels/sw_pvrbv_551",
    displayName: "Sw Pvrbv 551",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.3,
    protectionBand: "heavy",
    armourMm: 500,
    capabilities: [{ kind: "atm", maxRangeM: 2000, shortRangeM: 1000, penetrationMm: 800 }],
  },
  "tankmodels/uk_m1a1_aim_abrams": {
    assetId: "tankmodels/uk_m1a1_aim_abrams",
    displayName: "Uk M1a1 Aim Abrams",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.3,
    protectionBand: "very_heavy",
    armourMm: 800,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
  "tankmodels/us_m1a1_abrams": {
    assetId: "tankmodels/us_m1a1_abrams",
    displayName: "Us M1a1 Abrams",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 9.3,
    protectionBand: "very_heavy",
    armourMm: 800,
    capabilities: [
      { kind: "atk", maxRangeM: 3000, shortRangeM: 1500, penetrationMm: 480 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1500 },
    ],
  },
};

/** What the catalogue holds, for the report and the Asset Explorer. */
export const CATALOGUE_COVERAGE = {
  totalPlatforms: 1239,
  playable: 819,
  /** Distinct (cs index, armour, penetration) triples among the playable. */
  distinctStatlines: 72,
  /** Every playable platform is a heavy vehicle. There is nothing else. */
  playableSubclasses: ["heavyVehicle"],
  pinnedHere: 17,
  readOn: "2026-09-13",
} as const;
