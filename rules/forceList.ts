// ── bgws/rules/forceList.ts ────────────────────────────────────────────────
// WHO IS ON THE BOARD. Named, versioned, and readable.
//
// This file exists because of one honest split that the rest of the platform
// depends on:
//
//   COMBAT STRENGTH IS DERIVABLE.  It comes from equipment, and equipment is
//   real data — bgws_cs_index on the L6 platform profile, multiplied by how
//   many platforms are in the sub-unit.
//
//   TROOP QUALITY IS NOT.  Nothing in the source knows whether a crew is
//   conscript or elite. It cannot be derived from a JSON file about a tank.
//   It is a DOCTRINAL ASSERTION about the scenario, so it is declared here,
//   in the open, with a name and a version — not left as a constant in a
//   module where it decides outcomes unseen.
//
// Before this file, every Force Element on the board was one platform with
// troopQuality hardcoded to 4 and every weapon range flattened to 3,000 m.
// The rules engine was complete and being fed a fiction, which meant the
// harness could measure a mechanic faithfully and still tell you nothing.
//
// WHY THE PLATFORM FIGURES ARE PINNED HERE RATHER THAN QUERIED
// ------------------------------------------------------------
// A batch result is only meaningful if you know which platform numbers
// produced it. If the harness read live datasets, a rebuild upstream would
// silently change last week's findings and there would be no way to tell a
// rule's effect from a data revision. So the figures are copied in, with the
// dataset RIDs and the date they were read. Refreshing them is a visible
// commit, which is the point.

import { CATALOGUE_PLATFORMS } from "../data/platformCatalogue.generated";
import type { MoveType, TargetClass } from "../data/profiles";
import type { ArmourByAspect, Capability, Side } from "../lib/state";

/**
 * The Troop Quality scale. OURS, not BGWS's.
 *
 * Named rather than numeric at the call site so a force list reads as a claim
 * about troops ("regular") instead of an unexplained 4.
 */
export const TROOP_QUALITY = {
  conscript: 1,
  regular: 4,
  veteran: 6,
  elite: 8,
} as const;

export type TroopQualityName = keyof typeof TROOP_QUALITY;

export interface PlatformSnapshot {
  assetId: string;
  displayName: string;
  moveType: MoveType;
  targetClass: TargetClass;
  /** bgws_cs_index — per platform, for ONE vehicle. */
  csIndex: number;
  protectionBand: string;
  /**
   * Frontal armour in millimetres, from `armor_max_mm` on the L6 profile.
   *
   * Undefined means the source does not record it, which is NOT the same as
   * unarmoured — see PenetrationRule.
   */
  armourMm?: number;
  /**
   * Armour by aspect, where it is known.
   *
   * ⚠ WITHOUT THIS THE facingArmour RULE IS UNREACHABLE. A resolver that can
   * read a side facing is worth nothing if no force element ever carries one
   * — the same failure as a rule that exists but that no sequence of play can
   * reach. Figures below are the curated L7 protection table.
   */
  armour?: ArmourByAspect;
  /** Explosive reactive armour is fitted; only tandem warheads care. */
  eraFitted?: boolean;
  /** L7 `statcard_speed_kmh`. Read by the real-time mode only. */
  speedKmh?: number;
  /** L7 `hp_per_tonne`. Read by the real-time mode only. */
  hpPerTonne?: number;
  /** L7 `aps_fit` is something other than "none". Read by the real-time mode only. */
  apsFitted?: boolean;
  /**
   * Where the figures came from, when they came from a source.
   *
   * ⚠ THE COUNTERPART TO valuesDeclared, AND IT IS NEW BECAUSE THE SITUATION
   * IS NEW. Until now every non-armour platform in this file was invented,
   * because the simulator catalogue models no infantry, so "not a vehicle"
   * and "figures are ours" were the same statement. The curated section
   * profile breaks that: these are dismounted troops with published tables of
   * organisation. A row must say one or the other, and the catalogue test
   * enforces it.
   */
  sourcedFrom?: string;
  capabilities: Capability[];
  /**
   * Set when this platform stands in for one the source does not model.
   *
   * There are no Soviet or Russian MBTs in the asset files — no T-90M, no
   * T-14, no T-64. Only NATO and export operator models. A red force
   * therefore fields proxies, and saying so on the row is the difference
   * between a known limitation and a quiet lie.
   */
  proxyFor?: string;
  /**
   * Set when the SOURCE NAMES THIS PLATFORM BUT MODELS NOTHING ABOUT IT, so
   * every figure below is ours.
   *
   * Different from `proxyFor`, and the difference matters. A proxy is a real
   * modelled platform standing in for one the source lacks — a T-72M1 for a
   * T-72B3 — so its numbers are somebody's measurements of something. This is
   * the other case: the asset exists in the catalogue with a name and an id
   * and nothing else.
   *
   * The 76 `foot` platforms in [SIM] L6 bgws_platform_profile are all like
   * this: `bgws_capabilities` null, `bgws_cs_index` 0, every range null,
   * `protection_band` unknown, `data_completeness_score` 0.3-0.4. They are
   * weapon-preset shells from an asset dump, not modelled infantry.
   *
   * So infantry could not be sourced. It could be DECLARED, which is what
   * this field is for — the same treatment troop quality and the force
   * management values already get, and the report prints it beside the
   * proxies.
   */
  valuesDeclared?: string;
}

/**
 * Platform figures as read from the L6 profiles.
 *
 * Pinned from:
 *   [SIM] L6 bgws_platform_profile   ri.foundry.main.dataset.eb91f557-fa16-482d-a474-bea9c97f6aa2
 *   [SIM] L6 bgws_capability_profile ri.foundry.main.dataset.4d28fc58-59b3-4814-b5cf-7754054f9dac
 *   read 2026-09-12, master
 *
 * Ranges are bgws_max_range_m / short_range_m, which are already capped to
 * board scale by the pipeline — a Challenger's 120 mm declares 20,000 m and
 * is carried here as 3,000, because that is the cap the game plays at.
 */
/**
 * Platforms pinned BY HAND, which override the generated catalogue.
 *
 * Two different kinds live here and the difference matters:
 *
 *   SOURCED BY HAND  Challenger 2, Warrior, the T-72M1 and the rest were read
 *                    out of the L6 profiles by a person before the generator
 *                    existed. Their figures are the source's. They stay
 *                    pinned because every force list and every number in
 *                    reports/ refers to them, and regenerating underneath a
 *                    published baseline would quietly change what the reports
 *                    are about.
 *   DECLARED         Rifle Infantry and the Mortar Section. The source models
 *                    neither — 76 foot rows that are empty shells, and no
 *                    mortar at all — so every figure is ours. Each says so in
 *                    `valuesDeclared`, and the sweep prints it.
 *
 * Spread AFTER the generated catalogue, so a regeneration can never silently
 * delete either kind.
 */
export const PINNED_PLATFORMS: Record<string, PlatformSnapshot> = {
  /**
   * DISMOUNTED INFANTRY. Every number here is OURS — see `valuesDeclared`.
   *
   * Infantry is the reason three rules could not be reached: `apers` and
   * `atk` target classes mean nothing in an all-armour game (9.2.1),
   * `defenderIsVehicleOnly` needs somebody who is not a vehicle (9.3.4), and
   * a tank's coaxial machine gun has nothing to shoot at.
   *
   * The figures are deliberately modest and deliberately shaped: short
   * ranges, a section that is weak in the open and dangerous close in with
   * a missile. A section of three is CS 3 against a Challenger troop's 10,
   * which is about right — it loses a firefight at a kilometre and can kill
   * a tank at four hundred metres.
   */
  "infantry/rifle_infantry": {
    assetId: "infantry/rifle_infantry",
    displayName: "Rifle Infantry",
    moveType: "F",
    targetClass: "foot",
    csIndex: 2,
    // Not "soft_skin": that band is for vehicles with no armour, and it would
    // make a dug-in section easier to kill than a Land Rover. Dispersed men
    // are hard to destroy wholesale even though they are easy to hurt.
    protectionBand: "light",
    // No armourMm on purpose. Undefined means unknown and fails open, which
    // is the honest reading — "how many millimetres is an infantryman" is not
    // a question with an answer.
    capabilities: [
      // Listed missile-first: against a tank it is the only thing that works,
      // and weaponFor takes the first capability that reaches AND applies.
      { kind: "atm", munition: "ce", topAttack: true, maxRangeM: 800, shortRangeM: 400, penetrationMm: 500 },
      { kind: "apers", maxRangeM: 600, shortRangeM: 300 },
    ],
    valuesDeclared:
      "the source names 76 foot platforms and models none of them — null " +
      "capabilities, cs_index 0, no ranges, completeness 0.3-0.4 — so the id " +
      "and display name are the source's and every figure is ours",
  },
  /**
   * A MORTAR SECTION. Everything here is ours, including the id.
   *
   * Infantry at least had names in the catalogue. This has nothing: a query
   * for any platform whose id or display name contains "mortar" returns zero
   * rows. The source models no indirect fire weapon of any kind.
   *
   * So the asset id is deliberately prefixed `declared/` rather than dressed
   * up to look like a catalogue entry — anybody grepping the asset files for
   * it should fail to find it, and should fail in a way that explains itself.
   *
   * Short-legged on purpose: 3,000 m reaches across a third of the board, so
   * a mortar has to be positioned rather than parked. It carries smoke as
   * well as HE, which is the whole reason indirect fire was worth building.
   */
  "declared/mortar_section": {
    assetId: "declared/mortar_section",
    displayName: "Mortar Section",
    moveType: "W",
    targetClass: "soft_skin",
    csIndex: 3,
    protectionBand: "soft_skin",
    capabilities: [
      { kind: "idf", maxRangeM: 3000, shortRangeM: 1500 },
      { kind: "smoke", maxRangeM: 3000, shortRangeM: 1500 },
      // It can defend itself, badly, and only against people on foot.
      { kind: "apers", maxRangeM: 400, shortRangeM: 200 },
    ],
    valuesDeclared:
      "the catalogue contains no mortar, no artillery piece and no attack " +
      "helicopter — a search for 'mortar' returns zero rows — so the id, the " +
      "name and every figure are ours",
  },
  // ── Fighting vehicles, from the curated L7 platform profile ──────────────
  //
  // ⚠ KEYED BY CURATED ASSET ID, NOT BY THE SIMULATOR'S. Everything below is
  // sourced, so it is keyed the way the source keys it — `var_11_default`,
  // not `var_11_default`. Three reasons, in order of how much
  // trouble each one has already caused:
  //
  //  1. THE OLD IDS WERE LYING. `var_22_1` held figures for a
  //     T-72B3, because the simulator modelled no T-72B3 and the entry stood
  //     in for one. The source now models one. An id that names a different
  //     vehicle from the one whose numbers it carries is a trap.
  //  2. Proxies are retired rather than re-labelled. `proxyFor` is still in
  //     the interface and still correct for anything that needs it; these two
  //     no longer do.
  //  3. The curated explorer shows `var_11_default`. Now a player can search
  //     for the thing they just fought.
  //
  // Armour, Combat Strength, protection band and ERA come from the profile.
  // CAPABILITY RANGES DO NOT, and that is deliberate — see the note on
  // `capabilities` below.
  "var_11_default": {
    assetId: "var_11_default",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_11_default)",
    displayName: "Challenger 2",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 8.6,
    protectionBand: "heavy",
    // Was 400, which was neither the hull figure nor the turret one. The
    // profile's armor_max_mm is the hull front, and the aspect table below
    // already agreed with it — so this field alone was out of step.
    armourMm: 700,
    armour: {
      frontKeMm: 700,
      frontCeMm: 1000,
      turretFrontKeMm: 950,
      turretFrontCeMm: 1300,
      sideKeMm: 140,
      sideCeMm: 400,
      rearKeMm: 40,
      rearCeMm: 60,
      roofKeMm: 30,
      roofCeMm: 30,
    },
    eraFitted: true,
    // L7 row: statcard_speed_kmh 59, hp_per_tonne 19.2, aps_fit none.
    speedKmh: 59,
    hpPerTonne: 19.2,
    apsFitted: false,
    // ⚠ RANGES ARE OURS AND STAY OURS. The profile says 4,000 m for the
    // L30A1 and the board is 1,600 m deep. Adopting source ranges would let
    // every tank engage across the whole map from its start line, which is a
    // change to what the game IS, not to how well it is informed. Worth doing
    // with the board size, not smuggled in with the armour.
    //
    // PENETRATION IS THE SOURCE'S, and this one closed a real hole: the
    // Challenger stated no figure at all, and `canPenetrate` fails OPEN, so
    // the gun that is meant to be the sharp end of the blue force was
    // defeating every armour value in the game unconditionally.
    capabilities: [
      {
        kind: "atk",
        munition: "ke",
        maxRangeM: 3000,
        shortRangeM: 3000,
        penetrationMm: 657,
        // L7 bgws_munition_profile m_l27a1 (L27A1 CHARM 3 APFSDS).
        penetrationCurveMm: [
          { rangeM: 0, mm: 676 },
          { rangeM: 1000, mm: 657 },
          { rangeM: 2000, mm: 620 },
          { rangeM: 3000, mm: 583 },
        ],
      },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1000 },
    ],
  },
  /**
   * THE FV510, NOT THE DESERT WARRIOR — a different vehicle, and the swap is
   * the largest single correction in this pass.
   *
   * ⚠ THE OLD ENTRY WAS AN EXPORT MODEL BRITAIN NEVER FIELDED. The simulator
   * catalogue only had `uk_desert_warrior`: the Kuwaiti Desert Warrior, with
   * a Delco turret and twin TOW. The entry displayed as plain "Warrior" and
   * sat in a British battlegroup, so a UK armoured infantry platoon has been
   * fighting with a tandem-warhead ATGM it does not own.
   *
   * That single borrowed weapon is why the upstream index rated it 8.5
   * against a Challenger's 8.6, and why `combatStrengthFor` needed a
   * protection term to pull it back down. The curated profile rates the real
   * FV510 at 4.3 and bands it `light`, so the house rule no longer has to
   * correct the input — it just weights it.
   *
   * Consequence to be clear about: blue loses an anti-tank missile from the
   * meeting engagement. It keeps the 30 mm, which cannot defeat a T-72 hull
   * and is not supposed to. Blue's anti-armour answer is the Challengers and
   * `sec_uk_javelin`, both of which are real.
   */
  "var_27_default": {
    assetId: "var_27_default",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_27_default)",
    displayName: "FV510 Warrior",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 4.3,
    protectionBand: "light",
    armourMm: 40,
    armour: { frontKeMm: 40, frontCeMm: 40, sideKeMm: 20, sideCeMm: 20, rearKeMm: 15, rearCeMm: 15, roofKeMm: 12, roofCeMm: 12 },
    capabilities: [
      // ⚠ THE ONE PENETRATION FIGURE THIS PASS ADDS, AND IT IS NOT OPTIONAL.
      // `defeatsArmour` fails OPEN: a capability with no penetrationMm
      // defeats anything it is allowed to shoot at. Taking the TOW away and
      // leaving the RARDEN blank would not have made the Warrior stop killing
      // tanks — it would have made it kill them with the autocannon instead,
      // and the claim in the note above would have been false.
      //
      // 50 mm is the profile's best_pen_mm_1000m for var_27_default. It
      // defeats a BTR and bounces off everything this scenario fields, which
      // is the point of putting it there.
      { kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 2500, penetrationMm: 50 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 2500 },
    ],
  },
  // The two below exist for GENERATION_PROBE_V1: the most and least modern
  // tanks in the source that are still directly comparable.
  "var_1_default": {
    assetId: "var_1_default",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_1_default, M1A2 SEPv3 estimates)",
    displayName: "M1 Abrams",
    moveType: "T",
    targetClass: "armoured_vehicle",
    // ⚠ WAS 9.4 AND `very_heavy`, AND BOTH WERE INVENTED. The curated index
    // could not emit 9.4 — it saturated at 8.6 — and `very_heavy` appears
    // nowhere in 579 rows. Those two figures existed to make this probe
    // discriminate, and they did it by inflating the modern tank.
    //
    // The probe still discriminates, off the Type 59's band instead: see the
    // note there. If it ever stops, the lever is that band or the fire
    // columns, NOT a number typed in here to force the answer.
    csIndex: 8.6,
    protectionBand: "heavy",
    armourMm: 600,
    armour: { frontKeMm: 600, frontCeMm: 900, sideKeMm: 100, sideCeMm: 300, rearKeMm: 40, rearCeMm: 60, roofKeMm: 25, roofCeMm: 25 },
    capabilities: [
      { kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 3000, penetrationMm: 848 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1700 },
    ],
  },
  /**
   * THE LAST DECLARED VEHICLE, RETIRED — and it turned out not to need a
   * judgement call at all.
   *
   * This was `tankmodels/cn_type_59`, whose figures were ours. Giving it a
   * penetration figure was the one place this pass looked like it would have
   * to invent one, because the curated tables model no Type 59.
   *
   * ⚠ THEY MODEL A T-55, AND THE TYPE 59 IS A CHINESE-BUILT T-54A. Same
   * 100 mm D-10T, same hull. The check that settles it is that the curated
   * aspect table and the hand-declared one are IDENTICAL — 200/80/45/15,
   * every figure — which says whoever declared the Type 59 was reading a
   * T-54/55 reference already. So this is not a substitution, it is the
   * same vehicle with its provenance restored.
   *
   * It also drops a bad Combat Strength. The declared 8.0 came from the L6
   * index, which is the SAME SATURATION the curated profile has just been
   * fixed for, seen from the bottom: L6 could not spread vehicles out, so a
   * 1950s tank scored 8.0 against a Challenger's 8.6. The curated index
   * says 6.1, and the `medium` band it earns is the one this file corrected
   * by hand in the previous pass. Both now come from the source.
   */
  "var_20_default": {
    assetId: "var_20_default",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_20_default)",
    displayName: "T-55",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 6.1,
    protectionBand: "medium",
    armourMm: 200,
    armour: { frontKeMm: 200, frontCeMm: 200, sideKeMm: 80, sideCeMm: 80, rearKeMm: 45, rearCeMm: 45, roofKeMm: 15, roofCeMm: 15 },
    capabilities: [
      { kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 3000, penetrationMm: 390 },
      { kind: "apers", maxRangeM: 2000, shortRangeM: 1700 },
    ],
  },
  /**
   * A PROXY RETIRED. This was `tankmodels/it_t_72m1`, an Italian export
   * T-72M1 standing in for the T-72B3 the simulator did not model, carrying
   * `proxyFor: "T-72B3 (absent from the source)"`.
   *
   * The curated tables model one. So the stand-in goes and the real vehicle
   * takes its place, which is the entire point of the curation effort — the
   * proxy note was a promissory note, and this redeems it.
   *
   * Red's tanks get harder as a result: 400 mm of hull front becomes 600,
   * with Kontakt-5 rather than a bare export hull. Combat Strength does not
   * move (8.5 and 8.6 both round to a troop CS of 10), so this shows up as
   * armour that defeats more rounds, not as a bigger number.
   */
  "var_22_1": {
    assetId: "var_22_1",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_22_1)",
    displayName: "T-72B3 obr.2016",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 8.6,
    protectionBand: "heavy",
    armourMm: 600,
    armour: { frontKeMm: 600, frontCeMm: 850, sideKeMm: 80, sideCeMm: 300, rearKeMm: 40, rearCeMm: 60, roofKeMm: 20, roofCeMm: 20 },
    eraFitted: true,
    capabilities: [
      { kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 3000, penetrationMm: 750 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1700 },
    ],
  },
  /**
   * THE SECOND PROXY RETIRED. This was `tankmodels/sw_t_80u`, a Swedish
   * trials T-80U — a vehicle bought for evaluation, not fielded by anyone as
   * a fighting tank — standing in for a Russian one.
   *
   * The curated tables carry the T-80BVM, which is the T-80 that Russia
   * actually fields, with Relikt rather than Kontakt-1.
   */
  "var_23_1": {
    assetId: "var_23_1",
    sourcedFrom: "[SIM] L7 bgws_platform_profile (var_23_1)",
    displayName: "T-80BVM",
    moveType: "T",
    targetClass: "armoured_vehicle",
    csIndex: 8.6,
    protectionBand: "heavy",
    armourMm: 650,
    armour: { frontKeMm: 650, frontCeMm: 900, sideKeMm: 100, sideCeMm: 350, rearKeMm: 40, rearCeMm: 60, roofKeMm: 20, roofCeMm: 20 },
    eraFitted: true,
    capabilities: [
      { kind: "atk", munition: "ke", maxRangeM: 3000, shortRangeM: 3000, penetrationMm: 750 },
      { kind: "apers", maxRangeM: 3000, shortRangeM: 1700 },
    ],
  },

  // ── Dismounted sections, from the curated L7 section profile ─────────────
  //
  // ⚠ THESE ARE SOURCED, NOT DECLARED, WHICH IS NEW. Every infantry figure in
  // this file used to be ours, because the simulator catalogue names 76 foot
  // platforms and models none of them — one has a weapon, none has a range.
  // The curated tables carry 20 real sections with published tables of
  // organisation, so these ids, strengths, ranges and penetrations come from
  // [SIM] L7 bgws_section_profile rather than from judgement.
  //
  // Combat Strength is still ours: it is derived in myproject/bgws_l7.py, on
  // the same 1-9.4 band as the vehicles, from what the section carries and
  // how many people carry it.
  //
  // Penetration is the round the section actually carries, not the best its
  // launcher can fire — a motor-rifle squad has four PG-7VL at 500 mm, not
  // the PG-7VR tandem at 750.
  "sec_uk_armd_inf": {
    assetId: "sec_uk_armd_inf",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "UK Armoured Infantry Section",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.5,
    protectionBand: "light",
    capabilities: [
      { kind: "atm", munition: "ce", topAttack: true, maxRangeM: 800, shortRangeM: 400, penetrationMm: 500 },
      { kind: "apers", maxRangeM: 1800, shortRangeM: 900 },
    ],
  },
  "sec_uk_javelin": {
    assetId: "sec_uk_javelin",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "UK Javelin Detachment",
    moveType: "F",
    targetClass: "foot",
    csIndex: 4.8,
    protectionBand: "light",
    capabilities: [
      { kind: "atm", munition: "ceTandem", topAttack: true, maxRangeM: 2500, shortRangeM: 1250, penetrationMm: 750 },
      { kind: "apers", maxRangeM: 400, shortRangeM: 200 },
    ],
  },
  "sec_uk_gpmg_sf": {
    assetId: "sec_uk_gpmg_sf",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "UK GPMG Sustained-Fire Team",
    moveType: "F",
    targetClass: "foot",
    csIndex: 2,
    protectionBand: "light",
    capabilities: [{ kind: "apers", maxRangeM: 1800, shortRangeM: 900 }],
  },
  "sec_uk_mortar": {
    assetId: "sec_uk_mortar",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "UK 81mm Mortar Section",
    moveType: "F",
    targetClass: "foot",
    csIndex: 2.5,
    protectionBand: "light",
    capabilities: [
      { kind: "idf", maxRangeM: 5650, shortRangeM: 2825 },
      { kind: "apers", maxRangeM: 400, shortRangeM: 200 },
    ],
  },
  "sec_ru_mr_bmp": {
    assetId: "sec_ru_mr_bmp",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "RU Motor-Rifle Squad (BMP)",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.5,
    protectionBand: "light",
    capabilities: [
      { kind: "atk", munition: "ce", maxRangeM: 500, shortRangeM: 250, penetrationMm: 500 },
      { kind: "apers", maxRangeM: 1500, shortRangeM: 750 },
    ],
  },
  "sec_ru_mr_btr": {
    assetId: "sec_ru_mr_btr",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "RU Motor-Rifle Squad (BTR)",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.5,
    protectionBand: "light",
    capabilities: [
      { kind: "atk", munition: "ce", maxRangeM: 500, shortRangeM: 250, penetrationMm: 500 },
      { kind: "apers", maxRangeM: 1500, shortRangeM: 750 },
    ],
  },
  "sec_ru_at": {
    assetId: "sec_ru_at",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "RU Anti-Tank Section (Kornet)",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.6,
    protectionBand: "light",
    capabilities: [
      { kind: "atm", munition: "ceTandem", maxRangeM: 5500, shortRangeM: 2750, penetrationMm: 1200 },
      { kind: "apers", maxRangeM: 400, shortRangeM: 200 },
    ],
  },
  "sec_ru_mortar": {
    assetId: "sec_ru_mortar",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "RU 82mm Mortar Section",
    moveType: "F",
    targetClass: "foot",
    csIndex: 2.5,
    protectionBand: "light",
    capabilities: [
      { kind: "idf", maxRangeM: 4000, shortRangeM: 2000 },
      { kind: "apers", maxRangeM: 400, shortRangeM: 200 },
    ],
  },
  "sec_us_mech_sqd": {
    assetId: "sec_us_mech_sqd",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "US Mechanised Infantry Squad",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.5,
    protectionBand: "light",
    capabilities: [
      { kind: "atk", munition: "ce", maxRangeM: 300, shortRangeM: 150, penetrationMm: 420 },
      { kind: "apers", maxRangeM: 800, shortRangeM: 400 },
    ],
  },
  "sec_us_stryker_sqd": {
    assetId: "sec_us_stryker_sqd",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "US Stryker Rifle Squad",
    moveType: "F",
    targetClass: "foot",
    csIndex: 6.3,
    protectionBand: "light",
    capabilities: [
      { kind: "atm", munition: "ceTandem", topAttack: true, maxRangeM: 2500, shortRangeM: 1250, penetrationMm: 750 },
      { kind: "atk", munition: "ce", maxRangeM: 300, shortRangeM: 150, penetrationMm: 420 },
      { kind: "apers", maxRangeM: 800, shortRangeM: 400 },
    ],
  },
  "sec_generic_light": {
    assetId: "sec_generic_light",
    sourcedFrom: "[SIM] L7 bgws_section_profile (curated equipment tables, published TO&E)",
    displayName: "Territorial / Light Infantry Section",
    moveType: "F",
    targetClass: "foot",
    csIndex: 5.5,
    protectionBand: "light",
    capabilities: [
      { kind: "atk", munition: "ce", maxRangeM: 500, shortRangeM: 250, penetrationMm: 500 },
      { kind: "apers", maxRangeM: 1500, shortRangeM: 750 },
    ],
  },
};

/**
 * Every platform the game can field: the catalogue, plus what we declared.
 *
 * ⚠ THE OVERLAY WINS, AND IT HAS TO.
 *
 * `CATALOGUE_PLATFORMS` is generated from the L6 profiles and is overwritten
 * wholesale every time anybody regenerates it. `PINNED_PLATFORMS` is
 * hand-written for the things the source does not model at all — infantry,
 * whose 76 catalogue rows are empty shells, and the mortar, which the
 * catalogue does not contain in any form.
 *
 * Spreading the declared platforms LAST means a regeneration can never
 * silently delete the only non-armour in the game. If a future catalogue ever
 * does carry a real rifle section, the overlay entry should be deleted
 * deliberately rather than shadowed by accident — which is what the
 * `valuesDeclared` note on each one is for.
 */
export const PLATFORM_SNAPSHOT: Record<string, PlatformSnapshot> = {
  ...CATALOGUE_PLATFORMS,
  ...PINNED_PLATFORMS,
};

export interface ForceElementSpec {
  id: string;
  label: string;
  side: Side;
  /** Key into PLATFORM_SNAPSHOT. */
  platform: string;
  /** How many platforms in the sub-unit. A troop is 4; a platoon 3. */
  platformCount: number;
  /** DECLARED. See the note at the top of this file. */
  troopQuality: TroopQualityName;
  /** Metres east and north of the scenario origin. */
  offsetM: { east: number; north: number };
  /**
   * Starts hidden, and is harder to sight until it reveals itself.
   *
   * Declared here rather than waiting on terrain. Concealment is properly a
   * terrain property, but making it a force-list option means the
   * `concealment` module and the `targetConcealed` modifier are MEASURABLE
   * now — and an unmeasurable mechanic is the one thing this project will not
   * carry. Terrain will later grant it as well; it will not replace this.
   */
  concealed?: boolean;
  /**
   * A headquarters, and how many subordinates it can activate.
   *
   * Presence of an HQ is what makes `commandActivations` and the `hqPresent`
   * morale modifier reachable at all.
   */
  commandRating?: number;
  /** A decoy. Absorbs attention until something fires at it. */
  isDummy?: boolean;
}

export interface ForceList {
  id: string;
  name: string;
  /** Where the two sides deploy from, as a metre offset from the origin. */
  deployment: Record<Side, { east: number; north: number }>;
  elements: ForceElementSpec[];
  /** Anything a reader should know before trusting a result from this list. */
  caveats: string[];
}

/**
 * A meeting engagement: a British armoured squadron against a Russian tank
 * company, both mounted, on open ground.
 *
 * Deliberately symmetric in equipment quality (CS index 8.5-8.6 on both
 * sides) so that the FIRST harness run measures the rules rather than a
 * mismatch. Troop Quality is the one asymmetry, because that is the lever
 * the rules are supposed to care about and it was previously constant.
 */
export const MEETING_ENGAGEMENT_V1: ForceList = {
  id: "meeting-engagement-v1",
  name: "Meeting engagement — armour, open ground",
  deployment: {
    blue: { east: 0, north: 0 },
    red: { east: 0, north: 1600 },
  },
  elements: [
    {
      id: "blue-1",
      label: "1 Troop",
      side: "blue",
      platform: "var_11_default",
      platformCount: 4,
      troopQuality: "veteran",
      offsetM: { east: -400, north: 0 },
    },
    {
      id: "blue-2",
      label: "2 Troop",
      side: "blue",
      platform: "var_11_default",
      platformCount: 4,
      troopQuality: "regular",
      offsetM: { east: 400, north: 0 },
    },
    {
      id: "blue-3",
      label: "3 Troop",
      side: "blue",
      platform: "var_11_default",
      platformCount: 4,
      troopQuality: "regular",
      offsetM: { east: 0, north: -200 },
    },
    {
      id: "blue-4",
      label: "4 Platoon",
      side: "blue",
      platform: "var_27_default",
      platformCount: 4,
      troopQuality: "regular",
      offsetM: { east: 0, north: -300 },
    },
    /**
     * A SECOND WARRIOR PLATOON, ADDED TO KEEP THE SCENARIO MEASURABLE.
     *
     * ⚠ THIS IS SCENARIO DESIGN, NOT DATA, AND IT IS HERE BECAUSE OF A DATA
     * CORRECTION. Blue balanced against red only while the Warrior was rated
     * 8.5 and banded `medium` — an IFV scoring within two points of a
     * Challenger troop on the strength of a TOW belonging to an export model
     * Britain never bought. Correcting it to the sourced 4.3/`light` took
     * blue from 56 to 53 against red's 59, which is 11% apart, and the list
     * is the instrument the module sweep runs on: at that gap the sweep
     * measures the imbalance rather than the module.
     *
     * The gap is structural rather than about vehicles. Both sides field 14
     * tanks, but `countWeight` awards a point per platform per ELEMENT and
     * rounds per element, so red's six three-vehicle sub-units bank more than
     * blue's five larger ones. Evening the element count is the fix that
     * addresses the cause; adding strength to an existing element would only
     * hide it.
     *
     * It is also the more realistic order of battle. Three tank troops with a
     * single Warrior platoon is infantry-light for a British battlegroup; two
     * platoons is a conventional square combat team.
     */
    {
      id: "blue-5",
      label: "5 Platoon",
      side: "blue",
      platform: "var_27_default",
      platformCount: 4,
      troopQuality: "regular",
      offsetM: { east: -200, north: -300 },
    },
    {
      id: "red-1",
      label: "1st Tank Platoon",
      side: "red",
      platform: "var_22_1",
      platformCount: 3,
      troopQuality: "conscript",
      offsetM: { east: -400, north: 0 },
    },
    {
      id: "red-2",
      label: "2nd Tank Platoon",
      side: "red",
      platform: "var_22_1",
      platformCount: 3,
      troopQuality: "conscript",
      offsetM: { east: 400, north: 0 },
    },
    {
      id: "red-3",
      label: "3rd Tank Platoon",
      side: "red",
      platform: "var_22_1",
      platformCount: 3,
      troopQuality: "conscript",
      offsetM: { east: 0, north: 300 },
    },
    {
      id: "red-4",
      label: "Guards Tank Platoon",
      side: "red",
      platform: "var_23_1",
      platformCount: 3,
      troopQuality: "regular",
      offsetM: { east: 800, north: 300 },
    },
    // The three below exist to make mechanics REACHABLE, not for realism.
    // Each one is the minimum force-list change that lets a declared rule
    // fire at all, and without them the harness cannot tell "this mechanic
    // does nothing" from "no scenario has ever exercised it".
    {
      id: "blue-hq",
      label: "Squadron HQ",
      side: "blue",
      platform: "var_11_default",
      platformCount: 2,
      troopQuality: "veteran",
      // Makes commandActivations bind and hqPresent reachable.
      commandRating: 3,
      offsetM: { east: -800, north: -400 },
    },
    {
      id: "red-hq",
      label: "Company HQ",
      side: "red",
      platform: "var_23_1",
      platformCount: 1,
      troopQuality: "regular",
      commandRating: 3,
      offsetM: { east: -800, north: 400 },
    },
    {
      id: "red-recce",
      label: "Recce Screen",
      side: "red",
      platform: "var_22_1",
      platformCount: 1,
      troopQuality: "veteran",
      // Makes concealment and targetConcealed reachable before terrain does.
      concealed: true,
      offsetM: { east: 1200, north: -400 },
    },
    // Two decoys, one a side. Without these the first full sweep reported
    // `dummies` at 0% and labelled it ceremony — when in fact no force list
    // had ever contained a dummy. "Does nothing" and "was never tested" need
    // opposite fixes and the sweep cannot tell them apart, so the scenario
    // has to be able to exercise the rule before its verdict means anything.
    {
      id: "blue-decoy",
      label: "Decoy Troop",
      side: "blue",
      platform: "var_11_default",
      platformCount: 4,
      troopQuality: "regular",
      isDummy: true,
      offsetM: { east: 800, north: -200 },
    },
    {
      id: "red-decoy",
      label: "Decoy Platoon",
      side: "red",
      platform: "var_22_1",
      platformCount: 3,
      troopQuality: "conscript",
      isDummy: true,
      offsetM: { east: -1200, north: 200 },
    },
  ],
  caveats: [
    "Both sides now field the vehicles they actually operate: the T-72M1 " +
      "export proxy and the Swedish trials T-80U are gone, replaced by the " +
      "curated T-72B3 obr.2016 and T-80BVM.",
    "Troop Quality on every element is a declared assumption, not data. " +
      "Nothing in the source describes crews.",
    "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
    // The one a reader must not miss, because it caps what any batch run on
    // this list is allowed to conclude.
    //
    // ⚠ NARROWED, NOT DELETED. The old version of this caveat said Combat
    // Strength barely discriminated equipment at all, because the index ran
    // 8.0 to 9.4 across everything from a Type 59 to an M1A2 and rated a
    // Warrior at 8.5 against a Challenger's 8.6. Both halves have been fixed
    // — the Warrior is 4.3 and `light`, and the curated index no longer
    // saturates at the top — so repeating the old wording would understate
    // the evidence. What remains true is narrower and still limiting.
    "COMBAT STRENGTH DISCRIMINATES COARSELY AT THE TOP. The curated index " +
      "separates an IFV from a tank properly, but ten modern MBTs still " +
      "share a narrow band: every tank on this list is csIndex 8.6, so " +
      "between tanks Combat Strength is close to proportional to platform " +
      "count alone. Differences between them show up through ARMOUR and " +
      "PENETRATION instead, which is where they belong. A result from this " +
      "list is evidence about the RULES and about troop quality; it is not a " +
      "ranking of one modern tank against another.",
    // ⚠ THE HOLE IS CLOSED AND IT REVEALED A HARDER ONE. Named here because
    // it caps what a run with the penetration module ON is worth.
    "Every gun now states a sourced penetration figure, so armour finally " +
      "bites — and with the module ON these tanks mostly cannot kill each " +
      "other. A Challenger 2 needs 700 mm and the best round here makes 750 " +
      "at a kilometre, falling to 638 at two; beyond about 1.1 km nobody " +
      "defeats anybody frontally. The sweep measures this as 25 to 31 extra " +
      "turns against a 40-turn cap. That is the armour being modelled " +
      "honestly, not a data error, but it means a penetration-on run is " +
      "evidence about a STALEMATE, and the levers are the gate, the ranges " +
      "or the board — not the figures.",
  ],
};

/**
 * The same engagement with four evenly matched sub-units a side.
 *
 * Exists because MEETING_ENGAGEMENT_V1 gives blue 102 Combat Strength to
 * red's 104 only after red's fourth platoon was added — British troops are
 * four platforms and Soviet platoons three, which is doctrinally right and
 * experimentally awful. Any module comparison run on an unbalanced list
 * measures the imbalance first.
 *
 * This list is the control: identical platforms, identical counts, identical
 * Troop Quality. If a module moves the win rate HERE, it moved it.
 */
export const SYMMETRIC_CONTROL_V1: ForceList = {
  id: "symmetric-control-v1",
  name: "Symmetric control — identical forces, open ground",
  deployment: {
    blue: { east: 0, north: 0 },
    red: { east: 0, north: 1600 },
  },
  elements: [
    ...(["blue", "red"] as const).flatMap((side, sideIndex) =>
      [0, 1, 2].map((index) => ({
        id: `${side}-${index + 1}`,
        label: `${side === "blue" ? "Blue" : "Red"} ${index + 1} Troop`,
        side,
        platform: "var_11_default",
        platformCount: 4,
        troopQuality: "regular" as TroopQualityName,
        offsetM: { east: (index - 1) * 500, north: sideIndex * 0 },
      })),
    ),
  ],
  caveats: [
    "Both sides field Challenger 2 troops. This is a control for measuring " +
      "rules, not a scenario anybody would fight.",
    "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
  ],
};

/**
 * COMBINED ARMS: tanks and dismounted infantry, both sides.
 *
 * Exists because an all-armour game cannot reach three rules, in the same way
 * an all-point-blank game could not reach `firerMoved` and advance-to-contact
 * was written to fix it:
 *
 *   9.2.1  "Apers Capabilities may only be used against Foot FEs and
 *           soft-skinned Wheeled vehicles. Atk Capabilities may be used
 *           against any vehicle FE, but not Foot FEs." Meaningless when
 *           everything on the board is a tank.
 *   9.3.4  `defenderIsVehicleOnly` — needs somebody who is not a vehicle for
 *           the absence of them to mean anything.
 *   2.1.8  Target class at all.
 *
 * Symmetric, so it is also usable as a control: both sides field the same
 * two tank troops and two infantry sections. The infantry sit forward of
 * their armour, which is where the interesting decisions are — tanks cannot
 * usefully shoot them with a main gun, and they can kill a tank at 400 m.
 *
 * ⚠ THE INFANTRY FIGURES ARE OURS, not the source's. See the
 * `valuesDeclared` note on `infantry/rifle_infantry`.
 */
export const COMBINED_ARMS_V1: ForceList = {
  id: "combined-arms-v1",
  name: "Combined arms — tanks and dismounted infantry, symmetric",
  deployment: {
    blue: { east: 0, north: 0 },
    red: { east: 0, north: 1600 },
  },
  elements: [
    ...(["blue", "red"] as const).flatMap((side) => {
      const name = side === "blue" ? "Blue" : "Red";
      // Infantry forward, armour behind — the deployment that makes the
      // target-class rules bite rather than a tidy line of counters.
      const towardsEnemy = side === "blue" ? 1 : -1;
      return [
        ...[0, 1].map((index) => ({
          id: `${side}-tank-${index + 1}`,
          label: `${name} ${index + 1} Troop`,
          side,
          platform: "var_11_default",
          platformCount: 4,
          troopQuality: "regular" as TroopQualityName,
          offsetM: { east: (index === 0 ? -1 : 1) * 400, north: 0 },
        })),
        // One tube per side, well back — it does not need to see what it
        // shoots at (9.2.2.1), which is the point of having it.
        {
          id: `${side}-mortar`,
          label: `${name} Mortar Section`,
          side,
          platform: "declared/mortar_section",
          platformCount: 2,
          troopQuality: "regular" as TroopQualityName,
          offsetM: { east: 0, north: -towardsEnemy * 600 },
        },
        ...[0, 1].map((index) => ({
          id: `${side}-inf-${index + 1}`,
          label: `${name} ${index + 1} Section`,
          side,
          platform: "infantry/rifle_infantry",
          platformCount: 3,
          troopQuality: "regular" as TroopQualityName,
          offsetM: { east: (index === 0 ? -1 : 1) * 200, north: towardsEnemy * 400 },
        })),
      ];
    }),
  ],
  caveats: [
    "The infantry figures are DECLARED, not sourced: the L6 profiles name 76 " +
      "foot platforms and model none of them. Ranges, combat strength and " +
      "capabilities are ours.",
    "No mounting or dismounting (9.01) — the sections start on their feet and " +
      "stay there. There are no carriers on the board.",
    "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
  ],
};

/**
 * CATALOGUE ARMOUR: built entirely from the generated catalogue.
 *
 * Every other force list names a platform somebody pinned by hand. This one
 * names nothing hand-written — both sides come straight out of the L6
 * profiles through the generator, which is the proof that the catalogue path
 * works end to end rather than a claim that it does.
 *
 * ⚠ DELIBERATELY NOT IN `FORCE_LISTS`, AND THE REASON IS THE POINT.
 *
 * Play it with `penetration` ON and it deadlocks: 40 games out of 40 run to
 * the turn limit, blue "wins" all 40 on the tie-break because a Merkava troop
 * starts with 30 combat strength against a Leopard troop's 20. Nobody is
 * killed, because nobody CAN be.
 *
 * The source's penetration column is templated at 480 mm across almost all of
 * the 819 playable rows, and every frontal armour figure in the same source
 * is above it — 500, 530, 585, 605, 650, 800, 900. So with penetration on,
 * every sourced gun bounces off every sourced tank. The only thing in the
 * catalogue that can defeat anything is the Pvrbv 551's 800 mm missile, and
 * that kills everything up to 650 mm without being killed back.
 *
 * There is therefore NO balanced matchup available from sourced figures: the
 * data gives mutual immunity or total one-sidedness and nothing between. That
 * is a finding about the pipeline's penetration figures, not about the rule.
 *
 * Including it in the sweep would have put `penetration` at 100% of decisions
 * and 54% of outcomes — the most decisive rule in the game — on the strength
 * of a stalemate. It stays out of the measured set and stays available to
 * play, which is the honest arrangement for a scenario that exists to
 * demonstrate a data problem.
 */
export const CATALOGUE_ARMOUR_V1: ForceList = {
  id: "catalogue-armour-v1",
  name: "Catalogue armour — Merkava against Leopard, straight from the profiles",
  deployment: {
    blue: { east: 0, north: 0 },
    red: { east: 0, north: 2000 },
  },
  elements: [
    ...[0, 1].map((index) => ({
      id: `blue-${index + 1}`,
      label: `Blue ${index + 1} Troop`,
      side: "blue" as const,
      platform: "tankmodels/il_merkava_mk_3b",
      platformCount: 3,
      troopQuality: "regular" as TroopQualityName,
      offsetM: { east: (index === 0 ? -1 : 1) * 300, north: 0 },
    })),
    ...[0, 1].map((index) => ({
      id: `red-${index + 1}`,
      label: `Red ${index + 1} Troop`,
      side: "red" as const,
      platform: "tankmodels/it_leopard_2a4",
      platformCount: 3,
      troopQuality: "regular" as TroopQualityName,
      offsetM: { east: (index === 0 ? -1 : 1) * 300, north: 0 },
    })),
  ],
  caveats: [
    "Both platforms are sourced, but the source's figures are templated: 480 mm " +
      "of penetration and 2,000-3,000 m of range recur across most of the 819 " +
      "playable rows. The armour differs; almost nothing else does.",
    "Display names are the catalogue's own, capitalisation and all.",
    "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
  ],
};

/**
 * An advance to contact: forces start OUT OF RANGE and have to close.
 *
 * Exists because every other force list starts inside weapon range, and that
 * turned out to hide a whole third of the rulebook. A Challenger's gun has a
 * short range of 3,000 m and the other lists deploy at 1,600 m, so no element
 * was ever beyond short range, so no commander ever had a reason to move, so
 * `firerMoved` and `targetMoved` were unreachable — not because the rules
 * were wrong but because the scenarios were all point-blank.
 *
 * 5,000 m apart. Nothing can shoot until somebody drives.
 */
export const ADVANCE_TO_CONTACT_V1: ForceList = {
  id: "advance-to-contact-v1",
  name: "Advance to contact — symmetric, starts out of range",
  deployment: {
    blue: { east: 0, north: 0 },
    red: { east: 0, north: 5000 },
  },
  elements: [
    ...(["blue", "red"] as const).flatMap((side) =>
      [0, 1, 2].map((index) => ({
        id: `${side}-${index + 1}`,
        label: `${side === "blue" ? "Blue" : "Red"} ${index + 1} Troop`,
        side,
        platform: "var_11_default",
        platformCount: 4,
        troopQuality: "regular" as TroopQualityName,
        offsetM: { east: (index - 1) * 600, north: 0 },
      })),
    ),
    ...(["blue", "red"] as const).map((side) => ({
      id: `${side}-hq`,
      label: `${side === "blue" ? "Blue" : "Red"} HQ`,
      side,
      platform: "var_11_default",
      platformCount: 2,
      troopQuality: "veteran" as TroopQualityName,
      commandRating: 3,
      offsetM: { east: 1200, north: 0 },
    })),
  ],
  caveats: [
    "Symmetric by construction. Both sides field Challenger 2 troops with an " +
      "HQ, so any asymmetry in the result is the rules or the dice.",
    "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
  ],
};

/**
 * A probe: two lists identical but for ONE variable.
 *
 * These are not scenarios. They are instruments. Each isolates a single thing
 * the game is supposed to care about and asks whether it changes the win
 * rate, so that "does troop quality matter?" has an answer rather than an
 * opinion. If a probe comes back 50/50, the game does not model that thing,
 * whatever the rulebook says.
 */
function probePair(
  id: string,
  name: string,
  blue: Pick<ForceElementSpec, "platform" | "troopQuality">,
  red: Pick<ForceElementSpec, "platform" | "troopQuality">,
  caveats: string[],
): ForceList {
  return {
    id,
    name,
    deployment: { blue: { east: 0, north: 0 }, red: { east: 0, north: 1600 } },
    elements: (["blue", "red"] as const).flatMap((side) =>
      [0, 1, 2].map((index) => ({
        id: `${side}-${index + 1}`,
        label: `${side === "blue" ? "Blue" : "Red"} ${index + 1} Troop`,
        side,
        platformCount: 4,
        offsetM: { east: (index - 1) * 500, north: 0 },
        ...(side === "blue" ? blue : red),
      })),
    ),
    caveats,
  };
}

/**
 * CRITERION 4. Identical equipment, identical counts, different crews.
 *
 * Blue veteran, red conscript. If blue does not win appreciably more often,
 * Troop Quality is decoration — which it WAS until the morale check was
 * wired into the turn loop, because nothing else in the game read it.
 */
export const QUALITY_PROBE_V1: ForceList = probePair(
  "quality-probe-v1",
  "Probe — troop quality only",
  { platform: "var_11_default", troopQuality: "veteran" },
  { platform: "var_11_default", troopQuality: "conscript" },
  [
    "An instrument, not a scenario. Everything is identical except Troop " +
      "Quality, so any difference in win rate is Troop Quality.",
  ],
);

/**
 * CRITERION 6. Identical crews and counts, forty years of tank design apart.
 *
 * M1A2 SEP3 against Type 59. The upstream index rates them 9.4 and 8.0 — a
 * 17% gap between a 1950s tank and a modern one — so without the protection
 * bonus this probe would come back close to even and the game would be
 * telling you equipment does not matter.
 */
export const GENERATION_PROBE_V1: ForceList = probePair(
  "generation-probe-v1",
  "Probe — equipment generation only",
  { platform: "var_1_default", troopQuality: "regular" },
  { platform: "var_20_default", troopQuality: "regular" },
  [
    "An instrument, not a scenario. Identical crews and counts; only the " +
      "equipment differs.",
    "A near-even result here means Combat Strength is not discriminating " +
      "equipment, NOT that a Type 59 is a match for an M1A2.",
  ],
);

/**
 * The MEASURED set: what the module sweep plays.
 *
 * `catalogue-armour-v1` is deliberately absent — see its own note. A scenario
 * that deadlocks under a module would hand that module a headline number it
 * has not earned.
 */
export const FORCE_LISTS: Record<string, ForceList> = {
  [ADVANCE_TO_CONTACT_V1.id]: ADVANCE_TO_CONTACT_V1,
  [QUALITY_PROBE_V1.id]: QUALITY_PROBE_V1,
  [GENERATION_PROBE_V1.id]: GENERATION_PROBE_V1,
  [MEETING_ENGAGEMENT_V1.id]: MEETING_ENGAGEMENT_V1,
  [SYMMETRIC_CONTROL_V1.id]: SYMMETRIC_CONTROL_V1,
  [COMBINED_ARMS_V1.id]: COMBINED_ARMS_V1,
};

/**
 * Build a symmetric scenario around any platform in the snapshot.
 *
 * This is what lets the Play screen field ANY of the 819 playable catalogue
 * entries without somebody hand-writing 819 force lists. Both sides get the
 * same thing, so whatever happens is attributable to the rules and the dice
 * rather than to the matchup — the same reasoning that makes
 * symmetric-control the fairest place to test a commander.
 *
 * ⚠ NOT REGISTERED IN `FORCE_LISTS`, and it must not be. These are generated
 * on demand for play; a sweep over 819 of them would take a day and would
 * mostly re-measure the 72 distinct statlines underneath them.
 */
export function symmetricListFrom(
  platformId: string,
  options: { troopsPerSide?: number; platformCount?: number; separationM?: number } = {},
): ForceList {
  const platform = PLATFORM_SNAPSHOT[platformId];
  if (!platform) throw new Error(`No such platform: ${platformId}`);

  const troops = options.troopsPerSide ?? 2;
  const count = options.platformCount ?? 3;

  return {
    id: `mirror:${platformId}`,
    name: `${platform.displayName} — mirror match`,
    deployment: {
      blue: { east: 0, north: 0 },
      red: { east: 0, north: options.separationM ?? 1600 },
    },
    elements: (["blue", "red"] as const).flatMap((side) =>
      Array.from({ length: troops }, (_, index) => ({
        id: `${side}-${index + 1}`,
        label: `${side === "blue" ? "Blue" : "Red"} ${index + 1} Troop`,
        side,
        platform: platformId,
        platformCount: count,
        troopQuality: "regular" as TroopQualityName,
        offsetM: { east: (index - (troops - 1) / 2) * 500, north: 0 },
      })),
    ),
    caveats: [
      `Generated mirror match. Both sides field ${platform.displayName}, so the ` +
        "result is about the rules and the dice, not the matchup.",
      ...(platform.valuesDeclared
        ? [`⚠ This platform's figures are ours, not the source's: ${platform.valuesDeclared}.`]
        : []),
      "Ground is flat. Until the DEM is wired in, terrain contributes nothing.",
    ],
  };
}

/** Platforms the Play screen may offer, sorted for a human to scan. */
export function playablePlatforms(): PlatformSnapshot[] {
  return Object.values(PLATFORM_SNAPSHOT).sort((a, b) =>
    a.displayName.localeCompare(b.displayName),
  );
}

/**
 * Every force list that exists, measured or not.
 *
 * The guard tests walk this rather than `FORCE_LISTS`, so a scenario kept out
 * of the sweep still cannot name a platform that does not exist or cannot
 * fight. Being unmeasured is not the same as being unchecked.
 */
export const ALL_FORCE_LISTS: Record<string, ForceList> = {
  ...FORCE_LISTS,
  [CATALOGUE_ARMOUR_V1.id]: CATALOGUE_ARMOUR_V1,
};
