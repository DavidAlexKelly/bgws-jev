// ── bgws/rules/ruleset.ts ──────────────────────────────────────────────────
// Every number a die roll touches, in one object.
//
// THE RULE ABOUT NUMBERS
// ----------------------
// A number may be ours. It may not be hidden. Anything that decides an outcome
// lives here, is named, is attributable to a ruleset id, and can be changed
// without editing a resolver — because the point of this platform is to find
// out which mechanics earn their place, and a mechanic buried in code cannot
// be argued with, measured or switched off.
//
// `house-v1` below is OURS. Its structure follows BGWS, because BGWS is a
// well-designed reference and its vocabulary is one a military audience
// already speaks. Its numbers are not BGWS's and do not claim to be. When the
// real Player Aids are transcribed they become a SECOND ruleset alongside this
// one, and the interesting thing will be running them against each other.

import type { AllowanceTable } from "../lib/movement";
import { allowanceFromSpeeds, VEHICLE_CONTACT_FRACTION } from "../lib/terrainAdapter";

/** What a fire resolution can produce. */
export type FireResult = "miss" | "suppress" | "oneHit" | "twoHits" | "threeHits";

/**
 * One column of the fire table.
 *
 * Read as: a modified 2D6 of at least `suppressAt` suppresses, at least
 * `oneHitAt` scores a hit, and so on. Expressing a column as thresholds rather
 * than as eleven rows makes it legible — a reader can see the shape of the
 * ladder at a glance, which matters when the numbers are up for debate.
 */
export interface FireColumn {
  label: string;
  /** Lowest combined Combat Strength that uses this column. */
  minCombatStrength: number;
  suppressAt: number;
  oneHitAt: number;
  twoHitsAt: number;
  threeHitsAt: number;
}

/**
 * How a sub-unit's Combat Strength is derived from its equipment.
 *
 * THIS IS A RULE, NOT A CALCULATION, and it lives here for that reason.
 *
 * It was briefly implemented in the force builder as `csIndex * platformCount`,
 * which is dimensionally wrong and produced a silent, total failure: a troop
 * of four Challengers came out at Combat Strength 34, which saturates the
 * top fire column so every unit on the board fires identically, and needs
 * about thirty hits to eliminate so no game ever reaches a decision. A full
 * module sweep then reported all ten mechanics as having "no measurable
 * effect — a candidate for deletion". Every one of those readings was an
 * artefact of this mapping.
 *
 * `bgws_cs_index` is already on a 0-9.4 scale that looks like a Combat
 * Strength for a single counter, so platform count must ADD, not multiply.
 *
 * Being in the RuleSet means the mapping is versioned, swappable, and
 * measurable by the harness — which is the only reason anyone would have
 * found the bug above deliberately rather than by accident.
 */
/**
 * How much Combat Strength one hit removes.
 *
 * The single lever for game length, and in the RuleSet so it is measurable
 * rather than a constant somebody tuned once.
 *
 * At one point per hit, a sub-unit at Combat Strength 8 needed eight hits to
 * destroy, and games ran to the 40-turn limit without resolving — which makes
 * every outcome a draw and every outcome-based comparison worthless. BGWS
 * games run nearer 10-15 turns.
 */
export interface LethalityRule {
  /** Combat Strength removed per hit scored. */
  strengthPerHit: number;
  /** Morale steps lost per hit. Kept separate: shaken is not destroyed. */
  moraleStepsPerHit: number;
}

export interface CombatStrengthRule {
  /** Share of the platform's own strength index that carries through. */
  perPlatformWeight: number;
  /** Added per platform in the sub-unit. */
  countWeight: number;
  /**
   * Added by protection band, keyed by the L6 `protection_band` value.
   *
   * THIS IS HERE BECAUSE THE UPSTREAM INDEX CANNOT TELL A TANK FROM AN IFV.
   * `bgws_cs_index` is driven by best penetration and barely weights armour,
   * so it rates a Warrior IFV at 8.5 against a Challenger 2's 8.6 — on the
   * strength of the Warrior's TOW — and a 1950s Type 59 at 8.0. Across all
   * 1,029 armoured vehicles the index sits between 8.0 and 9.4 for almost
   * everything, which made Combat Strength very nearly a function of platform
   * count alone and left equipment quality unable to influence a game.
   *
   * Protection is the axis the index misses, and the source does carry it:
   * very_heavy averages 831 mm of armour, heavy 293, medium 150, light 36.
   *
   * Corrected HERE rather than in the pipeline on purpose. The index is
   * documented upstream as a calibration aid, not a Combat Strength; what a
   * sub-unit is worth in THIS game is the ruleset's business, and putting it
   * here keeps it swappable and measurable instead of frozen in a dataset.
   */
  protectionBonus: Record<string, number>;
  /** Clamped into the fire table's range — see fireColumns. */
  min: number;
  max: number;
}

export interface DrmTable {
  /** Target is in woods, urban or otherwise covered. */
  targetInCover: number;
  /** Target moved in its last activation. */
  targetMoved: number;
  /** Target is already suppressed — easier to hit, harder to break further. */
  targetSuppressed: number;
  /** Firer moved this activation. */
  firerMoved: number;
  /** Firer is suppressed. */
  firerSuppressed: number;
  /** Smoke between firer and target. */
  smoke: number;
  /** Beyond 50% of the weapon's maximum range. */
  longRange: number;
  /** Fire from a flank or rear aspect. */
  flank: number;
}

/**
 * RALLY — the first step of the Command Sub-phase (5.2).
 *
 * ⚠ THE MORALE LADDER WAS A ONE-WAY RATCHET AND THIS IS WHY GAMES STALLED.
 *
 * Fire degrades morale, and a "suppress" result does it WITHOUT costing any
 * Combat Strength. Clean-up's morale check only tested elements that had
 * LOST strength, so an element suppressed but undamaged — the commonest state
 * on the board — could never take a check, could never pass one, and could
 * therefore never recover. It sat there for the rest of the game.
 *
 * It showed up in the LLM commander trial as plans that said "waiting to
 * rally" for eleven consecutive turns. The model was not confused: it was
 * correctly describing a recovery the rules promised and the code did not
 * implement.
 *
 * 5.2 is quoted rather than invented: "each FE with a Morale Status marker
 * takes a Morale Check to attempt to recover one or two levels", "apply an
 * automatic recovery of one level if the FE is Co-located with an
 * Un-Suppressed HQ or A1 Echelon", "an FE passes on a modified 1D6 roll of
 * 4+", and its worked example adds the other automatic case: "if 1 PL INF had
 * been out of LoS of any enemy FE and/or Co-located with an un-Suppressed HQ
 * ... it would have automatically improved Morale one level, and then rolled
 * to see if it improved another."
 *
 * Note what that makes true, and what it is worth as a game: BREAKING CONTACT
 * RECOVERS YOU. Pulling a shaken element back behind a ridge is a real move
 * with a real payoff, and it was unavailable while this was missing.
 *
 * MEASURED, 40 seeds x 4 force lists, rally off against on:
 *
 *   force list              mean turns      hit the 40-turn limit   balance
 *   advance-to-contact-v1   13.18 -> 11.20  2/40 -> 0/40            23/16 -> 20/20
 *   symmetric-control-v1    14.00 ->  8.50  7/40 -> 1/40            16/23 -> 21/18
 *   combined-arms-v1         9.93 ->  8.93  2/40 -> 0/40            15/25 -> 16/24
 *   meeting-engagement-v1   13.38 -> 10.05  3/40 -> 0/40            18/22 -> 17/23
 *
 * ⚠ A RECOVERY RULE MADE GAMES SHORTER, WHICH IS BACKWARDS UNTIL IT ISN'T.
 * Unresolved games fell from 14 in 160 to 1 in 160. Suppressed and broken
 * elements cannot advance or engage, so without a way back up the ladder both
 * sides accumulate paralysed units until nobody can finish anything and the
 * clock runs out. Restoring them does not prolong the fight; it lets the fight
 * REACH A CONCLUSION. It changed a decision in 40 of 40 games on every list,
 * and the winner in 5 to 21 of 40.
 */
export interface RallyRule {
  /**
   * A modified 1D6 at or above this recovers a level. THE RULEBOOK'S OWN 4+,
   * not ours — one of the few resolution numbers the Core Rulebook states
   * rather than leaving to a Player Aid.
   */
  passTarget: number;
  /**
   * DRM for notably good or poor troops. OURS.
   *
   * The rulebook's modifier here is the C2 rating of the commanding HQ, which
   * comes off Player Aid 2 and is not in the box we have. Troop Quality is
   * the quality axis this game does model, so it stands in — compressed to
   * +/-1 because the roll is a single D6 and a +4 on a D6 is not a modifier,
   * it is a guarantee.
   */
  qualityDrm: number;
  /** Troop Quality at or above this earns the bonus; at or below `poorAt`, the penalty. */
  goodAt: number;
  poorAt: number;
}

/**
 * What counts as winning (3.1).
 *
 * The GRADES are the rulebook's — "a Decisive, Substantive or Marginal
 * victory, or no success", and both sides may score something. The THRESHOLDS
 * are ours, because per-scenario victory conditions live in the SSIs and we do
 * not have them: a general rule stands in until a scenario brings its own.
 *
 * ⚠ THIS REPLACED "WHOEVER HAS MORE COMBAT STRENGTH AT THE TURN LIMIT". That
 * made every outcome figure in every report a measure of who did more damage
 * rather than of who achieved anything, and rewarded a side for trading well
 * while ignoring the ground it had been sent to take.
 */
export interface VictoryRule {
  /**
   * How close an element must be to count as holding its objective.
   *
   * OURS. 500 m is twice the Glossary's co-location radius: close enough to
   * be a presence on the objective rather than in the same grid square, and
   * loose enough that a troop deployed around a feature still holds it.
   */
  holdWithinM: number;
  /**
   * Below this share of its starting Combat Strength, a side is no longer a
   * force in being. OURS: a third, which is roughly the point at which a
   * company has lost two of its three platoons.
   */
  combatIneffectiveBelow: number;
  /**
   * How far apart two sides' effectiveness must be before attrition alone is
   * called a marginal win rather than a draw. OURS, and deliberately not
   * small: this is the weakest clause in the judgement and should decide as
   * few games as possible.
   */
  marginalEffectivenessGap: number;
}

export interface MoraleTable {
  /** 2D6 + Troop Quality - penalties must reach this to pass. */
  passTarget: number;
  /**
   * Penalty applied to a sub-unit that has lost ALL of its strength,
   * scaled proportionally for anything less.
   *
   * PROPORTIONAL, NOT PER POINT, and that distinction is the difference
   * between Troop Quality mattering and not.
   *
   * It used to be a flat penalty per point of Combat Strength lost. With
   * strengthPerHit at 3, a single hit cost 3 points of morale — which is the
   * entire spread between a conscript crew (2) and a veteran one (5). So
   * after one hit every sub-unit failed every check regardless of who was in
   * it, and a 400-game probe of veteran against conscript returned 50%.
   *
   * Worse, it was silently coupled to lethality: tuning game length in
   * Phase 2a changed how much morale a hit cost, and so quietly changed how
   * much Troop Quality was worth. A proportional penalty is scale-free — it
   * means the same thing whatever a hit is worth.
   */
  penaltyAtTotalLoss: number;
  /** Penalty for being under fire from more than one direction. */
  multipleDirections: number;
  /** Bonus for being co-located with an unsuppressed HQ. */
  hqPresent: number;
  /**
   * A passed check RECOVERS a step of morale, rather than merely not losing one.
   *
   * Without this, Troop Quality measurably did nothing. A probe of 400 games —
   * identical Challenger 2 troops, veteran against conscript, run in both
   * orientations — came back at exactly 50%.
   *
   * The reason: fire degrades morale one step per hit, and a sub-unit at
   * Combat Strength 10 dies to four hits. So everything is broken by about
   * the time it is destroyed whatever its crew, and a check that can only
   * ever cost a step has nothing left to protect. Quality was being applied
   * faithfully to a quantity that was already saturated.
   *
   * Letting a pass rally gives quality somewhere to show: good troops steady
   * themselves between turns and poor ones spiral.
   */
  rallyOnPass: boolean;
}

export interface SightingTable {
  /** 2D6 + modifiers at or above this is a full sighting. */
  fullAt: number;
  /** At or above this, a partial contact. Below, nothing. */
  partialAt: number;
  /** Modifier per full kilometre of range. */
  perKilometre: number;
  targetInCover: number;
  targetMoved: number;
  targetConcealed: number;
  observerIsRecce: number;
  /**
   * Looking from, through or into smoke (9.2.2.4). The rulebook's own -2.
   *
   * The DirF equivalent already existed as `drms.smoke` and had never fired,
   * because nothing could put smoke on the map.
   */
  throughSmoke: number;
}

/**
 * Indirect fire (9.2.2): mortars, and the smoke they lay.
 *
 * ⚠ THE ONE THING IN THIS FILE WITH NO SOURCE AT ALL.
 *
 * Direct fire has the L6 profiles behind it — ranges, penetration, combat
 * strength indices, all measured by somebody. Indirect fire has nothing: the
 * catalogue contains no mortar, no artillery piece, and no attack helicopter,
 * not even the empty shells it carries for infantry. The Fire Results Table's
 * IDF columns are on Player Aid 4, which is not in the box we have.
 *
 * So every number here is ours except the three the rulebook states in prose,
 * which are marked. That is a bigger declared surface than anywhere else in
 * the ruleset and it is why `indirectFire` is off by default.
 */
export interface IndirectFireRule {
  /**
   * Everything within this of a hit may lose a step of morale (9.2.2).
   *
   * "If a Hit is achieved on the targeted FE, other FEs (Friendly or Enemy)
   * within 250m of it may take a level of Morale Status loss." FRIENDLY OR
   * ENEMY — the first rule in the game that can hurt your own side, and the
   * reason a commander should think before dropping fire onto a melee.
   */
  areaEffectM: number;
  /** Radius a SMOKE marker affects (9.2.2.4). The rulebook's 250 m. */
  smokeRadiusM: number;
  /** Sighting and DirF penalty through smoke. The rulebook's -2, both. */
  smokeDrm: number;
  /** Bonus to the Surprise roll when assaulting through smoke. The rulebook's +2. */
  smokeSurpriseDrm: number;
  /** Penalty for firing at a target that is only Partially Sighted (9.2.2). */
  partialSightingDrm: number;
}

export interface InitiativeTable {
  /** Modifier per net transmission advantage (fewer is better). */
  perTransmissionAdvantage: number;
  /** Modifier per force element lost last turn. */
  perLossLastTurn: number;
}

export interface AssaultTable {
  /** Odds columns, as attacker:defender strength ratios. */
  oddsColumns: number[];
  /** Column shifts, in columns. Negative shifts left (worse for the attacker). */
  shifts: {
    defenderInCover: number;
    defenderSuppressed: number;
    attackerSurprise: number;
    defenderIsVehicleOnly: number;
  };
  /** Modified 2D6 at or above this breaks the defender. */
  defenderBreaksAt: number;
  /** At or below this, the attack is repulsed. */
  attackRepulsedAt: number;
  /**
   * 1D6 at or above this achieves Surprise (9.3.4 step 2).
   *
   * The rulebook's own threshold: "1-3 No Surprise / 4-6 Surprise". Its
   * ASSAULT SURPRISE TABLE of modifiers is on Player Aid 6, which is not in
   * the box we have, so the roll is unmodified here. That is a known gap
   * rather than a house choice.
   */
  surpriseAt: number;
  /**
   * DRM on Defensive Fire. THE RULEBOOK'S OWN FIGURE, unusually for this file:
   * 9.3.2 says Defensive Fire "is resolved as an individual DirF (with a -2
   * DRM - see DF MODIFIERS TABLE)".
   */
  defensiveFireDrm: number;
  /**
   * Everyone within this of the Assault Location is in the Assault (9.3).
   *
   * "Any enemy FE/Group within 250m of the location participates in the
   * Assault as the 'defender'." It is also the line between who may Defensive
   * Fire and who may Reactive Fire: participants defend, everyone else shoots
   * in from outside (9.3.2).
   */
  defenderRadiusM: number;
  /**
   * How far a Retreat or Withdrawal goes (9.3.6, 9.3.7).
   *
   * "must Move a minimum of 500m, and may move up to 1,000m, away from the
   * enemy." Both figures are the rulebook's. We take the minimum, because the
   * choice between them belongs to a commander and no commander is asked yet —
   * see the retreat debt in wiring.test.ts.
   */
  retreatMinM: number;
  retreatMaxM: number;
  /** Ammo an attacker needs to stay in an assault (9.3.7). The rulebook's 2. */
  assaultAmmoFloor: number;
}

/**
 * Mechanics that can be switched off.
 *
 * All default OFF except the core loop. A mechanic earns inclusion by
 * demonstrably changing decisions — which is measurable, see rules/events.
 */
/**
 * Mechanics that can be switched off, one at a time, and measured.
 *
 * EVERY FLAG HERE MUST BE CONSULTED BY SOMETHING. rules/wiring.test.ts fails
 * the build otherwise, and that guard exists because a sweep once condemned
 * all ten mechanics as having "no measurable effect" when in fact seven of
 * them were never implemented. A declared-but-inert flag does not merely fail
 * to help — it corrupts the findings about the ones that work.
 *
 * `mountedInfantry` and `indirectFireMarkers` were deleted rather than
 * implemented: there is no infantry and no artillery in any force list, so
 * neither could be tested, and carrying an untestable flag is the thing this
 * file is not allowed to do. They come back when a force list needs them.
 *
 * `transmissions` and `electronicWarfare` were implemented, swept, and then
 * DELETED ON THE EVIDENCE. Both returned 0% of decisions and 0% of outcomes
 * changed across all five force lists and 100 seeds each. The reason is
 * structural, not a tuning problem: every activation cost both sides exactly
 * one transmission, so the difference between them was always zero, so the
 * initiative bonus never applied to either side and the chit penalty was
 * always symmetric. No commander could choose to go quiet, so there was no
 * decision to make and no asymmetry to exploit - only bookkeeping.
 *
 * They earn a second attempt if a commander is ever given a radio-silence
 * action, which would make emission a real trade. Until then they were a
 * rules page describing nothing. SideState still counts transmissions; no
 * rule reads the count.
 */
export interface ModuleFlags {
  /** Fire consumes rounds; an empty sub-unit cannot engage. */
  ammunition: boolean;
  /** A round that cannot defeat the armour cannot destroy the target. */
  penetration: boolean;
  /**
   * Penetration reads the ASPECT the shot arrived from, and the MUNITION it
   * was fired with, rather than one frontal number and one falloff.
   *
   * Requires `penetration`; on its own it does nothing. Separate from it so
   * the sweep can answer a question the old flag could not: how much of the
   * penetration rule's effect came from modelling armour at all, and how much
   * from modelling where it is thick and what is hitting it. Flanking a tank
   * is a real tactic only under this flag; without it a flank shot meets the
   * glacis.
   */
  facingArmour: boolean;
  /**
   * The R of ARC: Reactive Fire (7.1.3).
   *
   * Without this the Action-Reaction Round is only the Action. Activations
   * execute one at a time and nobody interferes, so a troop can drive across
   * a loaded gun's frontage at 400 m and take no fire.
   */
  reactionFire: boolean;
  /**
   * The C of ARC: the Counteraction Round (7.2) — Reserve Movement, then
   * Counteraction Fire by anything that has not yet fired.
   *
   * Separate flag from `reactionFire` because they are separate mechanics
   * that answer separate questions, and a sweep that cannot tell them apart
   * cannot say which one earned its place. Off by default like everything
   * else here, which means the DEFAULT ruleset plays an incomplete ARC — that
   * is deliberate, and it is what makes the completion measurable.
   */
  counteraction: boolean;
  /**
   * Surprise, and the fire an assault has to go in through (9.3.1-9.3.2).
   *
   * Without it an assault is a bare odds comparison: the defender never gets
   * to shoot at the people walking towards it, and Surprise — which the
   * rulebook rolls for on every assault and which suppresses both Reactive
   * and Defensive Fire — never happens. `attackerSurprise` was a declared
   * column shift that nothing ever set, and it sat on the wiring guard's debt
   * list for exactly that reason.
   */
  defensiveFire: boolean;
  /**
   * What happens AFTER an assault: Retreat, Melee and REORG (9.3.6-9.3.10).
   *
   * Without it an assault is a roll with no aftermath. A broken defender
   * stayed exactly where it was, a repulsed attacker sat on the objective it
   * had just failed to take, and the MELEE marker — which 9.3.8 says
   * explicitly is NOT removed at the end of the turn — was wiped by clean-up
   * every turn, so close combat never lasted longer than the roll that
   * started it.
   */
  closeCombat: boolean;
  /**
   * Firing and assaulting together (9.2.1 COMBINED, 9.3 COMBINED).
   *
   * ⚠ THE FIRE TABLE IS CALIBRATED AROUND THIS AND COULD NOT REACH IT.
   *
   * `resolveDirectFire` has always taken an ARRAY of firers, summed their
   * Combat Strength and applied the worst modifier among them — BGWS's rule,
   * and the file says why it is a good one. Every call site passed exactly
   * one element. `optionsFor` generates one option per firer per target and
   * `resolveAction` resolves one actor, so no sequence of play could produce
   * a combined shot.
   *
   * That is not a missing nicety. HOUSE_V1's fire columns are explicitly
   * spaced "wide enough to make concentration of force the obviously correct
   * play, because a wargame in which massing does not pay teaches the wrong
   * lesson" — and the game could not mass. The central claim of the ladder
   * was untestable, underneath every number in reports/.
   */
  combinedFire: boolean;
  /**
   * Indirect fire and smoke (9.2.2).
   *
   * Off by default like everything else, and with more reason than most: it
   * is the only mechanic in the game whose every figure is declared rather
   * than sourced. See IndirectFireRule.
   */
  indirectFire: boolean;
  /**
   * Concealment, and with it the Attempt Sighting interrupt (2.1.15, 10.0).
   *
   * The interrupt is folded in here rather than given a flag of its own
   * because it can only ever fire against a Concealed FE: with no
   * concealment there is nothing for a non-activating element to attempt to
   * sight, so a separate flag would be one that could never be measured
   * independently. See attemptSightingInterrupt.
   */
  concealment: boolean;
  /** Decoy elements that absorb fire until engaged. */
  dummies: boolean;
  /** A side may only activate as many elements as its HQs can command. */
  commandActivations: boolean;
  /** Three-state contact instead of seen / not seen. */
  partialSighting: boolean;
  /** Five-step morale ladder instead of good / broken. */
  moraleLadder: boolean;
  /**
   * A move halts when it makes contact (the other half of 7.1's interrupt).
   *
   * ⚠ WITHOUT IT, ELEMENTS DRIVE STRAIGHT PAST EACH OTHER. A move used to be
   * atomic: the loop took the turn's destination and put the element there,
   * with nothing happening in between. Two troops whose routes crossed swapped
   * positions inside one turn, each arriving behind the other, and noticed only
   * at the next turn's sighting sweep — by which time they were past and facing
   * the wrong way. On the map it looked like two units ignoring each other at
   * 300 m; in a meeting engagement it is the single most important event there
   * is.
   *
   * With it on, a move is walked in 200 m steps and any enemy the moving side
   * has NOT already sighted gets a sighting attempt the moment it has line of
   * sight. The first success stops the element there and abandons its march.
   * An element already in contact is not stopped — it saw the enemy and its
   * commander chose to move anyway, which BGWS plainly allows.
   */
  contactHalt: boolean;
  /**
   * A march: a route planned once and walked over several turns.
   *
   * ⚠ WITHOUT IT, "GO TO THE OBJECTIVE" IS RE-DECIDED EVERY TURN FROM SCRATCH.
   * A one-turn bound can only ever move towards the goal, so it cannot go
   * ROUND anything: skirting a lake means ending the turn further from the
   * objective than it started, which a bound will never choose. A route can,
   * because the route is the plan and the turn is only a slice of it.
   *
   * With it on, an element given a distant objective plans a way there — the
   * raster's A* in the browser, a greedy bearing search offline — commits to
   * it, and spends one turn's allowance along it per turn. Sighting an enemy
   * abandons the plan, because a march is a plan made in the absence of one.
   */
  routeMarch: boolean;
  /**
   * Rally: the Command Sub-phase's first step (5.2).
   *
   * Without it the morale ladder only goes down for anything that is
   * suppressed without being damaged, which is most of what gets shot at.
   * ON in CORE_MODULES for the same reason as moraleLadder and
   * terrainMovement: a rule whose absence makes a state absorbing is not an
   * optional refinement. The flag exists so the sweep can price it, and the
   * sweep priced it at 40/40 decisions changed on every force list, with
   * unresolved games falling from 14 in 160 to 1 in 160. See RallyRule.
   */
  rally: boolean;
  /**
   * Movement costs the ground it crosses, and impassable ground stops it.
   *
   * ⚠ THE TABLE BELOW (`movement`) EXISTED FOR WEEKS WITH NO CALLER.
   *
   * `consumeAllowance` in lib/movement.ts implemented the rule — an allowance
   * per Move Type per terrain class, spent in fractions of a turn, with a
   * missing or zero entry meaning IMPASSABLE rather than slow — and every
   * RuleSet carried a table for it. No sequence of play consulted either.
   * `optionsFor` offered a destination 40% of the way towards the enemy and
   * the loop put the element there: across a river, up a cliff and through a
   * forest at the same rate as down a road.
   *
   * With this on, a move option is CLIPPED to what the Move Type can actually
   * cover this turn over the terrain in the way, and is withheld entirely when
   * the element cannot leave the ground it is on. The commander therefore
   * cannot choose an impossible move, which is the loop's whole contract.
   *
   * It is a flag rather than unconditional behaviour for the usual reason: so
   * the sweep can price it. It is ON in CORE_MODULES, which almost nothing
   * else is — see the note there.
   */
  terrainMovement: boolean;
}

/**
 * Whether a round can hurt what it hit.
 *
 * Until this existed, an ATGM and a coaxial machine gun damaged a Challenger 2
 * identically: nothing asked whether a round could get through. Forty years of
 * armour development bought nothing, and the only reason a modern tank beat a
 * 1950s one was a Combat Strength bonus derived from its protection band.
 *
 * ⚠ THIS COMMENT USED TO SAY "`targetClass` gated WHETHER you could engage".
 * It did not. `targetClass` was parsed from the profiles, shown in the Asset
 * Explorer, and read by no rule anywhere. The sentence was written as
 * background to a real fix and quietly became the reason nobody checked —
 * the one place a reader would look to see whether 9.2.1's Apers/Atk rule
 * existed said that it did. It exists now; see capabilityCanEngage.
 *
 * ⚠ UNKNOWN PENETRATION FAILS OPEN, and that is the whole design.
 *
 * The source has no penetration curve for much of the catalogue — 751 WWII
 * rounds have none at all, and Challenger 2 and Type 59 are both marked
 * `fragmentation_not_penetration` upstream. Treating absent as zero would
 * silently render a Challenger 2 unable to damage a Warrior: a flagship unit
 * quietly disabled by missing data, with nothing failing and nobody told.
 *
 * So a round with no known penetration is assumed to penetrate. That is
 * generous and it is visible, which is the right way round. The alternative
 * is a rule that punishes gaps in the data rather than weaknesses in the gun.
 */
export interface PenetrationRule {
  /**
   * What insufficient penetration does.
   *
   * `hardGate` — hits become suppression. You can pin it, you cannot kill it.
   *
   * ⚠ `columnShift` WAS ALSO DECLARED HERE, AND WAS NEVER IMPLEMENTED. The
   * type admitted it, `columnsLost: 2` configured it, the doc comment
   * described it as "softer and faster" — and NOTHING READ EITHER. The single
   * use of this field is one `=== "hardGate"` test in resolvers.ts, so
   * selecting `columnShift` did not soften the rule, it silently switched
   * armour OFF: every bounce became a full-effect hit.
   *
   * It failed in the PERMISSIVE direction, which is what makes it worth this
   * many lines. A sweep run to decide whether the hard gate stalls games
   * reports the column shift as changing nothing at all on every force list —
   * 0 decisions, 0 outcomes, 0 turns — and the natural reading of that is
   * "the softer option is harmless", when the truth is that it removed the
   * mechanic being measured. That is how a tuning pass talks itself into
   * turning off armour.
   *
   * So the union is narrowed to what exists. Choosing the missing option is
   * now a COMPILE error rather than a quiet nerf. To bring it back: restore
   * the union member and `columnsLost`, and shift the result down that many
   * columns where `gated` is computed — `fireColumnFor` picks by combined
   * strength, so the shift needs a column index, not another strength lookup.
   * It is a rules change and wants its own sweep.
   */
  onFailure: "hardGate";
  /**
   * Penetration is quoted at 1 km. Beyond that it falls off by this fraction
   * per additional kilometre, for rounds whose falloff the source does not
   * describe. Shaped charges are flat with range and set falloff to zero.
   *
   * ⚠ THIS USED TO APPLY TO EVERY ROUND IN THE GAME, WHICH WAS WRONG RATHER
   * THAN APPROXIMATE. The comment above always said shaped charges are flat;
   * nothing implemented it, because a capability carried a single
   * penetration number with no munition type attached. Every Kornet, Javelin
   * and RPG in the game therefore weakened with distance, which is not how a
   * shaped charge works — the jet is formed on impact. Now that capabilities
   * declare a MunitionKind, this applies to kinetic rounds only.
   */
  falloffPerKmBeyond1Km: number;
  /**
   * How much of a target's chemical-energy protection a TANDEM warhead
   * removes, where explosive reactive armour is fitted.
   *
   * ⚠ CALIBRATED AGAINST THE DATA, NOT CHOSEN. The curated calibration matrix
   * resolves a Challenger 2's side at 400 mm against a plain shaped charge
   * and 240 mm against a tandem one. 0.4 reproduces that exactly, so the
   * rule agrees with the reference implementation rather than competing with
   * it.
   */
  tandemEraDefeatFraction: number;
}

/**
 * Reactive Fire — the R of ARC (Core Rules 7.1.3).
 *
 * "It is possible for an FE/Group of the non-Activating side to interrupt a
 * Move or Assault Action ... by Firing (DirF only) at it while it is moving.
 * ... It reflects that FE/Group operating in an 'overwatch' capacity, with a
 * remit to 'watch and shoot'."
 *
 * ⚠ TWO DELIBERATE DEVIATIONS FROM THE RULEBOOK, BOTH DECLARED.
 *
 * 1. THE RULEBOOK CHARGES NO DRM. Its cost for reacting is positional, not
 *    arithmetic: the reactor must not have Activated, and having fired it
 *    "cannot take any further Action for the remainder of the Turn". That
 *    cost is now implemented, so `snapShotDrm` is an ADDITIONAL house penalty
 *    on top of it and is ours to justify or delete. It is kept at -2 because
 *    a shot taken at a target that chose the moment is not a deliberate one,
 *    and because it is measurable: set it to 0 and sweep.
 *
 * 2. THE RULEBOOK GATES REACTION ON ORDERS AND GROUND, NOT ON A ROE SETTING.
 *    An FE may only Reactive Fire if it holds one of eleven Order Verbs
 *    (BLOCK, CONTAIN, DEFEAT, DEFEND, DELAY, DISRUPT, FIX, HOLD, INTERDICT,
 *    SUPPRESS, TURN) AND the target is moving in a TAI assigned to that
 *    specific FE on the Planning Map. We model neither Order Verbs nor TAIs,
 *    so `StandingEngagement` stands in for both: it is the same decision —
 *    "which of my elements are watching, and what will they shoot at" — taken
 *    at the same moment, in the Command/Orders step, and binding for the turn.
 *    When Order Verbs and TAIs arrive, this collapses into them.
 */
export interface ReactionRule {
  /** Applied to a Reactive Fire shot. Negative is a penalty. House, not BGWS. */
  snapShotDrm: number;
  /**
   * What an element does when its commander declared no standing order.
   *
   * A default of "never" would make the mechanic invisible unless every
   * commander remembered to set it, and a model that forgets is then
   * indistinguishable from a mechanic that does nothing.
   */
  defaultEngage: StandingEngagement;
  /**
   * Elements that may react to a single action. Caps a cascade.
   *
   * HOUSE, AND STRICTER THAN THE RULEBOOK, which says plainly: "It is possible
   * for multiple FE/Groups of the non-Activating side to take Reactive Fire
   * against the same moving enemy FE, if the above criteria are met" — no cap
   * at all. It is capped here because the rulebook's own limiter is a human
   * one (a player will not burn six elements' entire turns on one truck) and
   * a heuristic commander has no such instinct. Raise it to Infinity and
   * sweep if you want to find out what the uncapped game plays like.
   */
  maxReactorsPerAction: number;
}

/**
 * The Counteraction Round — the C of ARC (Core Rules 7.2).
 *
 * ⚠ THIS IS A ROUND, NOT A RIPOSTE, AND THE DIFFERENCE IS THE WHOLE MECHANIC.
 *
 * It was first built as an immediate answer-back: the actor shooting the
 * element that had just reacted to it, inside the same activation. That is
 * what "Action-Reaction-Counteraction" sounds like it means, and it is not
 * what BGWS does. 7.2 is a second round of alternating activations after
 * every FE has acted, containing exactly two things:
 *
 *   7.2.1 RESERVE MOVEMENT  FEs with a Reserve Order and no FIRED marker move
 *                           up to 1,000 m towards a priority — "even if they
 *                           have a MOVED marker" — and may then DirF or Hasty
 *                           Assault. Subject to Reactive Fire.
 *   7.2.2 COUNTERACTION FIRE Any FE without a FIRED marker may DirF, at a
 *                           penalty. Sides alternate. "A side that has passed
 *                           cannot at a later point ... declare that it wishes
 *                           to DirF. Passing is final for the Turn."
 *
 * The riposte reading made holding fire worthless and made a reserve
 * meaningless — there was nothing for an uncommitted element to do. This
 * reading makes both real: an element that did not fire in the first round
 * still has a shot in the second, so keeping one silent is a live choice.
 */
export interface CounteractionRule {
  /**
   * DRM on a DirF taken in the Counteraction Round (7.2.2).
   *
   * The rulebook says "note that DRM penalties for Firing in the
   * Counteraction Round apply" and puts the figure on Player Aid 4, which we
   * do not have. -1 is OURS: enough that firing in the first round is the
   * better shot, small enough that holding a reserve is not self-harm.
   */
  fireDrm: number;
  /**
   * Share of a side's elements that may be nominated Reserve (2.1.13).
   *
   * "Only one-third of FE/Groups in a side may be given a Reserve Order."
   * A cap, not a target — a side may hold none.
   */
  reserveFraction: number;
  /** Reserve Move distance, metres. The rulebook's own figure (7.2.1). */
  reserveMoveM: number;
}

/** When an element will answer an enemy action it can see. */
export type StandingEngagement =
  /** Hold fire. Stay concealed, keep the ammunition. */
  | "never"
  /** Only answer an element that has just engaged one of yours. */
  | "ifFiredUpon"
  /** Answer anything that comes inside short range. */
  | "withinShortRange"
  /** Answer anything you can reach. */
  | "always";

/** Rounds a sub-unit carries, and what command can reach. */
export interface LogisticsTable {
  /** Engagements per capability before a sub-unit is dry. */
  roundsPerCapability: number;
  /** Elements a side may activate per turn with no surviving HQ. */
  activationsWithoutHq: number;
  /** Added per point of an HQ's command rating. */
  activationsPerCommandRating: number;
}

export interface RuleSet {
  id: string;
  name: string;
  /** Where the numbers came from. Shown in the UI and on every export. */
  provenance: string;
  /** Minutes of game time per turn. Movement allowances are derived from it. */
  turnMinutes: number;
  /**
   * Co-location, in metres. The Glossary's own definition:
   *
   *   "Co-located — An FE within 250m of another Friendly FE is Co-located
   *    with it."
   *
   * At the RuleSet root because the rulebook uses it globally rather than per
   * mechanic: it is the radius for Combined Fire (9.2.1), for who is drawn
   * into an Assault (9.3), for Mounting (9.01) and for an HQ steadying a unit
   * in a Morale Check.
   */
  coLocatedM: number;
  /**
   * Half the frontal arc, in degrees. Beyond it, a shot is a flank or rear
   * shot and earns `drms.flank`.
   *
   * ⚠ `flank` WAS A DECLARED MODIFIER THAT NOTHING COULD SET, and it was
   * excused on the wiring guard's debt list as "needs a commander that
   * manoeuvres for aspect rather than firing frontally". That was the same
   * misdiagnosis `attackerSurprise` carried: there was no facing on a Force
   * Element and no code computing aspect, so the cleverest commander alive
   * would not have made it fire once.
   *
   * 60° gives a 120° frontal arc, which is roughly a tank's glacis and
   * turret front. OURS — the rulebook puts aspect on Player Aid 4, which is
   * not in the box we have.
   */
  frontArcDeg: number;
  movement: AllowanceTable;
  combatStrength: CombatStrengthRule;
  lethality: LethalityRule;
  fireColumns: FireColumn[];
  drms: DrmTable;
  morale: MoraleTable;
  victory: VictoryRule;
  rally: RallyRule;
  sighting: SightingTable;
  initiative: InitiativeTable;
  assault: AssaultTable;
  logistics: LogisticsTable;
  penetration: PenetrationRule;
  reaction: ReactionRule;
  counteraction: CounteractionRule;
  indirectFire: IndirectFireRule;
  modules: ModuleFlags;
}

/** Core loop on, everything else off until it argues its way in. */
export const CORE_MODULES: ModuleFlags = {
  ammunition: false,
  penetration: false,
  facingArmour: false,
  reactionFire: false,
  counteraction: false,
  defensiveFire: false,
  closeCombat: false,
  combinedFire: false,
  indirectFire: false,
  concealment: false,
  dummies: false,
  commandActivations: false,
  partialSighting: false,
  moraleLadder: true,
  // ON, like moraleLadder and unlike everything else here, because it is not
  // an optional refinement: with it off the loop offers moves the rules
  // forbid, and "a commander never sees an illegal option" is the contract
  // optionsFor is written to keep. A module that is on by default is still
  // measurable — the sweep flips it both ways.
  terrainMovement: true,
  // ON for the same class of reason: with it off, morale for an undamaged but
  // suppressed element is an absorbing state.
  rally: true,
  // ON: without it nothing can go round an obstacle, because going round costs
  // ground in the turn it happens and a one-turn bound never pays that.
  routeMarch: true,
  // ON: without it two troops can swap positions inside a turn without either
  // noticing the other, which is not a subtlety, it is the whole engagement.
  contactHalt: true,
};

/**
 * HOUSE RULES v1 — ours.
 *
 * Calibrated by hand against 2D6, whose mean is 7. Reading the fire ladder:
 * a weak firer needs an above-average roll to do anything at all, and a strong
 * one suppresses on almost anything. The gap between columns is deliberately
 * wide enough to make concentration of force the obviously correct play,
 * because a wargame in which massing does not pay teaches the wrong lesson.
 *
 * These numbers are a starting point for play, not a claim about combat. They
 * are expected to move.
 */
/**
 * Minutes in a turn, in ONE place.
 *
 * It was written twice — `turnMinutes: 15` and `allowanceFromSpeeds(15)` — and
 * the two could have drifted silently, with the movement table quietly priced
 * for a different turn length than the one the ruleset declares. Now the table
 * is derived from the declared figure.
 */
const TURN_MINUTES = 15;

export const HOUSE_V1: RuleSet = {
  id: "house-v3",
  name: "House rules v3",
  provenance:
    "Ours. Structure follows BGWS; the numbers are not BGWS's and are not doctrine. " +
    "v2 calibrated by harness sweep (scripts/bgwsTune.ts) rather than by hand: " +
    "v1 ran 27-29 turns with a third of games hitting the turn limit, which makes " +
    "every unresolved game a draw and every outcome comparison worthless. " +
    "strengthPerHit 3 and every fire threshold one point easier brings both " +
    "symmetric force lists to a mean of 11.7 turns with 1 of 80 games at the limit, " +
    "and leaves them balanced (23/17 and 22/18 over 40 seeds). " +
    "v3 changes ONE number and it is the movement table: vehicle allowances were " +
    "march rates taken from a route planner's speed table (5,000 m of open ground " +
    "in a 15-minute turn for tracks), so the allowance sat 3-8x above the largest " +
    "bound the turn loop ever asked for and no terrain could bind it. Scaled to a " +
    "tactical rate at VEHICLE_CONTACT_FRACTION 0.45 — swept, 40 seeds x 4 force " +
    "lists x 5 candidates — movement is now decided by the ground rather than by " +
    "the loop's invented fraction, games run 11.9-14.0 turns with 3-4 of 40 at the " +
    "limit, and the symmetric control list comes out 20/20. The v3 module-impact " +
    "report has NOT been regenerated (no environment available that can run the " +
    "full sweep); reports/module-impact-house-v2.* describe v2's tempo, not this " +
    "one, and should be replaced rather than read.",
  turnMinutes: TURN_MINUTES,
  // The Glossary's figure, not ours.
  coLocatedM: 250,
  // A 120-degree frontal arc. Ours; Player Aid 4 is not in the box we have.
  frontArcDeg: 60,
  /**
   * How far each Move Type gets over each terrain in one turn.
   *
   * Derived from this repo's speed table, with vehicles scaled to a tactical
   * rate rather than a march rate — see VEHICLE_CONTACT_FRACTION, which is
   * where the argument and the arithmetic are.
   *
   * At the march rate this used to be, a tracked element covered 5,000 m of
   * open ground in a turn: a 10 km board in two turns, arriving before
   * anything could be sighted at the 3 km cap, with the ground it crossed
   * unable to matter. That is why the allowance, once wired in, measured as
   * having no effect for vehicles — and the sweep found it is also the worst
   * setting for resolution, stalling the advance-to-contact list in 15 of 40
   * games. Tracks now cross 2,250 m of open ground, 450 m of woodland and
   * 5,062 m of road.
   *
   * The fraction was swept (40 seeds x 4 force lists x 5 candidates) and 0.45
   * won on game length, on games reaching a result, and on the balance canary,
   * which came out dead even. The table is in VEHICLE_CONTACT_FRACTION's
   * docstring; `scripts/bgwsMovementTune.ts` reproduces it.
   *
   * It also agrees with the rulebook, which was read afterwards: 9.1 bounds
   * Maximum Allowable Distance to "a figure between 1 and 6", each value
   * 1,000 m, off the TERRAIN EFFECTS TABLE on Player Aid 3. 2,250 m of open
   * ground for tracks is a "2" on that scale; the 5,000 m this replaced was a
   * "5", for cross-country movement.
   */
  movement: allowanceFromSpeeds(TURN_MINUTES, VEHICLE_CONTACT_FRACTION),
  // Calibrated so a four-platform troop lands mid-table rather than off the
  // end of it: 8.6 * 0.5 + 4 = 8.3, which reads as CS 8 on the "CS 6-9"
  // column. A three-platform platoon of the same equipment comes out at 7,
  // so count matters without swamping the table.
  combatStrength: {
    perPlatformWeight: 0.5,
    countWeight: 1,
    // Roughly tracks the armour each band actually averages in the source
    // (831 / 293 / 150 / 36 / 5 mm), compressed into something a fire table
    // can use. A Challenger troop comes out at 10 against a Warrior
    // platoon's 8 — which the upstream index alone could not express at all.
    protectionBonus: {
      very_heavy: 7,
      heavy: 2,
      medium: 0,
      light: -1,
      soft_skin: -2,
      unknown: 0,
    },
    min: 1,
    max: 16,
  },
  // Two strength per hit brings a troop of four Challengers (CS 8) down in
  // four hits rather than eight, which is what pulls a game back inside the
  // 10-15 turn band BGWS plays in.
  lethality: { strengthPerHit: 3, moraleStepsPerHit: 1 },
  // Every threshold one point easier than v1. Swept, not guessed — see the
  // provenance note and scripts/bgwsTune.ts.
  fireColumns: [
    { label: "CS 1-2", minCombatStrength: 0, suppressAt: 7, oneHitAt: 10, twoHitsAt: 12, threeHitsAt: 99 },
    { label: "CS 3-5", minCombatStrength: 3, suppressAt: 6, oneHitAt: 9, twoHitsAt: 11, threeHitsAt: 99 },
    { label: "CS 6-9", minCombatStrength: 6, suppressAt: 5, oneHitAt: 8, twoHitsAt: 10, threeHitsAt: 12 },
    { label: "CS 10-15", minCombatStrength: 10, suppressAt: 4, oneHitAt: 7, twoHitsAt: 9, threeHitsAt: 11 },
    { label: "CS 16+", minCombatStrength: 16, suppressAt: 3, oneHitAt: 6, twoHitsAt: 8, threeHitsAt: 10 },
  ],
  drms: {
    targetInCover: -2,
    targetMoved: 1,
    targetSuppressed: 1,
    firerMoved: -2,
    firerSuppressed: -2,
    smoke: -2,
    longRange: -1,
    flank: 1,
  },
  // House numbers, all of them, and all expected to move. Six engagements
  // per capability is enough that ammunition bites in a long fight and not a
  // short one, which is the behaviour worth measuring.
  logistics: {
    roundsPerCapability: 6,
    activationsWithoutHq: 2,
    activationsPerCommandRating: 1,
  },
  // Hard gate: if you cannot get through, you can pin it and nothing more.
  // Chosen over a column shift because it is the honest reading of armour,
  // and because the sweep can measure whether it stalls games.
  //
  // ⚠ IT NOW HAS, AND THE ANSWER IS YES. That measurement was impossible
  // until the platforms carried real penetration figures: a Challenger 2 and
  // a Type 59 stated none, so `canPenetrate` failed open for them and the
  // penetration module scored 0 decisions and 0 outcomes on three of the six
  // force lists. It read as a rule that did nothing. It was a rule with
  // nothing to bite on.
  //
  // With the figures sourced, the hard gate adds 25 to 31 turns against a
  // 40-turn cap on every list where peer tanks meet — they stop being able to
  // kill each other and the games run to the limit. The one list that stays
  // short is the generation probe, where an Abrams meets a T-55 and the
  // answer is decided rather than deadlocked, which is the mechanic working.
  //
  // So this constant is now a known open question rather than an untested
  // assumption. Softening it is a rules decision; see PenetrationRule for why
  // the obvious softer option is not available to reach for.
  penetration: {
    onFailure: "hardGate",
    falloffPerKmBeyond1Km: 0.15,
    tandemEraDefeatFraction: 0.4,
  },
  // A snap shot at a moving target, two points worse than deliberate fire.
  // Default "withinShortRange": aggressive enough that the mechanic shows up
  // without a commander having to opt in, cautious enough that a force does
  // not empty itself at maximum range.
  reaction: { snapShotDrm: -2, defaultEngage: "withinShortRange", maxReactorsPerAction: 2 },
  // 250 m, -2 and +2 are the rulebook's, stated in prose in 9.2.2 and
  // 9.2.2.4. The partial-sighting penalty is OURS: 9.2.2 says a DRM applies
  // when firing at a Partially Sighted target without saying what it is.
  indirectFire: {
    areaEffectM: 250,
    smokeRadiusM: 250,
    smokeDrm: -2,
    smokeSurpriseDrm: 2,
    partialSightingDrm: -2,
  },
  // One third is the rulebook's (2.1.13); 1,000 m is the rulebook's (7.2.1);
  // the -1 is ours, because Player Aid 4 is not in the box we have.
  counteraction: { fireDrm: -1, reserveFraction: 1 / 3, reserveMoveM: 1000 },
  morale: {
    passTarget: 11,
    penaltyAtTotalLoss: 4,
    multipleDirections: 2,
    hqPresent: 2,
    rallyOnPass: true,
  },
  // 5.2. The 4+ is the rulebook's; the quality DRM stands in for a C2 rating
  // we do not model, and the bands are ours.
  rally: { passTarget: 4, qualityDrm: 1, goodAt: 4, poorAt: 2 },
  // 3.1's grades, our thresholds. See VictoryRule.
  victory: { holdWithinM: 500, combatIneffectiveBelow: 1 / 3, marginalEffectivenessGap: 0.2 },
  sighting: {
    fullAt: 9,
    partialAt: 6,
    perKilometre: -2,
    targetInCover: -2,
    targetMoved: 2,
    targetConcealed: -3,
    observerIsRecce: 2,
    // The rulebook's own -2, same figure as the DirF penalty.
    throughSmoke: -2,
  },
  initiative: {
    perTransmissionAdvantage: 1,
    perLossLastTurn: -1,
  },
  assault: {
    oddsColumns: [0.5, 1, 1.5, 2, 3, 4, 6],
    shifts: {
      defenderInCover: -1,
      defenderSuppressed: 1,
      attackerSurprise: 1,
      defenderIsVehicleOnly: 2,
    },
    defenderBreaksAt: 8,
    attackRepulsedAt: 5,
    // The rulebook's figures, all three: 4-6 on 1D6 is Surprise (9.3.4), the
    // -2 is quoted verbatim in 9.3.2, and 250 m is the Assault radius (9.3).
    surpriseAt: 4,
    defensiveFireDrm: -2,
    defenderRadiusM: 250,
    retreatMinM: 500,
    retreatMaxM: 1000,
    assaultAmmoFloor: 2,
  },
  modules: CORE_MODULES,
};

/** The column a combined Combat Strength falls in. */
/**
 * Combat Strength for a sub-unit of `platformCount` platforms.
 *
 * Additive in count, not multiplicative. See CombatStrengthRule for what
 * happened when it was the other way round.
 */
export function combatStrengthFor(
  ruleset: RuleSet,
  platformCsIndex: number,
  platformCount: number,
  protectionBand?: string,
): number {
  const rule = ruleset.combatStrength;
  // An unrecognised band contributes nothing rather than throwing: a new band
  // upstream should make a unit slightly wrong, not stop the game.
  const protection = protectionBand ? (rule.protectionBonus[protectionBand] ?? 0) : 0;
  const raw =
    platformCsIndex * rule.perPlatformWeight + platformCount * rule.countWeight + protection;
  return Math.min(rule.max, Math.max(rule.min, Math.round(raw)));
}

export function fireColumnFor(ruleset: RuleSet, combatStrength: number): FireColumn {
  let column = ruleset.fireColumns[0];
  for (const candidate of ruleset.fireColumns) {
    if (combatStrength >= candidate.minCombatStrength) column = candidate;
  }
  return column;
}

/** Read a fire column against a modified roll. */
export function fireResultFor(column: FireColumn, modifiedRoll: number): FireResult {
  if (modifiedRoll >= column.threeHitsAt) return "threeHits";
  if (modifiedRoll >= column.twoHitsAt) return "twoHits";
  if (modifiedRoll >= column.oneHitAt) return "oneHit";
  if (modifiedRoll >= column.suppressAt) return "suppress";
  return "miss";
}

/** Hits a result inflicts. Suppression is not a hit; it is a morale event. */
export function hitsFor(result: FireResult): number {
  if (result === "threeHits") return 3;
  if (result === "twoHits") return 2;
  if (result === "oneHit") return 1;
  return 0;
}

/**
 * A ruleset with some modules flipped, for A/B comparison.
 *
 * The id encodes the VALUE, not just the key. Naming both variants
 * `house-v1+transmissions` made the two sides of every module comparison
 * indistinguishable in the report whose entire purpose is telling them apart —
 * and a result labelled with the wrong ruleset is worse than an unlabelled one.
 */
export function withModules(ruleset: RuleSet, overrides: Partial<ModuleFlags>): RuleSet {
  const suffix = Object.entries(overrides)
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, value]) => `${key}:${value ? "on" : "off"}`)
    .join(",");
  return {
    ...ruleset,
    id: `${ruleset.id}+${suffix}`,
    modules: { ...ruleset.modules, ...overrides },
  };
}

// `allowanceFor(ruleset, moveType)` used to live here: a one-line accessor for
// the movement table that nothing ever called, written at the same time as the
// table and the allowance rule and left behind when none of the three were
// wired to a sequence of play. The table is now read by the turn loop through
// `config.ruleset.movement`, and an accessor with one caller fewer than none
// is not worth keeping. Deleted rather than excused — the wiring guard's own
// second legitimate option.
