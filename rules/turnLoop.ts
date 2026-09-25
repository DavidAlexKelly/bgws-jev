// ── bgws/rules/turnLoop.ts ─────────────────────────────────────────────────
// The sequence of play: the thing that makes the resolvers a game.
//
// Per turn: Initiative, then an alternating activation round, then clean-up.
// The side with initiative activates first and the other replies, which is
// BGWS's structure and is worth keeping — alternating activation is what makes
// holding a unit back a real decision, and simultaneous turns quietly remove
// most of the interesting choices from a wargame.
//
// The loop OFFERS options and APPLIES results. It never chooses: that is the
// commander's job, and keeping the two apart is what lets a person, a bot and
// a model be swapped without the rules noticing.

import { bearingDeg, bearingDeltaDeg, distanceM } from "../lib/board";
import type { CapabilityClass, TargetClass } from "../data/profiles";
import { projectForSide } from "../lib/fogOfWar";
import { inCover } from "../lib/proceduralTerrain";
import { describeMovePlan, planMoveAround } from "../lib/movePlan";
import {
  advanceAlongRoute,
  bearingRoutePlanner,
  routeInterrupted,
  type PlannedRoute,
  type RoutePlanner,
} from "../lib/routePlan";
import type { ForceElement, GameState, Phase, Side } from "../lib/state";
import type { LatLng } from "../lib/board";
import { degradeMorale, forceElementsOf, hasMarker, opposing, sightingOf } from "../lib/state";
import { lineOfSight, type TerrainSampler } from "../lib/lineOfSight";
import { applyEffects, clearAllMarkers } from "./apply";
import { resolveAssault, retreatTo } from "./assault";
import type { Commander, ActionOption } from "./commander";
import type { Rng } from "./dice";
import { EventLog, type ResolutionEvent } from "./events";
import {
  resolveDirectFire,
  resolveInitiative,
  resolveMoraleCheck,
  resolveRally,
  resolveSighting,
} from "./resolvers";
import { canAdvance, canEngage } from "./resolvers";
import type { RuleSet, StandingEngagement } from "./ruleset";
import { judgeVictory, type Verdict } from "./victory";
import { walkUntilContact } from "./contact";
import type {
  CommanderIntent,
  ReactionCandidate,
  TacticalDecider,
  TacticalTrace,
} from "./tactical";

/**
 * One moment inside a turn, kept so a turn can be played back.
 *
 * ⚠ RECORDED, NOT RECONSTRUCTED, AND THE DIFFERENCE MATTERS. The event log
 * carries the effects of every RESOLUTION, but plenty of state changes are not
 * resolutions: a MOVED marker, a facing, a route being trimmed, clean-up
 * wiping markers, reserves being nominated. Rebuilding the middle of a turn by
 * replaying only the logged effects would drift from what actually happened,
 * and the drift would be invisible — a playback that looks right and is wrong
 * is worse than no playback.
 *
 * So the engine hands out the state it actually had, at the moment it had it.
 */
export interface TurnStep {
  turn: number;
  phase: Phase;
  /** What just happened, in the words the commander was offered or the log used. */
  label: string;
  /** The state immediately AFTER it happened. */
  state: GameState;
  /** Which side acted, where one did. */
  side?: Side;
  /** The element that acted, where one did. */
  actorId?: string;
  /**
   * The resolutions this step produced: the rolls, the modifiers and what they
   * came to.
   *
   * ⚠ A STEP WITHOUT ITS DICE IS A CLAIM WITHOUT ITS EVIDENCE. "B1 fires on
   * R1" is the same sentence whether the shot was a certainty botched or a
   * long chance that landed, and the difference is the entire teaching value
   * of a wargame. The events are carried rather than looked up later because
   * the log is a flat list for the whole game: matching entries back to a step
   * after the fact means guessing, and guessing wrong is invisible.
   *
   * Usually one. A Combined Fire or an assault can produce several.
   */
  events?: ResolutionEvent[];
}

export interface GameConfig {
  ruleset: RuleSet;
  terrain: TerrainSampler;
  /**
   * Called after each step of a turn, if anyone is listening.
   *
   * Optional and side-effecting on purpose: the harness plays tens of
   * thousands of games and wants none of this, and the play screen wants all
   * of it. Nothing in the rules reads it back, so a recorder can never change
   * an outcome.
   */
  onStep?: (step: TurnStep) => void;
  /**
   * How a long march is planned. Optional, and absent means bearings.
   *
   * The browser passes the terrain raster's A*; the harness passes nothing and
   * gets the greedy bearing planner, because it plays tens of thousands of
   * games with no network and therefore no raster. The rules never learn which
   * one they were given — only the route records it, so a map or a log can say.
   */
  routePlanner?: RoutePlanner;
  commanders: Record<Side, Commander>;
  /**
   * Who makes each side's in-the-moment calls — reactive fire, pressing on
   * through contact. Optional, per side, and absent means the declared rules
   * decide, exactly as before. See rules/tactical.ts.
   */
  tactical?: Partial<Record<Side, TacticalDecider>>;
  rng: Rng;
  log: EventLog;
  /** Stop after this many turns even if neither side has broken. */
  maxTurns: number;
}

/**
 * The phases that do not need a commander.
 *
 * Sighting, option generation, resolution, morale and the command budget all
 * read the ruleset, terrain, dice and log and never ask anyone to decide
 * anything. Typing them against this rather than GameConfig is what lets the
 * orders sequence of play (rules/orders.ts) reuse them without a cast, and
 * without either sequence depending on the other's commander interface.
 */
export type PhaseConfig = Omit<GameConfig, "commanders">;

/** Distance within which an assault can be launched (BGWS: co-location). */
const ASSAULT_RANGE_M = 250;

/**
 * Below this, a move has not happened.
 *
 * A bound that covers 20 m still costs the element its activation and its
 * MOVED marker, and still flips its counter face-up. Offering it is offering a
 * commander a way to waste a turn, so a move this short is simply not a move.
 */
const MIN_USEFUL_MOVE_M = 50;

/**
 * The planner this game uses, or the offline one.
 *
 * Built per call rather than held: it closes over nothing that changes within
 * a turn, and a stale planner pointing at last turn's terrain would be a bug
 * nobody would look for.
 */
/** Hand the current state to whoever is recording the turn. */
function recordStep(
  config: PhaseConfig,
  state: GameState,
  turn: number,
  phase: Phase,
  label: string,
  who: { side?: Side; actorId?: string; events?: ResolutionEvent[] } = {},
): void {
  config.onStep?.({ turn, phase, label, state, ...who });
}

function plannersFor(config: PhaseConfig): RoutePlanner[] {
  const bearings = bearingRoutePlanner(config.terrain, config.ruleset.movement);
  return config.routePlanner ? [config.routePlanner, bearings] : [bearings];
}

/**
 * One turn of an existing march.
 *
 * Returns the option to continue, or null when the route cannot be walked any
 * further — which is its own kind of answer and is why the route is cleared
 * rather than silently kept.
 */
function marchOption(
  fe: ForceElement,
  config: PhaseConfig,
  route: PlannedRoute,
): ActionOption | null {
  const progress = advanceAlongRoute(
    config.terrain,
    fe.moveType,
    config.ruleset.movement,
    fe.position,
    route.waypoints,
  );
  if (progress.distanceM < MIN_USEFUL_MOVE_M) return null;

  const left = Math.round(distanceM(progress.end, route.goal));
  return {
    id: `${fe.id}:march`,
    kind: "move",
    actorId: fe.id,
    destination: progress.end,
    route: { ...route, waypoints: progress.remaining },
    summary:
      `${fe.label} continues to ${route.label} ` +
      `(${Math.round(progress.distanceM).toLocaleString("en-GB")} m, ` +
      `${left.toLocaleString("en-GB")} m to go)`,
  };
}

/**
 * Plan a march to somewhere further than a bound, and offer its first turn.
 *
 * ⚠ THIS IS THE DIFFERENCE BETWEEN A MOVE AND A ROUTE. A bound asks "how far
 * towards that can I get this turn"; a march asks "what is the way there" and
 * then spends turns on it. Only the second can go round a lake, because going
 * round a lake means being further from the objective at the end of the turn
 * than at the start — which a one-turn bound will never choose.
 */
function planMarch(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
  goal: LatLng,
  label: string,
): ActionOption | null {
  // ⚠ A ROUTER THAT SAYS NO MUST NOT COST THE ELEMENT ITS MARCH.
  //
  // The first game played on the real raster showed why: blue marched and red
  // did not, because red's objective sat on a river and A* refuses to plan a
  // route that ends on impassable ground. Red silently fell back to blind
  // one-turn bounds — the exact behaviour routes exist to replace — and the
  // only visible symptom was that half the force had no line drawn on the map.
  //
  // So the planners are tried in order and the FIRST that answers wins, with
  // the greedy bearing search last because it can always answer. Which one
  // actually drew the route is recorded on it, so a map or a log can say
  // whether a line is a real route or a fallback.
  for (const planner of plannersFor(config)) {
    const waypoints = planner.plan(fe.position, goal, fe.moveType);
    if (!waypoints || waypoints.length === 0) continue;

    const route: PlannedRoute = {
      goal,
      label,
      waypoints,
      plannedOnTurn: state.turn,
      planner: planner.kind,
    };
    const option = marchOption(fe, config, route);
    if (option) return option;
  }
  return null;
}

/**
 * Where an element can actually get to this turn, heading for `target`.
 *
 * ⚠ THIS IS THE ONLY PLACE THE GROUND GETS A SAY IN MOVEMENT, and until it
 * existed it had none: lib/movement.ts held the allowance rule, ruleset.ts
 * carried a table per Move Type per terrain, and nothing called either. Every
 * destination in this file was a blind fraction of the straight line and the
 * loop teleported the element to it — across water, up a slope, through thick
 * woods, all at the same rate.
 *
 * Returns null when the move is not worth offering: blocked on the spot, or so
 * truncated that it costs an activation to go nowhere. A commander cannot then
 * choose it, which is the point — the rules say no HERE, not in the resolver,
 * because a rejected choice is a bug that looks like a bad decision.
 */
function moveBound(
  config: PhaseConfig,
  fe: ForceElement,
  /** Where the loop's own invented bound would put it, with no rule involved. */
  blindBound: LatLng,
  /**
   * Where the element is actually trying to get to, when there IS a rule to
   * say how far it gets. Defaults to the blind bound, for the moves whose
   * length a RULE already fixes — a Retreat's minimum, a Reserve Move's
   * 1,000 m, closing up to co-location. Only the two invented fractions (40%
   * of the way to a contact, 25% of the way to the objective) differ, and they
   * are exactly the stand-ins the allowance replaces.
   */
  fullAsk: LatLng = blindBound,
): { destination: LatLng; note: string } | null {
  if (!config.ruleset.modules.terrainMovement) {
    return { destination: blindBound, note: "" };
  }

  const plan = planMoveAround({
    terrain: config.terrain,
    moveType: fe.moveType,
    table: config.ruleset.movement,
    from: fe.position,
    to: fullAsk,
  });

  if (plan.distanceM < MIN_USEFUL_MOVE_M) return null;

  // A move that arrives says nothing extra. A move that does not arrive has to
  // say so in its summary, because a commander — heuristic, human or model —
  // chooses from summaries and would otherwise believe it had closed the gap.
  const arrived = plan.outcome === "completed" && plan.detouredDeg === undefined;
  return {
    destination: plan.destination,
    note: arrived ? "" : ` (${describeMovePlan(plan)})`,
  };
}

/**
 * Could this element meet something it has not seen?
 *
 * The press-on/halt choice is only a choice when contact is possible. Offering
 * both variants when there is nothing left to bump into would double the
 * option list for nothing, and decision breadth is measured.
 */
function contactPossible(state: GameState, fe: ForceElement): boolean {
  return forceElementsOf(state, opposing(fe.side)).some(
    (enemy) => enemy.combatStrength > 0 && sightingOf(state, fe.side, enemy.id) === "none",
  );
}

/**
 * A move option, and — where contact is possible — its pressing-on twin.
 *
 * 7.1.3's choice, pre-committed. The default is to halt, which is the cautious
 * reading; pressing on is the deliberate one, and it is the option a commander
 * takes when the objective matters more than the contact.
 */
function withContactChoice(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
  option: ActionOption,
): ActionOption[] {
  if (!config.ruleset.modules.contactHalt) return [option];
  if (!contactPossible(state, fe)) return [option];
  return [
    { ...option, onContact: "halt", summary: `${option.summary} — halt on contact` },
    {
      ...option,
      id: `${option.id}:press`,
      onContact: "press",
      summary: `${option.summary} — press on through contact`,
    },
  ];
}

/**
 * Has this element got rounds left for that capability?
 *
 * True whenever the ammunition module is off, so the rest of the loop does
 * not have to care which modules are on.
 */
function hasRounds(
  fe: ForceElement,
  kind: CapabilityClass | undefined,
  config: PhaseConfig,
): boolean {
  if (!config.ruleset.modules.ammunition) return true;
  if (!kind) return false;
  // Absent means "not yet tracked", which is full — an FE built before the
  // module was switched on must not start the game dry.
  const remaining = fe.ammo?.[kind];
  return remaining === undefined || remaining > 0;
}

/**
 * Spend one round of a capability.
 *
 * Initialises from the ruleset on first use rather than at force-build time,
 * so switching the module on mid-experiment does not require rebuilding every
 * force list — and so `moduleImpact` compares the same force under both arms.
 */
function spendRound(
  state: GameState,
  feId: string,
  kind: CapabilityClass,
  config: PhaseConfig,
): GameState {
  const fe = state.forceElements[feId];
  if (!fe) return state;

  const current = fe.ammo?.[kind] ?? config.ruleset.logistics.roundsPerCapability;
  return {
    ...state,
    forceElements: {
      ...state.forceElements,
      [feId]: { ...fe, ammo: { ...fe.ammo, [kind]: Math.max(0, current - 1) } },
    },
  };
}

/**
 * How many elements a side may activate this turn.
 *
 * Infinity when the module is off. With it on, command is the constraint
 * rather than the number of vehicles: a side with more sub-units than command
 * capacity has to choose which ones fight, which is a DECISION rather than a
 * die roll, and decisions are what a mechanic has to change to earn its keep.
 */
export function activationBudget(state: GameState, side: Side, config: PhaseConfig): number {
  if (!config.ruleset.modules.commandActivations) return Number.POSITIVE_INFINITY;

  const { activationsWithoutHq, activationsPerCommandRating } = config.ruleset.logistics;
  const commanded = forceElementsOf(state, side)
    .filter((fe) => fe.combatStrength > 0 && fe.morale === "good" && fe.commandRating != null)
    .reduce((sum, fe) => sum + (fe.commandRating ?? 0) * activationsPerCommandRating, 0);

  // No HQ is not no activations: a leaderless force still fights, badly.
  return Math.max(activationsWithoutHq, commanded);
}

/**
 * Legal actions for one force element.
 *
 * This is where the rules say no. A commander never sees an illegal option, so
 * a model cannot hallucinate one into being resolved — the failure mode
 * becomes "it chose badly", which is visible, rather than "it chose something
 * impossible", which is a bug that looks like cleverness.
 */
export function optionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): ActionOption[] {
  const options: ActionOption[] = [
    { id: `${fe.id}:hold`, kind: "hold", actorId: fe.id, summary: `${fe.label} holds` },
  ];

  if (fe.combatStrength <= 0) return [];

  if (config.ruleset.modules.closeCombat) {
    // 9.3.8: an element in a Melee is fixed in place. "The only Action they
    // can take while they have a MELEE marker is on subsequent Turns to
    // Retreat, DirF against the enemy FE that are currently in the Melee, or
    // continue with the Assault."
    if (hasMarker(fe, "melee") && stillLocked(state, fe, config)) {
      return meleeOptionsFor(state, fe, config, options);
    }

    // 7.1.2: "an FE may be required to have a REORG marker, meaning it cannot
    // take an Action that Turn except to DirF and Attempt Sighting."
    if (hasMarker(fe, "reorg")) return reorgOptionsFor(state, fe, config, options);
  }

  const enemies = forceElementsOf(state, opposing(fe.side)).filter((e) => e.combatStrength > 0);

  for (const enemy of enemies) {
    // Only what this side has actually sighted can be engaged.
    if (sightingOf(state, fe.side, enemy.id) === "none") continue;

    const rangeM = distanceM(fe.position, enemy.position);
    const capability = weaponFor(fe, enemy, rangeM);

    // A dummy has nothing to shoot with. It exists to be looked at, and
    // offering it a fire option would give the game away for free.
    const canShoot =
      !(config.ruleset.modules.dummies && fe.isDummy) &&
      // A broken sub-unit is still on the board but is not fighting.
      canEngage(fe.morale) &&
      hasRounds(fe, capability?.kind, config) &&
      // A RESERVE HOLDS ITS FIRE IN THE FIRST ROUND, and this is the one
      // place the rulebook has to be interpreted rather than quoted.
      //
      // 2.1.13 makes Reserve an ORDER given "instead of a specific Order",
      // and 9.0 requires the Action taken to "best allow the FE to fulfil its
      // Order". 7.2.1 then lets a Reserve Move happen "even if they have a
      // MOVED marker" but only with "no FIRED marker" — so the rules plainly
      // expect a reserve to be able to move in the first round, and plainly
      // deny it the reserve move if it shoots.
      //
      // Firing is therefore never compatible with the Reserve Order: it
      // forfeits the entire privilege the order exists to grant. Offering a
      // reserve a shot it should never take made the heuristic commander take
      // it every time, which is why the Counteraction Round fired ONCE in
      // twelve turns and could not have been measured.
      //
      // The trade this creates is the mechanic: hold a third of your
      // firepower out of the first round to have a mobile, unfired third in
      // the second.
      !(config.ruleset.modules.counteraction && hasMarker(fe, "reserve"));

    if (capability && canShoot && !hasMarker(fe, "fired")) {
      const visible = lineOfSight(config.terrain, {
        from: fe.position,
        to: enemy.position,
      }).visible;
      if (visible) {
        options.push({
          id: `${fe.id}:fire:${enemy.id}`,
          kind: "fire",
          actorId: fe.id,
          targetId: enemy.id,
          summary: `${fe.label} fires on ${enemy.label} at ${Math.round(rangeM)} m`,
        });

        // COMBINED FIRE (9.2.1). Everyone co-located with this element who
        // can also reach the same target, firing as one.
        const partners = combinedFirePartners(state, fe, enemy, config);
        if (partners.length > 0) {
          const all = [fe, ...partners];
          options.push({
            id: `${fe.id}:combinedFire:${enemy.id}`,
            kind: "fire",
            actorId: fe.id,
            actorIds: all.map((one) => one.id),
            targetId: enemy.id,
            summary:
              `${fe.label} and ${partners.length} other${partners.length > 1 ? "s" : ""} ` +
              `fire together on ${enemy.label} at ${Math.round(rangeM)} m`,
          });
        }
      }
    }

    // An assault sets FIRED too, so it forfeits the Reserve Move for the same
    // reason firing does. A reserve gets its Hasty Assault in the
    // Counteraction Round instead (7.2.1).
    const reserveHoldingBack = config.ruleset.modules.counteraction && hasMarker(fe, "reserve");

    if (
      rangeM <= ASSAULT_RANGE_M &&
      canAdvance(fe.morale) &&
      !hasMarker(fe, "melee") &&
      !reserveHoldingBack
    ) {
      options.push({
        id: `${fe.id}:assault:${enemy.id}`,
        kind: "assault",
        actorId: fe.id,
        targetId: enemy.id,
        summary: `${fe.label} assaults ${enemy.label}`,
      });

      // COMBINED ASSAULT (9.3). Same co-location test; going in together is
      // what makes an odds ratio something a commander can choose rather
      // than something the force list decided for them.
      const partners = combinedAssaultPartners(state, fe, enemy, config);
      if (partners.length > 0) {
        const all = [fe, ...partners];
        options.push({
          id: `${fe.id}:combinedAssault:${enemy.id}`,
          kind: "assault",
          actorId: fe.id,
          actorIds: all.map((one) => one.id),
          targetId: enemy.id,
          summary:
            `${fe.label} and ${partners.length} other${partners.length > 1 ? "s" : ""} ` +
            `assault ${enemy.label} together`,
        });
      }
    }

    // Movement is offered towards each known contact, one bound at a time.
    // Deliberately coarse: a full movement planner belongs with the router,
    // and the loop only needs somewhere legal to go.
    if (!hasMarker(fe, "moved") && canAdvance(fe.morale) && rangeM > ASSAULT_RANGE_M) {
      // 40% of the way was the stand-in for having no movement rule at all:
      // "a unit closes over several turns" was a guess at a bound, chosen
      // because nothing knew how far a bound was. With the allowance wired in,
      // something does — so the element asks to close the gap and the ground
      // decides how much of it it gets. The 40% survives only as the arm the
      // module is measured against.
      const step = 0.4;
      const bound = moveBound(
        config,
        fe,
        {
          lat: fe.position.lat + (enemy.position.lat - fe.position.lat) * step,
          lng: fe.position.lng + (enemy.position.lng - fe.position.lng) * step,
        },
        stepTowards(fe.position, enemy.position, rangeM - ASSAULT_RANGE_M),
      );
      if (bound) {
        options.push(
          ...withContactChoice(state, fe, config, {
            id: `${fe.id}:move:${enemy.id}`,
            kind: "move",
            actorId: fe.id,
            targetId: enemy.id,
            destination: bound.destination,
            summary: `${fe.label} advances on ${enemy.label}${bound.note}`,
          }),
        );
      }
    }
  }

  // Nothing to shoot at, but there are still orders. An advance towards the
  // objective, offered whenever this element has NO FIRE OPTION — not merely
  // when it has sighted nothing.
  //
  // The narrower test was "has sighted nothing", and on real ground that
  // stalled the game. A unit that has sighted an enemy through a gap but
  // cannot see it now — woods in the way, or a ridge — had no fire option and
  // no move option either, so it held. Mean game length went from 11 turns on
  // flat ground to 35 on wooded ground, most of them timing out with both
  // sides sitting in cover looking at trees. Units must be able to reposition
  // to regain observation.
  // Tried the broader test — "no fire option" rather than "nothing sighted" —
  // and it was worse, which is worth recording. A unit that has sighted an
  // enemy it cannot currently see is already offered a move TOWARDS that
  // enemy (the move option does not require line of sight, only sighting), so
  // the broader test merely added a competing pull towards the objective.
  // Units stopped converging and games got longer, not shorter: one cell went
  // to 40 turns with 27 of 60 drawn. The stall this fixes is specifically
  // having sighted NOTHING AT ALL.
  const sightedAnything = enemies.some(
    (enemy) => sightingOf(state, fe.side, enemy.id) !== "none",
  );
  const objective = state.objectives?.[fe.side];

  // Indirect fire, for anything that has a tube (9.2.2). Offered alongside
  // everything else; the mortar chooses like any other element.
  options.push(...indirectFireOptionsFor(state, fe, config));

  // CLOSING UP ON YOUR OWN SIDE, which nothing could do.
  //
  // ⚠ THE THIRD TIME IN ONE SITTING THAT A COMMANDER PROBLEM WAS A WIRING ONE.
  //
  // Combined Fire needs elements within 250 m of each other. The force lists
  // deploy them 447-599 m apart — roughly double — and every move option led
  // either towards an enemy or towards the objective. There was no move that
  // closed the distance to a FRIEND, so no commander could ever concentrate,
  // and a formation-keeping commander built without this changed almost
  // nothing: a probe put combined shots at 3% against 4%.
  //
  // The giveaway was that the commander took 87-100% of the chances it was
  // offered in both arms. It was never declining to mass; it was never in a
  // position to.
  //
  // Gated on `combinedFire` so the control arm stays exactly the game it was:
  // with no combined fire there is no mechanical reason to concentrate, and
  // an option that changed movement in every arm would contaminate every
  // other module's measurement.
  if (
    config.ruleset.modules.combinedFire &&
    !hasMarker(fe, "moved") &&
    canAdvance(fe.morale)
  ) {
    const friends = forceElementsOf(state, fe.side).filter(
      (other) => other.id !== fe.id && other.combatStrength > 0,
    );
    const nearestFriend = friends.reduce<{ fe: ForceElement; rangeM: number } | null>(
      (best, other) => {
        const rangeM = distanceM(fe.position, other.position);
        return best == null || rangeM < best.rangeM ? { fe: other, rangeM } : best;
      },
      null,
    );

    // Only when actually separated. An element already in the troop has
    // nothing to close up on.
    if (nearestFriend && nearestFriend.rangeM > config.ruleset.coLocatedM) {
      const bound = moveBound(
        config,
        fe,
        stepTowards(fe.position, nearestFriend.fe.position, nearestFriend.rangeM / 2),
      );
      if (bound) {
        options.push({
          id: `${fe.id}:regroup:${nearestFriend.fe.id}`,
          kind: "move",
          actorId: fe.id,
          destination: bound.destination,
          summary: `${fe.label} closes up on ${nearestFriend.fe.label}${bound.note}`,
        });
      }
    }
  }

  if (!sightedAnything && objective && !hasMarker(fe, "moved") && canAdvance(fe.morale)) {
    const rangeM = distanceM(fe.position, objective);
    // Close enough to be there already: no point offering the move.
    if (rangeM > ASSAULT_RANGE_M) {
      // A MARCH, not a bound, when the element is already committed to one or
      // when routes are enabled: the objective is usually several turns away,
      // and several turns away is precisely the case a one-turn bound cannot
      // plan for.
      if (config.ruleset.modules.routeMarch) {
        const march = fe.route
          ? marchOption(fe, config, fe.route)
          : planMarch(state, fe, config, objective, "the objective");
        if (march) {
          options.push(...withContactChoice(state, fe, config, march));
          return options;
        }
      }

      const step = 0.25;
      const bound = moveBound(
        config,
        fe,
        {
          lat: fe.position.lat + (objective.lat - fe.position.lat) * step,
          lng: fe.position.lng + (objective.lng - fe.position.lng) * step,
        },
        // The objective is a place to BE, so that is what it asks for. How far
        // it gets this turn is the allowance's business.
        objective,
      );
      if (bound) {
        options.push(
          ...withContactChoice(state, fe, config, {
            id: `${fe.id}:move:objective`,
            kind: "move",
            actorId: fe.id,
            destination: bound.destination,
            summary: `${fe.label} advances on the objective${bound.note}`,
          }),
        );
      }
    }
  }

  return options;
}

/** Is there still an enemy in this element's Melee? (9.3.8) */
function stillLocked(state: GameState, fe: ForceElement, config: PhaseConfig): boolean {
  return forceElementsOf(state, opposing(fe.side)).some(
    (enemy) =>
      enemy.combatStrength > 0 &&
      hasMarker(enemy, "melee") &&
      distanceM(fe.position, enemy.position) <= config.ruleset.assault.defenderRadiusM,
  );
}

/**
 * Take the MELEE marker off anything that is no longer in one (9.3.8).
 *
 * "This marker remains in place (including on subsequent Turns) until: 1. One
 * side is Eliminated. 2. One side declares a Retreat or is forced to do so."
 *
 * Both of those are states of the board rather than events to hook, so they
 * are swept for once a turn. Leaving a stale marker on would fix an element
 * in place against an enemy that no longer exists, and would keep charging it
 * the `attackerAlreadyInMelee` penalty for a fight that is over.
 */
export function endStaleMelees(state: GameState, config: PhaseConfig): GameState {
  if (!config.ruleset.modules.closeCombat) return state;

  const effects = Object.values(state.forceElements)
    .filter((fe) => hasMarker(fe, "melee"))
    .filter((fe) => fe.combatStrength <= 0 || !stillLocked(state, fe, config))
    .map((fe) => ({ kind: "marker" as const, feId: fe.id, marker: "melee", added: false }));

  return effects.length === 0 ? state : applyEffects(state, effects);
}

/**
 * What an element locked in a Melee may do (9.3.8).
 *
 * Retreat, shoot the people it is locked with, or press the assault. It may
 * NOT move away casually, reposition, or engage anything else on the board —
 * being in close combat is a commitment, and that is the cost of having gone
 * in.
 */
function meleeOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
  options: ActionOption[],
): ActionOption[] {
  const locked = forceElementsOf(state, opposing(fe.side)).filter(
    (enemy) =>
      enemy.combatStrength > 0 &&
      hasMarker(enemy, "melee") &&
      distanceM(fe.position, enemy.position) <= config.ruleset.assault.defenderRadiusM,
  );

  // Nobody left to be locked with: the melee is over in fact if not in
  // bookkeeping, so the element gets its ordinary options back next turn.
  if (locked.length === 0) return options;

  for (const enemy of locked) {
    if (canEngage(fe.morale) && !hasMarker(fe, "fired")) {
      options.push({
        id: `${fe.id}:fire:${enemy.id}`,
        kind: "fire",
        actorId: fe.id,
        targetId: enemy.id,
        summary: `${fe.label} fires on ${enemy.label} at point blank`,
      });
    }
    if (canAdvance(fe.morale) && !hasMarker(fe, "fired")) {
      options.push({
        id: `${fe.id}:assault:${enemy.id}`,
        kind: "assault",
        actorId: fe.id,
        targetId: enemy.id,
        summary: `${fe.label} presses the assault on ${enemy.label}`,
      });
    }
  }

  // A Retreat, declared rather than forced (9.3.8: "One side declares a
  // Retreat ... this is upon the FE/Group's Activation"). Modelled as a move,
  // because that is what it is; the extra morale step is charged by
  // resolveAction so that a declared retreat costs the same as a forced one.
  if (canAdvance(fe.morale)) {
    const away = retreatTo(fe, locked, config.ruleset.assault.retreatMinM);
    // Breaking contact is still movement over ground. An element backed onto
    // water it cannot cross has nowhere to go, and the absence of this option
    // is what says so — it fights on because it must.
    const bound = moveBound(config, fe, away);
    if (bound) {
      options.push({
        id: `${fe.id}:retreat`,
        kind: "move",
        actorId: fe.id,
        destination: bound.destination,
        retreat: true,
        summary: `${fe.label} breaks off and retreats${bound.note}`,
      });
    }
  }

  return options;
}

/** What a reorganising element may do (7.1.2): shoot, and nothing else. */
function reorgOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
  options: ActionOption[],
): ActionOption[] {
  if (!canEngage(fe.morale) || hasMarker(fe, "fired")) return options;

  for (const enemy of forceElementsOf(state, opposing(fe.side))) {
    if (enemy.combatStrength <= 0) continue;
    if (sightingOf(state, fe.side, enemy.id) === "none") continue;

    const rangeM = distanceM(fe.position, enemy.position);
    const capability = weaponFor(fe, enemy, rangeM);
    if (!capability || !hasRounds(fe, capability.kind, config)) continue;
    if (!lineOfSight(config.terrain, { from: fe.position, to: enemy.position }).visible) continue;

    options.push({
      id: `${fe.id}:fire:${enemy.id}`,
      kind: "fire",
      actorId: fe.id,
      targetId: enemy.id,
      summary: `${fe.label} fires on ${enemy.label} while reorganising`,
    });
  }

  return options;
}

/**
 * Who can join `lead` in firing on `enemy` (9.2.1 COMBINED).
 *
 * "Where multiple non-Mounted FEs within 250m of each other are Activated
 * simultaneously to DirF, targeting the same single enemy FE ... No
 * participating FE can already have a FIRED marker."
 *
 * Co-location is measured from the LEAD rather than between every pair. The
 * rulebook says "within 250m of each other", which for a real cluster of
 * counters is the same thing, and checking every pair would make the option
 * depend on which element the commander happened to be looking at.
 *
 * NOT MODELLED: the rule that FEs in different Groups need a Command
 * Activation to fire together — we have no Groups — and the rule that ATM
 * cannot combine, because ATM has no separate fire column here yet.
 */
function combinedFirePartners(
  state: GameState,
  lead: ForceElement,
  enemy: ForceElement,
  config: PhaseConfig,
): ForceElement[] {
  if (!config.ruleset.modules.combinedFire) return [];

  return forceElementsOf(state, lead.side).filter((other) => {
    if (other.id === lead.id) return false;
    if (other.combatStrength <= 0) return false;
    if (hasMarker(other, "fired") || hasMarker(other, "held")) return false;
    if (!canEngage(other.morale)) return false;
    if (config.ruleset.modules.dummies && other.isDummy) return false;
    // A reserve is holding its fire, and an element already locked in a melee
    // is not free to shoot at something across the field.
    if (config.ruleset.modules.counteraction && hasMarker(other, "reserve")) return false;
    if (config.ruleset.modules.closeCombat && hasMarker(other, "melee")) return false;
    if (distanceM(other.position, lead.position) > config.ruleset.coLocatedM) return false;

    // It has to be able to make the shot itself: sighted, in range, in sight.
    if (sightingOf(state, other.side, enemy.id) === "none") return false;
    const rangeM = distanceM(other.position, enemy.position);
    const capability = weaponFor(other, enemy, rangeM);
    if (!capability || !hasRounds(other, capability.kind, config)) return false;
    return lineOfSight(config.terrain, { from: other.position, to: enemy.position }).visible;
  });
}

/** Who can go in with `lead` (9.3 COMBINED). Same co-location test. */
function combinedAssaultPartners(
  state: GameState,
  lead: ForceElement,
  enemy: ForceElement,
  config: PhaseConfig,
): ForceElement[] {
  if (!config.ruleset.modules.combinedFire) return [];

  return forceElementsOf(state, lead.side).filter((other) => {
    if (other.id === lead.id) return false;
    if (other.combatStrength <= 0) return false;
    if (hasMarker(other, "fired") || hasMarker(other, "held")) return false;
    // 9.3.4: attackers must "not be Disrupted/Broken" and must not already
    // have fired.
    if (!canAdvance(other.morale)) return false;
    if (config.ruleset.modules.dummies && other.isDummy) return false;
    if (config.ruleset.modules.counteraction && hasMarker(other, "reserve")) return false;
    if (config.ruleset.modules.closeCombat && hasMarker(other, "melee")) return false;
    if (distanceM(other.position, lead.position) > config.ruleset.coLocatedM) return false;
    // Everyone going in has to be able to reach the objective.
    return distanceM(other.position, enemy.position) <= ASSAULT_RANGE_M;
  });
}

/** Sighting attempts for every pair that has a line. Runs before activations. */
export function runSighting(state: GameState, config: PhaseConfig, turn: number): GameState {
  let next = state;
  for (const viewer of ["blue", "red"] as const) {
    for (const observer of forceElementsOf(next, viewer)) {
      if (observer.combatStrength <= 0) continue;
      for (const target of forceElementsOf(next, opposing(viewer))) {
        if (target.combatStrength <= 0) continue;
        const hasLine = lineOfSight(config.terrain, {
          from: observer.position,
          to: target.position,
        }).visible;
        if (!hasLine) continue;

        const outcome = resolveSighting(
          observer,
          target,
          {
            // The wood the target is standing in. Nothing passed this before,
            // so targetInCover was a declared modifier that could not fire
            // however the ground was generated.
            targetInCover: inCover(config.terrain, target.position),
            observerIsRecce: observer.commandRating == null && observer.concealed,
            throughSmoke: smokeOnLine(state, observer.position, target.position, config.ruleset),
          },
          config.ruleset,
          config.rng,
          turn,
          "arcAction",
          viewer,
        );
        config.log.append(outcome.event);
        next = applyEffects(next, outcome.effects);
      }
    }
  }
  return next;
}

async function activate(
  state: GameState,
  side: Side,
  // The full config: this is the one phase that asks a commander to decide.
  config: GameConfig,
  turn: number,
  standing: StandingOrders,
): Promise<{ state: GameState; acted: boolean }> {
  // An element is finished for the turn once it has FIRED or HELD. Having
  // moved does not finish it, so it may advance and then engage — at the
  // firerMoved penalty, which is the whole point. Before this, a moved
  // element was done, so `firerMoved` was a declared DRM that no sequence of
  // play could ever reach.
  const available = forceElementsOf(state, side).filter(
    (fe) => fe.combatStrength > 0 && !hasMarker(fe, "fired") && !hasMarker(fe, "held"),
  );
  if (available.length === 0) return { state, acted: false };

  // Command capacity limits how many elements a side can COMMIT, not how many
  // actions a committed element takes. An element that has already been
  // activated may act again — move, then engage — without costing more
  // command; only bringing a NEW element into the fight does.
  //
  // Getting this wrong once made `firerMoved` unreachable: the budget was
  // checked before choosing, so once three elements had activated the side
  // stopped entirely, and an element that had moved could never come back to
  // fire. Across 80 games, no unit ever moved and then shot.
  const budget = activationBudget(state, side, config);
  const committed = forceElementsOf(state, side).filter((fe) => hasMarker(fe, "activated"));
  const eligible =
    committed.length >= budget ? available.filter((fe) => hasMarker(fe, "activated")) : available;
  if (eligible.length === 0) return { state, acted: false };

  const options = eligible.flatMap((fe) => optionsFor(state, fe, config));
  if (options.length === 0) return { state, acted: false };

  const view = projectForSide(state, side);
  const commander = config.commanders[side];
  const { optionId, rationale } = await commander.decide(
    view,
    options,
    "which force element acts, and how?",
  );
  const chosen = options.find((o) => o.id === optionId) ?? options[0];

  config.log.append({
    type: "decision",
    turn,
    phase: "arcAction",
    rulesetId: config.ruleset.id,
    side,
    actorId: chosen.actorId,
    question: "which force element acts, and how?",
    options,
    chosenId: chosen.id,
    chosenBy: commander.kind,
    rationale,
  });

  let next = state;

  // 7.1, in the order it lists them. First, ONE enemy element may try to make
  // out what is moving (10.0) — which can reveal it, and so can bring it
  // within reach of the Reactive Fire that follows.
  if (chosen.actorId) {
    next = await attemptSightingInterruptLive(next, chosen.actorId, side, config, turn);
  }

  // An assault has its own sequence (9.3.4): Surprise first, then Reactive
  // Fire from outside the objective and Defensive Fire from on it.
  if (chosen.kind === "assault" && chosen.actorId) {
    next = await resolveAssaultActionLive(next, chosen, config, turn, standing, "actionReaction");
    return { state: markActivated(next, chosen.actorId), acted: true };
  }

  // THE R, BEFORE THE MOVE IT INTERRUPTS (7.1.3). Moving across a loaded arc
  // offers the enemy a shot; if that shot Disrupts or Breaks the mover, the
  // move does not happen at all.
  const interruptible = chosen.kind === "move";
  if (interruptible && chosen.actorId) {
    next = await reactiveFireLive(next, chosen.actorId, side, config, turn, standing, "actionReaction");
  }

  const stopped = interruptible && chosen.actorId != null && !mayProceed(next, chosen.actorId);
  if (!stopped) next = await resolveMoveLive(next, chosen, config, turn);

  next = markActivated(next, chosen.actorId ?? "");
  if (stopped && chosen.actorId) {
    next = applyEffects(next, [
      { kind: "marker", feId: chosen.actorId, marker: "held", added: true },
    ]);
  }

  return { state: next, acted: true };
}

/**
 * Where in the sequence of play an action is being resolved.
 *
 * Defaults to the Action-Reaction Round, which is where all but the
 * Counteraction Round's actions happen. The Counteraction Round passes
 * `arcCounteraction` and `counteractionFire`, so a second-round shot is
 * logged under the round it was taken in and carries its penalty — see
 * CounteractionRule.
 */
export interface ResolveContext {
  phase?: "arcAction" | "arcCounteraction";
  counteractionFire?: boolean;
  /** The attacker went in unseen (9.3.1). Set by resolveAssaultAction. */
  surprise?: boolean;
  /**
   * Filled in by a move, if given: whether it was cut short by contact, and
   * by whom. Written to rather than returned so resolveAction keeps its
   * signature — this is how resolveMoveLive learns there is a decision to
   * make without walking the move twice.
   */
  report?: { halted?: boolean; contacts?: string[] };
}

/**
 * Mark an element as having used its activation.
 *
 * Command capacity counts elements committed, so this is what the budget
 * reads. Marked even when a move was stopped by Reactive Fire: the element
 * committed itself and spent its activation, which is exactly what being
 * caught in the open costs — and without it the loop would offer the same
 * element again and spin.
 */
/**
 * The elements taking part in an action.
 *
 * `actorIds` for a combined action, otherwise the lead alone. Anything in the
 * list that has died or gone since the option was generated is dropped rather
 * than resolved — an order given at the start of a turn may be partly
 * impossible by the time it executes, which is the cost of planning blind.
 */
function firersOf(
  state: GameState,
  option: ActionOption,
  lead: ForceElement,
): ForceElement[] {
  if (!option.actorIds || option.actorIds.length === 0) return [lead];

  const live = option.actorIds
    .map((id) => state.forceElements[id])
    .filter((fe): fe is ForceElement => fe != null && fe.combatStrength > 0);

  return live.length > 0 ? live : [lead];
}

function markActivated(state: GameState, feId: string): GameState {
  return applyEffects(state, [
    { kind: "marker", feId, marker: "activated", added: true },
  ]);
}

export function resolveAction(
  state: GameState,
  option: ActionOption,
  config: PhaseConfig,
  turn: number,
  context: ResolveContext = {},
): GameState {
  // Where the log stood before this action, so the resolutions it produces can
  // be handed to the step that produced them rather than matched up later.
  const logged = config.onStep ? config.log.all().length : 0;
  const next = resolveActionInner(state, option, config, turn, context);
  // Every action a commander takes is a step in the turn's playback, and this
  // is the ONE place every sequence of play passes through — recording here
  // rather than in each caller is what stops the two sequences producing
  // different timelines for the same game.
  if (next !== state) {
    const events = config.log
      .all()
      .slice(logged)
      .filter((event): event is ResolutionEvent => event.type === "resolution");
    recordStep(config, next, turn, context.phase ?? "arcAction", option.summary, {
      side: state.forceElements[option.actorId ?? ""]?.side,
      actorId: option.actorId,
      events,
    });
  }
  return next;
}

function resolveActionInner(
  state: GameState,
  option: ActionOption,
  config: PhaseConfig,
  turn: number,
  context: ResolveContext = {},
): GameState {
  const actor = option.actorId ? state.forceElements[option.actorId] : undefined;
  if (!actor) return state;
  const phase = context.phase ?? "arcAction";

  switch (option.kind) {
    case "fire": {
      const target = option.targetId ? state.forceElements[option.targetId] : undefined;
      if (!target) return state;

      // An indirect mission is a different procedure, not a modified shot:
      // no line of sight from the firer, an area effect, and for smoke no
      // roll at all.
      if (option.indirect) {
        return resolveIndirectFire(state, option, actor, config, turn, phase);
      }

      // One firer, or several firing together (9.2.1). resolveDirectFire has
      // always summed Combat Strength across the array and applied the worst
      // modifier among them; until combinedFire existed nothing ever handed
      // it more than one.
      const firers = firersOf(state, option, actor);
      const rangeM = distanceM(actor.position, target.position);
      const capability = weaponFor(actor, target, rangeM);
      const outcome = resolveDirectFire(
        firers,
        target,
        {
          rangeM,
          maxRangeM: capability?.maxRangeM,
          // What this particular weapon can defeat. Undefined is unknown and
          // fails open — see PenetrationRule.
          penetrationMm: capability?.penetrationMm,
          munition: capability?.munition,
          topAttack: capability?.topAttack,
          // Same wood, same fact: hard to see is also hard to hit.
          targetInCover: inCover(config.terrain, target.position),
          // Aspect: a shot arriving outside the target's frontal arc.
          flank: isFlankShot(firers, target, config.ruleset),
          // "any FE firing from, through or into Smoke suffers a -2 DRM".
          smoke: smokeOnLine(state, actor.position, target.position, config.ruleset),
          counteractionFire: context.counteractionFire,
        },
        config.ruleset,
        config.rng,
        turn,
        phase,
      );
      config.log.append(outcome.event);
      // 9.2.1.2 step 7: "Add a FIRED marker to the Firing FE(s)" — plural,
      // and step 6 reduces the Ammo of all of them.
      let fired = applyEffects(applyEffects(state, outcome.effects), [
        ...firers.map((one) => ({
          kind: "marker" as const,
          feId: one.id,
          marker: "fired",
          added: true,
        })),
        ...firers
          .filter((one) => one.concealed && config.ruleset.modules.concealment)
          .map((one) => ({ kind: "concealed" as const, feId: one.id, to: false })),
        // 2.1.15: "Concealment is removed ... if the FE Fires or Assaults."
        // Nothing flipped a counter before, so concealment was a starting
        // condition that survived a whole game of shooting.
        //
        // ⚠ GATED ON THE MODULE, and it matters for measurement rather than
        // for play. `projectForSide` hides a concealed element from the enemy
        // whatever the module says, so flipping counters with `concealment`
        // OFF changed what every OTHER module's arm could see: the first
        // sweep after this was added showed partialSighting falling from 85%
        // to 23% for no reason connected to partial sighting. A module that
        // half-operates while switched off corrupts every comparison run
        // against it.
        ...(actor.concealed && config.ruleset.modules.concealment
          ? [{ kind: "concealed" as const, feId: actor.id, to: false }]
          : []),
      ]);

      if (config.ruleset.modules.ammunition) {
        // Every firer spends a round, not just the lead. Charging one round
        // for a combined shot would make massing free as well as effective.
        for (const one of firers) {
          const theirRange = distanceM(one.position, target.position);
          const theirs = weaponFor(one, target, theirRange);
          if (theirs) fired = spendRound(fired, one.id, theirs.kind, config);
        }
      }

      // Firing at a dummy reveals it. It is removed from play rather than
      // destroyed: nothing was ever there, and leaving it standing would let
      // one decoy absorb an entire battle.
      if (config.ruleset.modules.dummies && target.isDummy) {
        fired = applyEffects(fired, [{ kind: "eliminated", feId: target.id }]);
      }

      return fired;
    }

    case "assault": {
      const target = option.targetId ? state.forceElements[option.targetId] : undefined;
      if (!target) return state;

      // 9.3: the assault draws in every enemy within 250 m of the location,
      // not just the one that was aimed at. Attacking into a mutually
      // supporting position should cost more than attacking an isolated one.
      const defenders = forceElementsOf(state, target.side).filter(
        (fe) =>
          fe.combatStrength > 0 &&
          distanceM(fe.position, target.position) <= config.ruleset.assault.defenderRadiusM,
      );

      // One attacker, or several going in together (9.3 COMBINED), whose
      // Combat Strength is summed by resolveAssault into the odds ratio.
      const attackers = firersOf(state, option, actor);

      const outcome = resolveAssault(
        attackers,
        defenders.length > 0 ? defenders : [target],
        {
          defenderInCover: inCover(config.terrain, target.position),
          surprise: context.surprise,
          // 9.3.4 step 4: "If only Vehicle FEs are present for the defender,
          // without Mounted FEs: the Vehicles automatically withdraw."
          //
          // We apply it as the column shift the assault table already
          // declares rather than as an automatic withdrawal, which is this
          // file's standing simplification — the odds ladder IS the shifts.
          // A heavy shift towards the attacker and an automatic withdrawal
          // are not the same rule, and the difference is worth revisiting
          // once anybody plays a scenario where it decides something.
          //
          // Unreachable until today: it needs a defender who is not a
          // vehicle for the absence of one to mean anything, and every force
          // list was pure armour.
          defenderIsVehicleOnly: defenders.every((one) => one.targetClass !== "foot"),
        },
        config.ruleset,
        config.rng,
        turn,
        phase,
      );
      config.log.append(outcome.event);
      // An assault is an engagement: it finishes the element's turn. It also
      // flips the counter (2.1.15) — you cannot charge someone unseen.
      return applyEffects(applyEffects(state, outcome.effects), [
        ...attackers.flatMap((one) => [
          { kind: "marker" as const, feId: one.id, marker: "melee", added: true },
          { kind: "marker" as const, feId: one.id, marker: "fired", added: true },
        ]),
        ...attackers
          .filter((one) => one.concealed && config.ruleset.modules.concealment)
          .map((one) => ({ kind: "concealed" as const, feId: one.id, to: false })),
      ]);
    }

    case "move": {
      if (!option.destination) return state;

      // THE MOVE IS WALKED, NOT TELEPORTED. It stops the moment this side sees
      // something it had not seen — see rules/contact.ts for why that is the
      // difference between a meeting engagement and two columns ignoring each
      // other at 300 m.
      // 7.1.3: the mover may press on through contact — "unless it has become
      // Disrupted or Broken", which is what canAdvance reads. A shaken element
      // has no choice to make and goes to ground where it stands.
      const mayPressOn = option.onContact === "press" && canAdvance(actor.morale);
      const walk = config.ruleset.modules.contactHalt
        ? walkUntilContact(state, actor, option.destination, {
            terrain: config.terrain,
            ruleset: config.ruleset,
            rng: config.rng,
            turn,
            phase,
            haltOnContact: !mayPressOn,
          })
        : null;
      for (const event of walk?.events ?? []) config.log.append(event);

      const arrival = walk?.halted ? walk.end : option.destination;
      const halted = walk?.halted ?? false;
      if (context.report) {
        context.report.halted = halted;
        context.report.contacts = walk?.contacts ?? [];
      }

      return applyEffects(state, [
        ...(walk?.sighted ?? []),
        {
          kind: "position",
          feId: actor.id,
          lat: arrival.lat,
          lng: arrival.lng,
        },
        // You face where you are going. This is the only thing that sets
        // facing, which is what makes aspect a cost of manoeuvring rather
        // than a property of the force list.
        {
          kind: "facing",
          feId: actor.id,
          to: bearingDeg(actor.position, option.destination),
        },
        { kind: "marker", feId: actor.id, marker: "moved", added: true },
        // The march as it stands after this turn. A move with no route on it
        // CLEARS any march the element had: choosing a bound instead of the
        // march is a change of plan, and leaving the old one in place would
        // have it silently resume next turn.
        {
          kind: "route" as const,
          feId: actor.id,
          // Halting on contact ABANDONS THE MARCH. The plan was made in the
          // absence of the enemy, the enemy is now here, and next turn the
          // commander should be choosing rather than resuming.
          route:
            !halted && option.route && option.route.waypoints.length > 0
              ? option.route
              : null,
        },
        // A declared Retreat out of a Melee (9.3.6, 9.3.8). It breaks the
        // lock, and it costs the extra step of morale that a forced retreat
        // costs — otherwise declaring one would be strictly better than
        // being made to, and nobody would ever fight on.
        ...(option.retreat
          ? [
              { kind: "marker" as const, feId: actor.id, marker: "melee", added: false },
              {
                kind: "morale" as const,
                feId: actor.id,
                to: degradeMorale(actor.morale, 1),
              },
            ]
          : []),
      ]);
    }

    case "hold":
    default:
      // "held", NOT "moved". Marking a stationary element as moved handed the
      // enemy the targetMoved bonus for standing still, which is backwards.
      return applyEffects(state, [
        { kind: "marker", feId: actor.id, marker: "held", added: true },
      ]);
  }
}

// ── ARC ────────────────────────────────────────────────────────────────────
// The R and the C, shared by BOTH sequences of play.
//
// ⚠ THIS LIVES HERE RATHER THAN IN A MODULE OF ITS OWN, ON PURPOSE.
//
// rules/orders.ts imports turnLoop.ts. If ARC lived in rules/arc.ts it would
// have to import `optionsFor` and `resolveAction` from here, and runTurn would
// have to import ARC from there — a cycle. Putting it beside the other shared
// phase functions, typed against PhaseConfig, is what lets both sequences call
// exactly the same rules.
//
// THE COST OF NOT DOING THIS WAS THE POINT OF FINISHING ARC. Reaction lived
// only in the orders sequence, and the module sweep measures the OTHER one —
// so `moduleImpact("reactionFire")` compared two identical games and would
// have reported "CEREMONY — delete" for a mechanic that demonstrably decides
// engagements. A mechanic that only one sequence of play can reach is not
// finished, however well it works where it is.

/**
 * An element's rules of engagement for the turn.
 *
 * Our stand-in for the rulebook's Order Verbs plus an assigned TAI (7.1.3) —
 * see ReactionRule for why the substitution is legitimate and what would
 * replace it.
 */
export interface StandingOrder {
  actorId: string;
  engage: StandingEngagement;
  /** Optional tighter limit than the engagement rule implies, in metres. */
  withinM?: number;
}

/** Rules of engagement indexed for lookup during a round. */
export type StandingOrders = Record<Side, Map<string, StandingOrder>>;

/** Nobody declared anything: every element falls back to `defaultEngage`. */
export function noStandingOrders(): StandingOrders {
  return { blue: new Map(), red: new Map() };
}

/**
 * Would this element answer that action?
 *
 * `wasFiredUpon` is the distinction between a sentry and an ambusher: an
 * element on "ifFiredUpon" stays quiet while armour drives past and answers
 * only once someone shoots at its own side.
 */
export function willReact(
  order: StandingOrder | undefined,
  ruleset: RuleSet,
  context: { rangeM: number; shortRangeM: number; wasFiredUpon: boolean },
): boolean {
  const engage = order?.engage ?? ruleset.reaction.defaultEngage;
  if (order?.withinM != null && context.rangeM > order.withinM) return false;

  switch (engage) {
    case "never":
      return false;
    case "ifFiredUpon":
      return context.wasFiredUpon;
    case "withinShortRange":
      return context.rangeM <= context.shortRangeM;
    case "always":
      return true;
    default:
      return false;
  }
}

/**
 * Can this capability class hurt that kind of target? (9.2.1)
 *
 * ⚠ `targetClass` WAS READ BY NOTHING, AND A COMMENT SAID OTHERWISE.
 *
 * The rulebook is blunt about it: "Apers Capabilities may only be used
 * against Foot FEs and soft-skinned Wheeled vehicles. Atk Capabilities may be
 * used against any vehicle FE, but not Foot FEs."
 *
 * `TargetClass` has existed on every Force Element since the beginning, is
 * parsed out of the L6 profiles, and is displayed in the Asset Explorer. No
 * rule consulted it. Worse, PenetrationRule's comment asserted that
 * "`targetClass` gated WHETHER you could engage" — so the one place a reader
 * would look to check said the rule was implemented. That is the fifth
 * unwired mechanic found in this codebase and the first that argued back.
 *
 * It mattered little while every force list was pure armour, which is
 * presumably how it survived. It decides everything the moment infantry
 * exists: a tank's coaxial machine gun cannot kill a tank, and its main gun
 * is not what you use on dug-in infantry.
 */
export function capabilityCanEngage(kind: CapabilityClass, target: TargetClass): boolean {
  switch (kind) {
    case "apers":
      return target === "foot" || target === "soft_skin";
    case "atk":
    case "atm":
      return target !== "foot";
    // Smoke is not a weapon. An element whose only capability is smoke cannot
    // engage, which is correct rather than a bug.
    case "smoke":
      return false;
    // Neither of these is modelled as a ground engagement, and neither is
    // excluded either. FAILING OPEN, deliberately and for the same reason
    // PenetrationRule does: a unit whose only listed capability is `aa` would
    // otherwise be silently unable to fire at anything for its whole life,
    // which is the kind of quiet disarmament that takes a week to notice.
    case "aa":
    case "air_delivered":
      return true;
    default:
      return true;
  }
}

/**
 * The capability an element would use against THIS target at this range.
 *
 * Takes the target rather than just a distance, because which weapon you
 * reach for depends on what you are shooting at as much as how far away it
 * is. Capabilities are listed best-first in the profiles, so the first one
 * that both reaches and applies is the right one.
 */
export function weaponFor(fe: ForceElement, target: ForceElement, rangeM: number) {
  return fe.capabilities.find(
    (capability) =>
      rangeM <= capability.maxRangeM && capabilityCanEngage(capability.kind, target.targetClass),
  );
}

/**
 * Is this shot arriving outside the target's frontal arc?
 *
 * A target with no facing has not moved, and an element that has not moved is
 * assumed to be oriented on its arc — so it cannot be flanked by someone who
 * walked around a stationary tank that was watching them the whole time.
 * Aspect is something you give away by manoeuvring.
 *
 * For COMBINED fire every firer must have the flank, because 9.2.1.2 says to
 * "use the most detrimental modifiers to the Firing side": one element in
 * defilade should not launder the whole troop's aspect.
 */
export function isFlankShot(
  firers: readonly ForceElement[],
  target: ForceElement,
  ruleset: RuleSet,
): boolean {
  if (target.facing == null || firers.length === 0) return false;

  return firers.every((firer) => {
    const fromTargetToFirer = bearingDeg(target.position, firer.position);
    return bearingDeltaDeg(target.facing!, fromTargetToFirer) > ruleset.frontArcDeg;
  });
}

// ── Smoke (9.2.2.4) ────────────────────────────────────────────────────────

/** Is this point inside a cloud? */
export function inSmoke(state: GameState, point: LatLng, ruleset: RuleSet): boolean {
  if (!ruleset.modules.indirectFire) return false;
  return (state.smoke ?? []).some(
    (cloud) => distanceM(cloud.position, point) <= ruleset.indirectFire.smokeRadiusM,
  );
}

/**
 * Does this shot start in, end in, or pass through smoke?
 *
 * "any FE firing from, through or into Smoke suffers a -2 DRM" — all three,
 * which is why this checks the endpoints AND samples the line between them.
 * Sampling rather than solving the circle intersection because the cloud is a
 * 250 m disc on a 10 km board and a 100 m step cannot miss one.
 */
export function smokeOnLine(
  state: GameState,
  from: LatLng,
  to: LatLng,
  ruleset: RuleSet,
): boolean {
  if (!ruleset.modules.indirectFire) return false;
  const clouds = state.smoke ?? [];
  if (clouds.length === 0) return false;

  const total = distanceM(from, to);
  const steps = Math.max(1, Math.ceil(total / 100));
  for (let step = 0; step <= steps; step += 1) {
    const fraction = step / steps;
    const point = {
      lat: from.lat + (to.lat - from.lat) * fraction,
      lng: from.lng + (to.lng - from.lng) * fraction,
    };
    if (inSmoke(state, point, ruleset)) return true;
  }
  return false;
}

/** Which round of the ARC sub-phase is being played. */
export type ArcRound = "actionReaction" | "counteraction";

/**
 * ATTEMPT SIGHTING, as an interrupt (10.0, and the second item in 7.1's list
 * of what the non-activating side may do while an enemy activates).
 *
 * "Any one non-Activating FE can make an Attempt Sighting Action, regardless
 * of if it has already made one this Turn, to attempt to Sight an enemy
 * Activating FE in its LoS."
 *
 * THREE THINGS THAT SENTENCE SETTLES, and all three were wrong before:
 *
 *   - It happens WHEN SOMETHING ACTIVATES, not once at the top of the turn.
 *     `runSighting` sweeps every pair before anyone moves, so a Concealed
 *     element that broke cover mid-turn was no easier to see than one that
 *     sat still. Movement is the thing that gives a position away.
 *   - It is ONE observer. Not the whole side — "any one non-Activating FE".
 *     We take the closest with a line, which is the one that would in
 *     practice be watching.
 *   - It is NOT an Action, so it costs the observer nothing and it does not
 *     matter whether it has already looked this turn.
 *
 * A full sighting flips the counter (2.1.15), which is what makes the
 * activating element targetable — including by the Reactive Fire that runs
 * immediately after this. Being seen is how you come to be shot at.
 */
/**
 * Who could make the Attempt Sighting interrupt, nearest first.
 *
 * Empty when there is nothing to attempt: the module is off, or the actor is
 * not Concealed. Pure — split out so a TacticalDecider can be offered the
 * same list the rule picks from.
 */
export function sightingObservers(
  state: GameState,
  activatingFeId: string,
  activatingSide: Side,
  config: PhaseConfig,
): ForceElement[] {
  if (!config.ruleset.modules.concealment) return [];

  const actor = state.forceElements[activatingFeId];
  // Only a Concealed FE can be the subject: there is nothing to reveal about
  // something already on the map.
  if (!actor || actor.combatStrength <= 0 || !actor.concealed) return [];

  return forceElementsOf(state, opposing(activatingSide))
    .filter((fe) => fe.combatStrength > 0)
    .filter((fe) => lineOfSight(config.terrain, { from: fe.position, to: actor.position }).visible)
    .sort((a, b) => distanceM(a.position, actor.position) - distanceM(b.position, actor.position));
}

export function attemptSightingInterrupt(
  state: GameState,
  activatingFeId: string,
  activatingSide: Side,
  config: PhaseConfig,
  turn: number,
  /**
   * The observer a TacticalDecider picked. Absent means the nearest, which is
   * the rule's reading of "any one". Ignored if it is not an eligible one.
   */
  observerId?: string,
): GameState {
  const observers = sightingObservers(state, activatingFeId, activatingSide, config);
  const actor = state.forceElements[activatingFeId];
  const watcher = opposing(activatingSide);

  const observer =
    (observerId ? observers.find((fe) => fe.id === observerId) : undefined) ?? observers[0];
  if (!observer || !actor) return state;

  const outcome = resolveSighting(
    observer,
    actor,
    {
      targetInCover: inCover(config.terrain, actor.position),
      observerIsRecce: observer.commandRating == null && observer.concealed,
      throughSmoke: smokeOnLine(state, observer.position, actor.position, config.ruleset),
    },
    config.ruleset,
    config.rng,
    turn,
    state.phase === "arcCounteraction" ? "arcCounteraction" : "arcAction",
    watcher,
  );
  config.log.append(outcome.event);

  let next = applyEffects(state, outcome.effects);

  // 2.1.15: "Concealment is removed (the counter is flipped) if ... 2. The FE
  // is revealed through an Attempt Sighting." A partial contact is not a
  // reveal — you know something is there, not what.
  if (outcome.event.result === "full") {
    next = applyEffects(next, [{ kind: "concealed", feId: actor.id, to: false }]);
  }

  return next;
}

/** An element the rules allow to answer an action, and what its ROE says. */
interface EligibleReactor {
  reactor: ForceElement;
  rangeM: number;
  capability: NonNullable<ReturnType<typeof weaponFor>>;
  engage: StandingEngagement;
  ruleSaysReact: boolean;
}

/**
 * Everything that COULD answer this action, in the order the rules take them.
 *
 * Split out of runReactiveFire so that the ability to fire (range, line of
 * sight, rounds, markers, fog of war) is decided in one place by the rules,
 * and the WILLINGNESS to fire can be decided either by the declared ROE or by
 * a TacticalDecider at the moment. Pure: no dice, no log.
 */
function eligibleReactors(
  state: GameState,
  actorId: string,
  actingSide: Side,
  config: PhaseConfig,
  standing: StandingOrders,
  round: ArcRound,
  actorFiredAtId?: string,
  excluded?: ReadonlySet<string>,
): EligibleReactor[] {
  const actor = state.forceElements[actorId];
  if (!actor || actor.combatStrength <= 0) return [];

  const defender = opposing(actingSide);

  // Sighting is held per SIDE, so this is one check rather than a filter. A
  // reaction is still subject to fog of war: you cannot shoot at something
  // your side has not seen.
  if (sightingOf(state, defender, actor.id) === "none") return [];

  const eligible: EligibleReactor[] = [];
  const reactors = forceElementsOf(state, defender)
    .filter((fe) => fe.combatStrength > 0)
    .filter((fe) => canEngage(fe.morale))
    // A dummy has nothing to shoot with, and reacting would reveal it for free.
    .filter((fe) => !(config.ruleset.modules.dummies && fe.isDummy))
    .filter((fe) => !excluded?.has(fe.id))
    .filter((fe) => !hasMarker(fe, "reacted"))
    .filter((fe) => {
      // 7.1.3, and the two rounds have DIFFERENT eligibility:
      //   Action-Reaction Round: "the reacting FE has not yet Activated, or
      //                           has been given a Hold Action"
      //   Counteraction Round:   "it did not Fire in the Action-Reaction
      //                           round (i.e. it does not have a FIRED marker)"
      if (round === "counteraction") return !hasMarker(fe, "fired");
      return !hasMarker(fe, "activated") || hasMarker(fe, "held");
    });

  for (const fe of reactors) {
    const rangeM = distanceM(fe.position, actor.position);
    const capability = weaponFor(fe, actor, rangeM);
    if (!capability) continue;
    if (!hasRounds(fe, capability.kind, config)) continue;
    if (!lineOfSight(config.terrain, { from: fe.position, to: actor.position }).visible) {
      continue;
    }
    // HOLDING IS OVERWATCH, and the rulebook says so twice: 7.1.3 names
    // HOLD among the eleven Order Verbs that permit Reactive Fire, and
    // names "has been given a Hold Action" as the other way to qualify in
    // the Action-Reaction Round. An element that has chosen to do nothing
    // else is watching its whole arc, not just short range.
    //
    // Without this the activation sequence produced ZERO reactions in every
    // force list with `reactionFire` on and `counteraction` off — the
    // per-activation Commander has no step in which to declare rules of
    // engagement, so everything fell back to `withinShortRange`, and by the
    // time anything was inside short range it was shooting rather than
    // moving. The sweep would have called the R ceremony for want of an
    // orders phase, which is the exact failure this whole exercise exists
    // to prevent.
    const declared = standing[defender].get(fe.id);
    const order =
      declared ?? (hasMarker(fe, "held") ? { actorId: fe.id, engage: "always" as const } : undefined);

    eligible.push({
      reactor: fe,
      rangeM,
      capability,
      engage: order?.engage ?? config.ruleset.reaction.defaultEngage,
      ruleSaysReact: willReact(order, config.ruleset, {
        rangeM,
        shortRangeM: capability.shortRangeM,
        wasFiredUpon: actorFiredAtId != null,
      }),
    });
  }

  return eligible;
}

/**
 * REACTIVE FIRE (7.1.3). The R.
 *
 * ⚠ CALLED BEFORE THE MOVE, NOT AFTER IT, AND THAT IS THE RULE.
 *
 * "The moving FE halts while the DirF is resolved. After it is resolved the
 * moving FE may continue its movement (unless it has become Disrupted or
 * Broken), or it may elect to stop moving at that point."
 *
 * So a reaction can STOP an advance, which is the entire tactical point of
 * overwatch and was missing while reactions resolved after the mover had
 * already arrived. The caller resolves the reaction first and then asks
 * `mayProceed` whether the move still happens.
 *
 * Only a MOVE or an ASSAULT can be interrupted. An element that stands still
 * and shoots is not offering anyone the chance to catch it in the open, and
 * letting fire draw reactions turned every exchange into a brawl in which
 * initiative was worthless.
 */
export function runReactiveFire(
  state: GameState,
  actorId: string,
  actingSide: Side,
  config: PhaseConfig,
  turn: number,
  standing: StandingOrders,
  round: ArcRound,
  /** The element the actor is engaging, if any — for "ifFiredUpon". */
  actorFiredAtId?: string,
  /**
   * Elements that may NOT react because they are in the assault themselves.
   *
   * 9.3.2: "FEs of the defending side that are not part of the Assault (i.e.
   * not within 250m of the Assault location) may Reactive Fire". Those inside
   * the radius get Defensive Fire instead, and must not get both.
   */
  excluded?: ReadonlySet<string>,
  /**
   * Reactors chosen at the moment by a TacticalDecider, best first.
   *
   * Absent means the declared rules of engagement decide, as they always
   * did. Present, it REPLACES `willReact` — but only among elements the rules
   * say are able to fire; an id that is not eligible is ignored.
   */
  chosenReactorIds?: readonly string[],
): GameState {
  if (!config.ruleset.modules.reactionFire) return state;

  let next = state;
  const actor = next.forceElements[actorId];
  if (!actor || actor.combatStrength <= 0) return next;

  const candidates = eligibleReactors(
    next,
    actorId,
    actingSide,
    config,
    standing,
    round,
    actorFiredAtId,
    excluded,
  );

  const reactors = (
    chosenReactorIds
      ? chosenReactorIds
          .map((id) => candidates.find((candidate) => candidate.reactor.id === id))
          .filter((candidate): candidate is EligibleReactor => candidate != null)
      : candidates.filter((candidate) => candidate.ruleSaysReact)
  )
    .map((candidate) => candidate.reactor)
    .slice(0, config.ruleset.reaction.maxReactorsPerAction);

  for (const reactor of reactors) {
    const live = next.forceElements[reactor.id];
    const stillThere = next.forceElements[actorId];
    if (!live || live.combatStrength <= 0) continue;
    // Nothing left to shoot at: the element ahead of this one finished it.
    if (!stillThere || stillThere.combatStrength <= 0) break;

    const rangeM = distanceM(live.position, stillThere.position);
    const capability = weaponFor(live, stillThere, rangeM);

    const outcome = resolveDirectFire(
      [live],
      stillThere,
      {
        rangeM,
        maxRangeM: capability?.maxRangeM,
        penetrationMm: capability?.penetrationMm,
          munition: capability?.munition,
          topAttack: capability?.topAttack,
        targetInCover: inCover(config.terrain, stillThere.position),
        // An element caught moving across an arc is very often showing its
        // side, which is most of why overwatch is worth setting.
        flank: isFlankShot([live], stillThere, config.ruleset),
        smoke: smokeOnLine(next, live.position, stillThere.position, config.ruleset),
        // Carried as a named modifier so it shows up in the log beside every
        // other DRM rather than being folded invisibly into the roll.
        snapShot: true,
      },
      config.ruleset,
      config.rng,
      turn,
      "arcReaction",
    );
    config.log.append(outcome.event);

    next = applyEffects(applyEffects(next, outcome.effects), [
      // FIRED is the rulebook's cost: "cannot take any further Action for the
      // remainder of the Turn". Everything downstream already gates on it.
      { kind: "marker", feId: live.id, marker: "fired", added: true },
      { kind: "marker", feId: live.id, marker: "reacted", added: true },
    ]);

    if (config.ruleset.modules.ammunition && capability) {
      next = spendRound(next, live.id, capability.kind, config);
    }
  }

  return next;
}

// ── Indirect fire (9.2.2) ──────────────────────────────────────────────────

/** The IDF capability an element has, if any. */
function mortarOf(fe: ForceElement) {
  return fe.capabilities.find((capability) => capability.kind === "idf");
}

/**
 * Indirect fire options for a mortar (9.2.2, 9.2.2.1).
 *
 * ⚠ THE RULE THAT MAKES A MORTAR A MORTAR: it does not need to see.
 *
 * "An Activated Mortar FE can IDF at a target in Range without LoS, provided
 * a friendly FE has LoS to the target." So the line-of-sight test is run from
 * every OTHER element on the side, not from the firer. That is the whole
 * point of the weapon and it is why indirect fire cannot reuse the direct
 * fire option builder.
 *
 * It can also engage a target that is only Partially Sighted, which direct
 * fire cannot — "Can be used against Concealed enemy FEs that have been
 * 'Partially Sighted'" — at a penalty.
 *
 * IDF is Action-Reaction Round only (9.2.2), so these options are never
 * offered in the Counteraction Round; that is enforced by the round's own
 * option builders, which do not call this.
 */
export function indirectFireOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): ActionOption[] {
  if (!config.ruleset.modules.indirectFire) return [];
  if (fe.combatStrength <= 0 || !canEngage(fe.morale)) return [];
  if (hasMarker(fe, "fired")) return [];

  const mortar = mortarOf(fe);
  if (!mortar || !hasRounds(fe, "idf", config)) return [];

  const options: ActionOption[] = [];
  const friends = forceElementsOf(state, fe.side).filter((one) => one.combatStrength > 0);

  for (const enemy of forceElementsOf(state, opposing(fe.side))) {
    if (enemy.combatStrength <= 0) continue;

    // Partial contacts count for indirect fire. A full sighting is not needed
    // because somebody else is doing the looking.
    const seen = sightingOf(state, fe.side, enemy.id);
    if (seen === "none") continue;

    const rangeM = distanceM(fe.position, enemy.position);
    if (rangeM > mortar.maxRangeM) continue;

    // SOMEBODY has to be able to see it. Not necessarily the mortar.
    const observed = friends.some(
      (friend) =>
        lineOfSight(config.terrain, { from: friend.position, to: enemy.position }).visible,
    );
    if (!observed) continue;

    options.push({
      id: `${fe.id}:idf:${enemy.id}`,
      kind: "fire",
      actorId: fe.id,
      targetId: enemy.id,
      indirect: true,
      summary:
        `${fe.label} fires indirect on ${enemy.label} at ${Math.round(rangeM)} m` +
        (seen === "full" ? "" : " (partial contact)"),
    });

    // SMOKE (9.2.2.4). Laid on the ground near the target rather than on it:
    // the point is to blind the people who would otherwise shoot at you, and
    // a marker centred on the enemy does that.
    if (fe.capabilities.some((capability) => capability.kind === "smoke")) {
      options.push({
        id: `${fe.id}:smoke:${enemy.id}`,
        kind: "fire",
        actorId: fe.id,
        targetId: enemy.id,
        indirect: true,
        smoke: true,
        summary: `${fe.label} lays smoke on ${enemy.label}`,
      });
    }
  }

  return options;
}

/**
 * Resolve an indirect fire mission.
 *
 * Smoke needs no roll — "No roll is required" — it simply arrives. High
 * explosive is resolved as fire, and then spreads: everything within 250 m of
 * a hit may lose a step of morale, FRIENDLY OR ENEMY. That last clause is the
 * first rule in this game that can hurt your own side, and it is the reason a
 * commander should think twice before dropping a mission onto a melee.
 */
function resolveIndirectFire(
  state: GameState,
  option: ActionOption,
  actor: ForceElement,
  config: PhaseConfig,
  turn: number,
  phase: "arcAction" | "arcCounteraction",
): GameState {
  const target = option.targetId ? state.forceElements[option.targetId] : undefined;
  if (!target) return state;

  const spend = (from: GameState) =>
    config.ruleset.modules.ammunition ? spendRound(from, actor.id, "idf", config) : from;

  if (option.smoke) {
    const id = `smoke-${actor.id}-${turn}`;
    config.log.append({
      type: "resolution",
      turn,
      phase,
      kind: "directFire",
      rulesetId: config.ruleset.id,
      actorIds: [actor.id],
      targetIds: [target.id],
      // No roll: 9.2.2.4 says "No roll is required" for smoke.
      roll: { dice: [], total: 0, cursor: config.rng.cursor },
      modifiers: [],
      total: 0,
      table: "idf:smoke",
      result: "smoke",
      effects: [],
      narrative: `${actor.label} put smoke down on ${target.label}.`,
    });

    return spend(
      applyEffects(state, [
        { kind: "smoke", id, lat: target.position.lat, lng: target.position.lng, turn },
        { kind: "marker", feId: actor.id, marker: "fired", added: true },
      ]),
    );
  }

  const rangeM = distanceM(actor.position, target.position);
  const partial = sightingOf(state, actor.side, target.id) !== "full";

  const outcome = resolveDirectFire(
    [actor],
    target,
    {
      rangeM,
      maxRangeM: mortarOf(actor)?.maxRangeM,
      targetInCover: inCover(config.terrain, target.position),
      // Firing at something you have only been told about.
      partialContact: partial,
    },
    config.ruleset,
    config.rng,
    turn,
    phase,
  );
  config.log.append(outcome.event);

  let next = applyEffects(applyEffects(state, outcome.effects), [
    { kind: "marker", feId: actor.id, marker: "fired", added: true },
  ]);

  // AREA EFFECT, on a hit only. "If a Hit is achieved on the targeted FE,
  // other FEs (Friendly or Enemy) within 250m of it may take a level of
  // Morale Status loss."
  const hit = outcome.effects.some((effect) => effect.kind === "combatStrength");
  if (hit) {
    const splash = Object.values(next.forceElements)
      .filter((one) => one.id !== target.id && one.combatStrength > 0)
      .filter(
        (one) =>
          distanceM(one.position, target.position) <= config.ruleset.indirectFire.areaEffectM,
      );

    next = applyEffects(
      next,
      splash.map((one) => ({
        kind: "morale" as const,
        feId: one.id,
        to: degradeMorale(one.morale, 1),
      })),
    );
  }

  return spend(next);
}

/**
 * AN ASSAULT, IN THE ORDER 9.3.4 SETS OUT. Steps 1-3, then the odds.
 *
 * "Assaults in BGWS are an involved process ... It is very important to
 * follow the steps below precisely!" — and the order is the mechanic, not
 * ceremony. An assault used to be a bare odds comparison: the defender never
 * got to shoot at the people walking towards it, so closing cost nothing and
 * massing was always correct.
 *
 *   step 2  Roll for Surprise. 1D6, 4-6 succeeds.
 *   step 3  If NO Surprise:
 *             - defenders OUTSIDE the 250 m radius may Reactive Fire (7.1.3)
 *             - defenders INSIDE it may Defensive Fire, individually, at -2
 *           If Surprise: neither. "enemy FEs may not Reactive Fire or
 *           Defensive Fire" (9.3.1).
 *   then    the assault resolves, with a column shift for Surprise
 *
 * The attacker can be stopped on the way in, which is the point. Defensive
 * Fire that Disrupts or Breaks it means no assault happens at all.
 */
export function resolveAssaultAction(
  state: GameState,
  option: ActionOption,
  config: PhaseConfig,
  turn: number,
  standing: StandingOrders,
  round: ArcRound,
  phase: "arcAction" | "arcCounteraction" = "arcAction",
  /** Reactors chosen at the moment by a TacticalDecider. See runReactiveFire. */
  chosenReactorIds?: readonly string[],
): GameState {
  const attackerId = option.actorId;
  const targetId = option.targetId;
  if (!attackerId || !targetId) return state;

  let next = state;
  const attacker = next.forceElements[attackerId];
  const target = next.forceElements[targetId];
  if (!attacker || !target) return next;

  // 9.3: "Any enemy FE/Group within 250m of the location participates in the
  // Assault as the 'defender'." An assault draws in the neighbours; it is not
  // a duel, and that is what makes a dispersed defence expensive to attack.
  const radius = config.ruleset.assault.defenderRadiusM;
  const defenders = forceElementsOf(next, target.side).filter(
    (fe) => fe.combatStrength > 0 && distanceM(fe.position, target.position) <= radius,
  );
  const defenderIds = new Set(defenders.map((fe) => fe.id));

  // ── step 2: Surprise ────────────────────────────────────────────────────
  let surprise = false;
  if (config.ruleset.modules.defensiveFire) {
    const roll = config.rng.d6();
    // 9.2.2.4: "if Smoke is present, an assaulting FE/Group gets a +2 DRM for
    // achieving Surprise." The one place smoke helps the side that laid it
    // rather than merely blinding everybody.
    const throughSmoke = smokeOnLine(next, attacker.position, target.position, config.ruleset);
    const smokeBonus = throughSmoke ? config.ruleset.indirectFire.smokeSurpriseDrm : 0;
    surprise = roll.total + smokeBonus >= config.ruleset.assault.surpriseAt;
    config.log.append({
      type: "resolution",
      turn,
      phase,
      kind: "assault",
      rulesetId: config.ruleset.id,
      actorIds: [attackerId],
      targetIds: [...defenderIds],
      roll,
      // The rulebook modifies this roll from its ASSAULT SURPRISE TABLE, which
      // is on a Player Aid we do not have. Unmodified is a stated gap.
      modifiers: [],
      total: roll.total,
      table: "assault:surprise",
      result: surprise ? "surprise" : "noSurprise",
      effects: [],
      narrative: surprise
        ? `${attacker.label} went in unseen (${roll.total}).`
        : `${attacker.label} was seen coming (${roll.total}).`,
    });
  }

  // ── step 3: the fire the assault goes in through ────────────────────────
  if (!surprise) {
    // Everyone outside the radius, by the ordinary Reactive Fire rules.
    next = runReactiveFire(
      next,
      attackerId,
      attacker.side,
      config,
      turn,
      standing,
      round,
      targetId,
      config.ruleset.modules.defensiveFire ? defenderIds : undefined,
      chosenReactorIds,
    );

    // Everyone inside it, defending. "Combined Fire is not possible" — so
    // each defender is resolved as an individual DirF, at -2.
    if (config.ruleset.modules.defensiveFire) {
      for (const defender of defenders) {
        const live = next.forceElements[defender.id];
        const incoming = next.forceElements[attackerId];
        if (!live || live.combatStrength <= 0 || !canEngage(live.morale)) continue;
        if (!incoming || incoming.combatStrength <= 0) break;
        if (config.ruleset.modules.dummies && live.isDummy) continue;

        const rangeM = distanceM(live.position, incoming.position);
        const capability = weaponFor(live, incoming, rangeM);
        if (!capability || !hasRounds(live, capability.kind, config)) continue;

        const outcome = resolveDirectFire(
          [live],
          incoming,
          {
            rangeM,
            maxRangeM: capability.maxRangeM,
            penetrationMm: capability.penetrationMm,
            munition: capability.munition,
            topAttack: capability.topAttack,
            targetInCover: inCover(config.terrain, incoming.position),
            flank: isFlankShot([live], incoming, config.ruleset),
            smoke: smokeOnLine(next, live.position, incoming.position, config.ruleset),
            defensiveFire: true,
          },
          config.ruleset,
          config.rng,
          turn,
          phase,
        );
        config.log.append(outcome.event);
        next = applyEffects(next, outcome.effects);
        next = applyEffects(next, [
          { kind: "marker", feId: live.id, marker: "fired", added: true },
          // 2.1.15: firing flips the counter. Gated for the same measurement
          // reason as the one in resolveAction's fire case.
          ...(live.concealed && config.ruleset.modules.concealment
            ? [{ kind: "concealed" as const, feId: live.id, to: false }]
            : []),
        ]);
        if (config.ruleset.modules.ammunition) {
          next = spendRound(next, live.id, capability.kind, config);
        }
      }
    }
  }

  // Stopped on the way in. The attacker has spent its activation and is
  // pinned where it started, which is what defensive fire is for.
  if (!mayProceed(next, attackerId)) {
    return applyEffects(next, [
      { kind: "marker", feId: attackerId, marker: "held", added: true },
    ]);
  }

  return resolveAction(next, option, config, turn, { phase, surprise });
}

// ── Live decisions ─────────────────────────────────────────────────────────
// The async doors into the three functions above, for the moments a
// TacticalDecider may be asked about. See rules/tactical.ts.
//
// ⚠ EVERY ONE OF THESE IS A PASS-THROUGH WHEN NO DECIDER IS CONFIGURED. Same
// function, same arguments, same dice in the same order — so every existing
// test, calibration and fixed-seed replay still describes the game that is
// played when `config.tactical` is absent.

/** How many times one move may stop for contact and be asked about it. */
const MAX_CONTACT_DECISIONS = 3;

/** Put a decider's reasoning in the log, in sequence with what it caused. */
function logTraces(
  config: PhaseConfig,
  turn: number,
  phase: Phase,
  side: Side,
  traces: readonly TacticalTrace[],
  /** The board at the moment of deciding, for the turn's playback. */
  state?: GameState,
): void {
  for (const trace of traces) {
    // A decision is a moment in the turn in its own right — "R1 held fire"
    // leaves nothing on the board, so without a step the playback would show
    // a tank crossing an arc and nothing else happening, which reads as a bug.
    if (state) {
      const p = trace.probabilities?.[trace.chosenId];
      recordStep(
        config,
        state,
        turn,
        phase,
        `${trace.actorId ?? side}: ${trace.question} \u2192 ${trace.chosenId}` +
          (trace.fallback
            ? ` (rules; Jev ${trace.fallback})`
            : p != null
              ? ` (Jev ${Math.round(p * 100)}%)`
              : " (Jev)"),
        { side, actorId: trace.actorId },
      );
    }
    config.log.append({
      type: "decision",
      turn,
      phase,
      rulesetId: config.ruleset.id,
      side,
      actorId: trace.actorId,
      question: trace.question,
      options: trace.options,
      chosenId: trace.chosenId,
      chosenBy: trace.chosenBy,
      rationale: trace.rationale,
      probabilities: trace.probabilities,
      confidence: trace.confidence,
      latencyMs: trace.latencyMs,
      costUsd: trace.costUsd,
      fallback: trace.fallback,
    });
  }
}

function toCandidates(eligible: readonly EligibleReactor[]): ReactionCandidate[] {
  return eligible.map((entry) => ({
    reactorId: entry.reactor.id,
    rangeM: Math.round(entry.rangeM),
    capability: entry.capability.kind,
    engage: entry.engage,
    ruleSaysReact: entry.ruleSaysReact,
  }));
}

/** Commander intent per side, where a sequence of play has one to give. */
export type SideIntents = Partial<Record<Side, CommanderIntent>>;

/**
 * Reactive Fire, with the reacting side asked at the moment.
 *
 * The rules decide who CAN fire; the decider decides who DOES. See
 * runReactiveFire for everything else.
 */
export async function reactiveFireLive(
  state: GameState,
  actorId: string,
  actingSide: Side,
  config: PhaseConfig,
  turn: number,
  standing: StandingOrders,
  round: ArcRound,
  actorFiredAtId?: string,
  excluded?: ReadonlySet<string>,
  intents?: SideIntents,
): Promise<GameState> {
  const side = opposing(actingSide);
  const decider = config.tactical?.[side];
  const plain = () =>
    runReactiveFire(state, actorId, actingSide, config, turn, standing, round, actorFiredAtId, excluded);
  if (!decider || !config.ruleset.modules.reactionFire) return plain();

  const eligible = eligibleReactors(
    state,
    actorId,
    actingSide,
    config,
    standing,
    round,
    actorFiredAtId,
    excluded,
  );
  if (eligible.length === 0) return plain();

  const verdict = await decider.decideReactions({
    state,
    config,
    turn,
    round,
    side,
    actorId,
    wasFiredUpon: actorFiredAtId != null,
    candidates: toCandidates(eligible),
    maxReactors: config.ruleset.reaction.maxReactorsPerAction,
    intent: intents?.[side],
  });
  logTraces(config, turn, "arcReaction", side, verdict.traces, state);

  return runReactiveFire(
    state,
    actorId,
    actingSide,
    config,
    turn,
    standing,
    round,
    actorFiredAtId,
    excluded,
    verdict.reactorIds,
  );
}

/**
 * An assault, with the defending side's Reactive Fire asked at the moment.
 *
 * Asked BEFORE the Surprise roll, because the roll and the answer are
 * independent and asking after would mean splitting resolveAssaultAction in
 * two. The cost is a question that goes unused when the attacker achieves
 * surprise, which is logged like any other — it was a decision, it simply
 * turned out not to matter.
 */
export async function resolveAssaultActionLive(
  state: GameState,
  option: ActionOption,
  config: PhaseConfig,
  turn: number,
  standing: StandingOrders,
  round: ArcRound,
  phase: "arcAction" | "arcCounteraction" = "arcAction",
  intents?: SideIntents,
): Promise<GameState> {
  const plain = () =>
    resolveAssaultAction(state, option, config, turn, standing, round, phase);
  const attacker = option.actorId ? state.forceElements[option.actorId] : undefined;
  const target = option.targetId ? state.forceElements[option.targetId] : undefined;
  if (!attacker || !target || !config.ruleset.modules.reactionFire) return plain();

  const decider = config.tactical?.[target.side];
  if (!decider) return plain();

  const radius = config.ruleset.assault.defenderRadiusM;
  const defenderIds = new Set(
    forceElementsOf(state, target.side)
      .filter((fe) => fe.combatStrength > 0 && distanceM(fe.position, target.position) <= radius)
      .map((fe) => fe.id),
  );
  const eligible = eligibleReactors(
    state,
    attacker.id,
    attacker.side,
    config,
    standing,
    round,
    target.id,
    config.ruleset.modules.defensiveFire ? defenderIds : undefined,
  );
  if (eligible.length === 0) return plain();

  const verdict = await decider.decideReactions({
    state,
    config,
    turn,
    round,
    side: target.side,
    actorId: attacker.id,
    wasFiredUpon: true,
    candidates: toCandidates(eligible),
    maxReactors: config.ruleset.reaction.maxReactorsPerAction,
    intent: intents?.[target.side],
  });
  logTraces(config, turn, "arcReaction", target.side, verdict.traces, state);

  return resolveAssaultAction(
    state,
    option,
    config,
    turn,
    standing,
    round,
    phase,
    verdict.reactorIds,
  );
}

/**
 * A move, with the mover asked what to do the moment it makes contact.
 *
 * The rulebook gives the moving player that choice at that moment (7.1.3);
 * the engine used to take it in advance, on the option, because a move
 * resolved synchronously. Here the move is walked with halt-on-contact, and
 * if it halts the side's decider is asked whether to press on. If it does,
 * the rest of the move is walked from where it stopped — and may stop again
 * for someone else, up to MAX_CONTACT_DECISIONS times.
 *
 * ⚠ THE ONE PLACE A DECIDER CHANGES THE DICE. A continued walk re-attempts
 * sighting against enemies the first leg looked for and missed, which the
 * single pre-committed walk would not have. It is only reachable with a
 * decider configured.
 */
export async function resolveMoveLive(
  state: GameState,
  option: ActionOption,
  config: PhaseConfig,
  turn: number,
  context: ResolveContext = {},
  intents?: SideIntents,
): Promise<GameState> {
  const actor = option.actorId ? state.forceElements[option.actorId] : undefined;
  const decider = actor ? config.tactical?.[actor.side] : undefined;
  if (
    !actor ||
    !decider ||
    option.kind !== "move" ||
    !option.destination ||
    !config.ruleset.modules.contactHalt
  ) {
    return resolveAction(state, option, config, turn, context);
  }

  const destination = option.destination;
  const preferred = option.onContact ?? "halt";
  let next = state;
  let leg: ActionOption = { ...option, onContact: "halt" };

  for (let asked = 0; asked <= MAX_CONTACT_DECISIONS; asked += 1) {
    const report: NonNullable<ResolveContext["report"]> = {};
    next = resolveAction(next, leg, config, turn, { ...context, report });
    if (!report.halted || asked === MAX_CONTACT_DECISIONS) return next;

    // A shaken element has no choice to make and goes to ground (7.1.3).
    const moved = next.forceElements[actor.id];
    if (!moved || moved.combatStrength <= 0 || !canAdvance(moved.morale)) return next;
    const remainingM = distanceM(moved.position, destination);
    if (remainingM < MIN_USEFUL_MOVE_M) return next;

    const verdict = await decider.decideContact({
      state: next,
      config,
      turn,
      side: actor.side,
      option,
      actorId: actor.id,
      newContacts: report.contacts ?? [],
      remainingM,
      preferred,
      intent: intents?.[actor.side],
    });
    logTraces(config, turn, context.phase ?? "arcAction", actor.side, verdict.traces, next);
    if (!verdict.press) return next;

    leg = {
      ...option,
      id: `${option.id}:press`,
      summary: `${option.summary} (pressed on through contact)`,
      onContact: "halt",
    };
  }

  return next;
}

/**
 * The Attempt Sighting interrupt, with the watching side choosing who looks.
 *
 * "Any one non-Activating FE" — the rule reads that as the nearest with a
 * line. A decider may pick a better one: a recce element further off, say,
 * or one in cover rather than one in the open. Only asked when there is more
 * than one candidate, since one is not a choice.
 */
export async function attemptSightingInterruptLive(
  state: GameState,
  activatingFeId: string,
  activatingSide: Side,
  config: PhaseConfig,
  turn: number,
  intents?: SideIntents,
): Promise<GameState> {
  const side = opposing(activatingSide);
  const decider = config.tactical?.[side];
  const plain = () => attemptSightingInterrupt(state, activatingFeId, activatingSide, config, turn);
  if (!decider?.chooseObserver) return plain();

  const observers = sightingObservers(state, activatingFeId, activatingSide, config);
  const actor = state.forceElements[activatingFeId];
  if (observers.length < 2 || !actor) return plain();

  const verdict = await decider.chooseObserver({
    state,
    config,
    turn,
    side,
    actorId: activatingFeId,
    observers: observers.map((fe) => ({
      observerId: fe.id,
      rangeM: Math.round(distanceM(fe.position, actor.position)),
      recce: fe.commandRating == null && fe.concealed,
    })),
    intent: intents?.[side],
  });
  logTraces(config, turn, state.phase, side, verdict.traces, state);

  return attemptSightingInterrupt(
    state,
    activatingFeId,
    activatingSide,
    config,
    turn,
    verdict.observerId ?? undefined,
  );
}

/**
 * Put a choice the engine would otherwise make by heuristic to the side's
 * decider.
 *
 * Returns `undefined` when there is no decider to ask (or it gave an answer
 * that is not on offer), so the caller keeps its own default; `null` when
 * the decider declined everything and passing is allowed.
 */
export async function chooseOptionLive(
  state: GameState,
  side: Side,
  options: ActionOption[],
  question: string,
  config: PhaseConfig,
  turn: number,
  allowPass: boolean,
  intents?: SideIntents,
): Promise<ActionOption | null | undefined> {
  const decider = config.tactical?.[side];
  if (!decider?.chooseOption || options.length === 0) return undefined;

  const verdict = await decider.chooseOption({
    state,
    config,
    turn,
    side,
    question,
    options,
    allowPass,
    intent: intents?.[side],
  });
  logTraces(config, turn, state.phase, side, verdict.traces, state);

  if (verdict.optionId === undefined) return undefined;
  if (verdict.optionId === null) return allowPass ? null : undefined;
  return options.find((option) => option.id === verdict.optionId);
}

/**
 * May an element still carry out the move it was interrupted during?
 *
 * "unless it has become Disrupted or Broken" — `canAdvance` is exactly that
 * test, and an eliminated element obviously goes nowhere.
 */
export function mayProceed(state: GameState, actorId: string): boolean {
  const fe = state.forceElements[actorId];
  return fe != null && fe.combatStrength > 0 && canAdvance(fe.morale);
}

/**
 * RESERVES (2.1.13). Nominated in the Command Sub-phase, one turn at a time.
 *
 * "Only one-third of FE/Groups in a side may be given a Reserve Order." A cap,
 * not a quota — and the cap is what makes it a decision rather than a setting.
 *
 * `preferred` is how a commander nominates; when it is absent (the harness's
 * per-activation Commander has no orders step in which to say) the rearmost
 * third are taken, because that is what a reserve physically is. A default
 * that nominated nothing would leave the Counteraction Round's first stage
 * permanently empty and make the module look inert for a reason that has
 * nothing to do with the rule.
 */
export function nominateReserves(
  state: GameState,
  side: Side,
  config: PhaseConfig,
  preferred?: readonly string[],
): GameState {
  if (!config.ruleset.modules.counteraction) return state;

  const own = forceElementsOf(state, side).filter((fe) => fe.combatStrength > 0);
  const limit = Math.floor(own.length * config.ruleset.counteraction.reserveFraction);
  if (limit <= 0) return state;

  let chosen: ForceElement[];
  if (preferred && preferred.length > 0) {
    const byId = new Map(own.map((fe) => [fe.id, fe]));
    chosen = preferred
      .map((id) => byId.get(id))
      .filter((fe): fe is ForceElement => fe != null)
      .slice(0, limit);
  } else {
    // Rearmost third: furthest from the enemy's centre of mass.
    const enemies = forceElementsOf(state, opposing(side)).filter((fe) => fe.combatStrength > 0);
    if (enemies.length === 0) return state;
    const centre = {
      lat: enemies.reduce((sum, fe) => sum + fe.position.lat, 0) / enemies.length,
      lng: enemies.reduce((sum, fe) => sum + fe.position.lng, 0) / enemies.length,
    };
    chosen = [...own]
      .sort((a, b) => distanceM(b.position, centre) - distanceM(a.position, centre))
      .slice(0, limit);
  }

  return applyEffects(
    state,
    chosen.map((fe) => ({
      kind: "marker" as const,
      feId: fe.id,
      marker: "reserve",
      added: true,
    })),
  );
}

/** How many elements a side may still nominate. Offered to a planning commander. */
export function reserveLimitFor(state: GameState, side: Side, config: PhaseConfig): number {
  if (!config.ruleset.modules.counteraction) return 0;
  const own = forceElementsOf(state, side).filter((fe) => fe.combatStrength > 0);
  return Math.floor(own.length * config.ruleset.counteraction.reserveFraction);
}

/** A point `metres` along the line from `from` towards `to`, never overshooting. */
function stepTowards(
  from: { lat: number; lng: number },
  to: { lat: number; lng: number },
  metres: number,
): { lat: number; lng: number } {
  const total = distanceM(from, to);
  if (total <= 0) return { ...from };
  const fraction = Math.min(1, metres / total);
  return {
    lat: from.lat + (to.lat - from.lat) * fraction,
    lng: from.lng + (to.lng - from.lng) * fraction,
  };
}

/**
 * RESERVE MOVEMENT options (7.2.1).
 *
 * "These FEs can move up to 1,000m towards a priority ... They can do this
 * even if they have a MOVED marker." Hence no `moved` check — the extra move
 * is the whole privilege of being in reserve.
 */
export function reserveMoveOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): ActionOption[] {
  if (!hasMarker(fe, "reserve")) return [];
  if (hasMarker(fe, "fired") || hasMarker(fe, "reserveMoved")) return [];
  if (fe.combatStrength <= 0 || !canAdvance(fe.morale)) return [];

  const distance = config.ruleset.counteraction.reserveMoveM;
  const options: ActionOption[] = [];

  for (const enemy of forceElementsOf(state, opposing(fe.side))) {
    if (enemy.combatStrength <= 0) continue;
    if (sightingOf(state, fe.side, enemy.id) === "none") continue;
    const rangeM = distanceM(fe.position, enemy.position);
    if (rangeM <= ASSAULT_RANGE_M) continue;
    // The 1,000 m is the RULE's ceiling; the ground can still lower it. The
    // allowance spent here is the full turn's, not a second one, because the
    // reserve move is the privilege of having held back — it is extra
    // permission to move, not extra fuel.
    const bound = moveBound(
      config,
      fe,
      stepTowards(fe.position, enemy.position, Math.min(distance, rangeM - ASSAULT_RANGE_M)),
    );
    if (!bound) continue;
    options.push({
      id: `${fe.id}:reserveMove:${enemy.id}`,
      kind: "move",
      actorId: fe.id,
      targetId: enemy.id,
      destination: bound.destination,
      summary: `${fe.label} moves up from reserve towards ${enemy.label}${bound.note}`,
    });
  }

  // "towards a priority, as set out on the Sync Matrix" — the objective is the
  // only standing priority we model, and it is what lets a reserve be
  // committed before anything has been sighted.
  const objective = state.objectives?.[fe.side];
  if (objective && distanceM(fe.position, objective) > ASSAULT_RANGE_M) {
    const bound = moveBound(config, fe, stepTowards(fe.position, objective, distance));
    if (bound) {
      options.push({
        id: `${fe.id}:reserveMove:objective`,
        kind: "move",
        actorId: fe.id,
        destination: bound.destination,
        summary: `${fe.label} moves up from reserve towards the objective${bound.note}`,
      });
    }
  }

  return options;
}

/**
 * COUNTERACTION FIRE options (7.2.2).
 *
 * "Any FE/Group that does not have a FIRED marker (i.e. that did not Fire in
 * the Action-Reaction Round) can DirF in the Counteraction Round."
 *
 * Note what is NOT required: the element need not be unactivated. Something
 * that moved in the first round and held its fire may shoot now — which is
 * precisely the manoeuvre the round exists to permit.
 */
export function counteractionFireOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): ActionOption[] {
  if (fe.combatStrength <= 0) return [];
  if (hasMarker(fe, "fired") || hasMarker(fe, "counteracted")) return [];
  if (!canEngage(fe.morale)) return [];
  if (config.ruleset.modules.dummies && fe.isDummy) return [];

  const options: ActionOption[] = [];
  for (const enemy of forceElementsOf(state, opposing(fe.side))) {
    if (enemy.combatStrength <= 0) continue;
    if (sightingOf(state, fe.side, enemy.id) === "none") continue;

    const rangeM = distanceM(fe.position, enemy.position);
    const capability = weaponFor(fe, enemy, rangeM);
    if (!capability || !hasRounds(fe, capability.kind, config)) continue;
    if (!lineOfSight(config.terrain, { from: fe.position, to: enemy.position }).visible) continue;

    options.push({
      id: `${fe.id}:counterFire:${enemy.id}`,
      kind: "fire",
      actorId: fe.id,
      targetId: enemy.id,
      summary: `${fe.label} fires on ${enemy.label} at ${Math.round(rangeM)} m (counteraction)`,
    });
  }
  return options;
}

/** Fire or assault available to a reserve at the end of its move (7.2.1). */
export function reserveFollowUpOptionsFor(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): ActionOption[] {
  const options = counteractionFireOptionsFor(state, fe, config);

  // "or Hasty Assault". Same co-location test as a deliberate assault.
  if (canAdvance(fe.morale) && !hasMarker(fe, "melee")) {
    for (const enemy of forceElementsOf(state, opposing(fe.side))) {
      if (enemy.combatStrength <= 0) continue;
      if (sightingOf(state, fe.side, enemy.id) === "none") continue;
      if (distanceM(fe.position, enemy.position) > ASSAULT_RANGE_M) continue;
      options.push({
        id: `${fe.id}:hastyAssault:${enemy.id}`,
        kind: "assault",
        actorId: fe.id,
        targetId: enemy.id,
        summary: `${fe.label} hasty assaults ${enemy.label}`,
      });
    }
  }

  return options;
}

/**
 * Who decides, in the Counteraction Round.
 *
 * A callback rather than a Commander, because the two sequences of play get
 * their decisions from different places — one asks a per-activation commander,
 * the other reads a plan the commander already gave. Returning null is a Pass.
 */
export type CounteractionChooser = (
  /**
   * The board as it stands AT THIS MOMENT IN THE ROUND.
   *
   * Passed rather than captured because the round mutates its own state as it
   * goes: a chooser that closed over the state at the start of the round would
   * be scoring options against positions two reserve moves out of date, and
   * would do it silently.
   */
  state: GameState,
  side: Side,
  options: ActionOption[],
  stage: "reserveMove" | "reserveFollowUp" | "counteractionFire",
) => Promise<ActionOption | null>;

/**
 * THE COUNTERACTION ROUND (7.2). The C.
 *
 * Two stages, alternating, Initiative side first in both — "The Initiative
 * side gets to Activate first in both the Action-Reaction Round and the
 * Counteraction Round" (6.0).
 *
 *   1. Reserve Movement, and a DirF or Hasty Assault at the end of it.
 *   2. Counteraction Fire by anything that has not fired.
 *
 * PASSING IS FINAL, which is the rule that makes the round a game rather than
 * a mopping-up loop: "A side that has passed cannot at a later point, after
 * seeing the other side's DirFs, then declare that it wishes to DirF."
 */
export async function runCounteractionRound(
  state: GameState,
  config: PhaseConfig,
  turn: number,
  initiative: Side,
  standing: StandingOrders,
  decide: CounteractionChooser,
  /** Each side's plan, for its TacticalDecider. Absent in the activation sequence. */
  intents?: SideIntents,
): Promise<GameState> {
  if (!config.ruleset.modules.counteraction) return state;

  let next: GameState = { ...state, phase: "arcCounteraction" };
  const order: Side[] = initiative === "blue" ? ["blue", "red"] : ["red", "blue"];

  // ── Stage 1: Reserve Movement ───────────────────────────────────────────
  const movePassed: Record<Side, boolean> = { blue: false, red: false };
  let guard = 0;
  while ((!movePassed.blue || !movePassed.red) && guard < 100) {
    guard += 1;
    for (const side of order) {
      if (movePassed[side]) continue;

      const options = forceElementsOf(next, side).flatMap((fe) =>
        reserveMoveOptionsFor(next, fe, config),
      );
      if (options.length === 0) {
        movePassed[side] = true;
        continue;
      }

      const chosen = await decide(next, side, options, "reserveMove");
      if (!chosen || !chosen.actorId || !chosen.destination) {
        movePassed[side] = true;
        continue;
      }

      config.log.append({
        type: "decision",
        turn,
        phase: "arcCounteraction",
        rulesetId: config.ruleset.id,
        side,
        actorId: chosen.actorId,
        question: "reserve movement",
        options,
        chosenId: chosen.id,
        chosenBy: "heuristic",
      });

      // Same two interrupts as any other activation: one enemy element may
      // try to make out what is moving up, and then anything unfired may
      // shoot at it. "A Reserve Move can be subject to Reactive Fire by any
      // enemy FE/Group in Range and LoS that does not have a FIRED marker."
      next = await attemptSightingInterruptLive(next, chosen.actorId, side, config, turn, intents);
      next = await reactiveFireLive(
        next,
        chosen.actorId,
        side,
        config,
        turn,
        standing,
        "counteraction",
        undefined,
        undefined,
        intents,
      );

      // Mark it regardless: an element shot to a standstill has still used its
      // reserve move, and without the marker it would be offered again forever.
      next = applyEffects(next, [
        { kind: "marker", feId: chosen.actorId, marker: "reserveMoved", added: true },
      ]);

      if (!mayProceed(next, chosen.actorId)) continue;

      next = await resolveMoveLive(
        next,
        chosen,
        config,
        turn,
        { phase: "arcCounteraction" },
        intents,
      );

      // "At the end of its Move, the Reserve FE/Group can DirF or Hasty Assault."
      const moved = next.forceElements[chosen.actorId];
      if (!moved || moved.combatStrength <= 0) continue;
      const followUps = reserveFollowUpOptionsFor(next, moved, config);
      if (followUps.length === 0) continue;

      const followUp = await decide(next, side, followUps, "reserveFollowUp");
      if (!followUp) continue;

      config.log.append({
        type: "decision",
        turn,
        phase: "arcCounteraction",
        rulesetId: config.ruleset.id,
        side,
        actorId: followUp.actorId,
        question: "reserve fire or hasty assault",
        options: followUps,
        chosenId: followUp.id,
        chosenBy: "heuristic",
      });

      // A Hasty Assault is still an assault: it rolls for Surprise and goes in
      // through Defensive Fire like any other (9.3, 9.3.4).
      next =
        followUp.kind === "assault"
          ? await resolveAssaultActionLive(
              next,
              followUp,
              config,
              turn,
              standing,
              "counteraction",
              "arcCounteraction",
              intents,
            )
          : resolveAction(next, followUp, config, turn, {
              phase: "arcCounteraction",
              counteractionFire: true,
            });
    }
  }

  // ── Stage 2: Counteraction Fire ─────────────────────────────────────────
  const firePassed: Record<Side, boolean> = { blue: false, red: false };
  guard = 0;
  while ((!firePassed.blue || !firePassed.red) && guard < 200) {
    guard += 1;
    for (const side of order) {
      if (firePassed[side]) continue;

      const options = forceElementsOf(next, side).flatMap((fe) =>
        counteractionFireOptionsFor(next, fe, config),
      );
      if (options.length === 0) {
        firePassed[side] = true;
        continue;
      }

      const chosen = await decide(next, side, options, "counteractionFire");
      if (!chosen || !chosen.actorId) {
        // Final for the turn, by rule.
        firePassed[side] = true;
        continue;
      }

      config.log.append({
        type: "decision",
        turn,
        phase: "arcCounteraction",
        rulesetId: config.ruleset.id,
        side,
        actorId: chosen.actorId,
        question: "counteraction fire",
        options,
        chosenId: chosen.id,
        chosenBy: "heuristic",
      });

      next = resolveAction(next, chosen, config, turn, {
        phase: "arcCounteraction",
        counteractionFire: true,
      });
      next = applyEffects(next, [
        { kind: "marker", feId: chosen.actorId, marker: "counteracted", added: true },
      ]);
    }
  }

  return next;
}

/**
 * Morale checks, in clean-up.
 *
 * ⚠ THIS PHASE DID NOT EXIST, AND ITS ABSENCE MADE TROOP QUALITY INERT.
 *
 * `resolveMoraleCheck` was written, unit tested, and never called by anything.
 * It is the ONLY consumer of `troopQuality`, so a veteran sub-unit and a
 * conscript one behaved identically: morale degraded from hits alone, through
 * the fire resolver, with no check and therefore no quality. A force list
 * could declare `elite` and it changed nothing.
 *
 * Every FE that has lost strength and is not already broken tests. Passing
 * does nothing; failing costs a morale step. That makes Troop Quality the
 * lever it is supposed to be — a better-trained unit absorbs the same
 * casualties and keeps fighting.
 *
 * `moraleLadder` off collapses the five-step ladder to good-or-broken, which
 * is what the flag is for and what makes it measurable.
 */
/**
 * THE RALLY STEP (5.2), the first thing that happens in a Turn.
 *
 * ⚠ THE ONLY WAY BACK UP THE MORALE LADDER, AND IT DID NOT EXIST.
 *
 * Clean-up's morale check skips any element that has not LOST Combat
 * Strength, and a "suppress" fire result costs no strength at all. So the
 * commonest damaged state on the board — shaken but intact — had no check to
 * take, no pass to make, and no way back. Suppressed meant suppressed for the
 * rest of the game.
 *
 * The LLM commander found it before the code did: eleven consecutive turns of
 * plans that said "waiting to rally". It was describing a rule the rulebook
 * has and this engine did not.
 *
 * Skipped on Turn 1, which is 5.2's own play note — everything starts at Good
 * Morale, so there would be nothing to roll for.
 */
export function runRally(state: GameState, config: PhaseConfig, turn: number): GameState {
  if (!config.ruleset.modules.rally) return state;
  if (turn <= 1) return state;

  let next = state;

  for (const fe of Object.values(state.forceElements)) {
    if (fe.combatStrength <= 0) continue;

    const current = next.forceElements[fe.id];
    // "each FE with a Morale Status marker" — an element at Good Morale has
    // no marker and nothing to recover.
    if (!current || current.morale === "good") continue;

    const outcome = resolveRally(
      current,
      {
        hqPresent: hasUnsuppressedHq(next, current),
        outOfContact: !inSightOfAnyEnemy(next, current, config),
      },
      config.ruleset,
      config.rng,
      turn,
    );
    // The log stamps the sequence number, and the step wants the stamped
    // event: an event without its place in the log cannot be traced back to it.
    const logged = config.log.append(outcome.event);
    next = applyEffects(next, outcome.effects);
    recordStep(config, next, turn, "command", logged.narrative ?? `${current.label} rallies`, {
      side: current.side,
      actorId: current.id,
      events: [logged],
    });
  }

  return next;
}

/**
 * Can any living enemy see this element?
 *
 * 5.2's worked example makes being "out of LoS of any enemy FE" an automatic
 * level of recovery, which is what turns breaking contact into a move worth
 * making. Geometry only: this asks what the enemy COULD see, not what it has
 * sighted, because the rule is about being out of view rather than about
 * anybody's fog of war.
 */
function inSightOfAnyEnemy(
  state: GameState,
  fe: ForceElement,
  config: PhaseConfig,
): boolean {
  return forceElementsOf(state, opposing(fe.side)).some(
    (enemy) =>
      enemy.combatStrength > 0 &&
      lineOfSight(config.terrain, { from: enemy.position, to: fe.position }).visible,
  );
}

/**
 * Abandon any march whose author has now seen the enemy.
 *
 * ⚠ A MARCH IS A PLAN MADE IN THE ABSENCE OF THE ENEMY. Sighting one makes it
 * stale by definition: the element is now choosing between fighting, skirting
 * and pressing on, and continuing to offer "continue along the route" would
 * hide that choice instead of posing it. Also drops a route that has arrived
 * or run out of waypoints — finished rather than interrupted, but equally over.
 *
 * Run after sighting and before anything activates, in both sequences of play.
 */
export function clearRoutesOnContact(
  state: GameState,
  config: PhaseConfig,
): GameState {
  if (!config.ruleset.modules.routeMarch) return state;

  let next = state;
  for (const fe of Object.values(state.forceElements)) {
    if (!fe.route) continue;
    const sightedEnemy = forceElementsOf(state, opposing(fe.side)).some(
      (enemy) => enemy.combatStrength > 0 && sightingOf(state, fe.side, enemy.id) !== "none",
    );
    if (
      routeInterrupted(fe.route, {
        sightedEnemy,
        position: fe.position,
        arrivedWithinM: ASSAULT_RANGE_M,
      })
    ) {
      next = applyEffects(next, [{ kind: "route", feId: fe.id, route: null }]);
    }
  }
  return next;
}

export function runMoraleChecks(state: GameState, config: PhaseConfig, turn: number): GameState {
  let next = state;

  for (const fe of Object.values(state.forceElements)) {
    if (fe.combatStrength <= 0) continue;
    // An unhurt unit has nothing to be shaken about, and checking anyway
    // would make morale a slow tax on time rather than a response to damage.
    if (fe.combatStrengthStart - fe.combatStrength <= 0) continue;
    if (fe.morale === "broken") continue;

    const current = next.forceElements[fe.id];
    if (!current) continue;

    const outcome = resolveMoraleCheck(
      current,
      {
        // An HQ in the same task group steadies the unit. Reachable only once
        // a force list fields one, which is why hqPresent is a named debt.
        hqPresent: hasUnsuppressedHq(next, current),
        multipleDirections: firedOnFromMultipleDirections(next, current),
      },
      config.ruleset,
      config.rng,
      turn,
      "cleanup",
    );
    config.log.append(outcome.event);
    next = applyEffects(next, outcome.effects);

    // With the ladder off, morale is binary: a failed check breaks you.
    if (!config.ruleset.modules.moraleLadder) {
      const after = next.forceElements[fe.id];
      if (after && after.morale !== "good") {
        next = applyEffects(next, [{ kind: "morale", feId: fe.id, to: "broken" }]);
      }
    }
  }

  return next;
}

/**
 * Is an HQ from the same side and task group co-located and still effective?
 *
 * Co-located means within a kilometre, which is a house number and belongs in
 * the ruleset the moment anybody argues about it.
 */
function hasUnsuppressedHq(state: GameState, fe: ForceElement): boolean {
  return Object.values(state.forceElements).some(
    (other) =>
      other.id !== fe.id &&
      other.side === fe.side &&
      other.commandRating != null &&
      other.morale === "good" &&
      other.combatStrength > 0 &&
      distanceM(other.position, fe.position) <= 1000,
  );
}

/** Was this FE engaged from two or more meaningfully different bearings? */
function firedOnFromMultipleDirections(state: GameState, fe: ForceElement): boolean {
  const bearings: number[] = [];
  for (const other of Object.values(state.forceElements)) {
    if (other.side === fe.side || other.combatStrength <= 0) continue;
    if (!other.markers.includes("fired")) continue;
    bearings.push(
      Math.atan2(other.position.lat - fe.position.lat, other.position.lng - fe.position.lng),
    );
  }

  // 90 degrees apart or more counts as two directions. Anything less is one
  // arc with a wide frontage, which is not the same tactical problem.
  for (let i = 0; i < bearings.length; i += 1) {
    for (let j = i + 1; j < bearings.length; j += 1) {
      let delta = Math.abs(bearings[i] - bearings[j]);
      if (delta > Math.PI) delta = 2 * Math.PI - delta;
      if (delta >= Math.PI / 2) return true;
    }
  }
  return false;
}

/**
 * One full turn: Command, Initiative, the two ARC rounds, Clean-up.
 *
 * ⚠ BOTH ARC ROUNDS ARE HERE NOW, AND THAT IS WHY THE SWEEP CAN BE BELIEVED.
 *
 * This is the sequence rules/harness.ts measures. While Reactive Fire and the
 * Counteraction Round existed only in the orders sequence, `moduleImpact`
 * compared two identical games for both of them and would have reported them
 * as ceremony. The rules are the same in both sequences; only who chooses,
 * and when, differs.
 *
 * Standing orders are empty here: the per-activation Commander has no step in
 * which to declare rules of engagement, so every element falls back to
 * `defaultEngage`. That is what the default is for, and it means the harness
 * measures the mechanic as an unbriefed force would fight it.
 */
export async function runTurn(state: GameState, config: GameConfig): Promise<GameState> {
  const turn = state.turn;
  const standing = noStandingOrders();

  // ── Command Sub-phase (5.1), step 1: RALLY (5.2) ────────────────────────
  // Before initiative, because 5.1 puts the whole Command Sub-phase before
  // the Initiative Sub-phase, and because an element that recovers here is
  // eligible for everything that follows in the same Turn.
  const rallied = runRally({ ...state, phase: "command" }, config, turn);
  recordStep(config, rallied, turn, "command", `Turn ${turn} begins`);

  const initiative = resolveInitiative(
    { blue: rallied.sides.blue.transmissions, red: rallied.sides.red.transmissions },
    { blue: rallied.sides.blue.eliminatedLastTurn, red: rallied.sides.red.eliminatedLastTurn },
    config.ruleset,
    config.rng,
    turn,
  );
  config.log.append(initiative.event);
  let next = applyEffects({ ...rallied, phase: "initiative" }, initiative.effects);

  next = runSighting({ ...next, phase: "arcAction" }, config, turn);
  next = clearRoutesOnContact(next, config);
  recordStep(config, next, turn, "arcAction", "Sighting");

  // Command Sub-phase: who is held back. Before any activation, because a
  // reserve is a plan, not a leftover.
  next = nominateReserves(next, "blue", config);
  next = nominateReserves(next, "red", config);

  // ── Action-Reaction Round (7.1) ─────────────────────────────────────────
  // Alternating activation until neither side can act. The side with
  // initiative goes first; a side that cannot act is skipped rather than
  // ending the round, so one side having more units does not end the turn
  // early for the other.
  const order: Side[] = initiative.winner === "blue" ? ["blue", "red"] : ["red", "blue"];
  let anyActed = true;
  let guard = 0;
  while (anyActed && guard < 200) {
    anyActed = false;
    for (const side of order) {
      const result = await activate(next, side, config, turn, standing);
      next = result.state;
      anyActed = anyActed || result.acted;
    }
    guard += 1;
  }

  // ── Counteraction Round (7.2) ───────────────────────────────────────────
  // The same commanders, asked from the same option lists, so a bot and a
  // model play this round by the same rules as the first one.
  next = await runCounteractionRound(
    next,
    config,
    turn,
    initiative.winner,
    standing,
    async (current, side, options) => {
      const view = projectForSide(current, side);
      const commander = config.commanders[side];
      const { optionId } = await commander.decide(
        view,
        options,
        "the counteraction round: move up, or open fire?",
      );
      return options.find((option) => option.id === optionId) ?? options[0];
    },
  );

  // Morale BEFORE markers are cleared: multipleDirections reads the "fired"
  // markers left by this turn's activations, so clearing first would make the
  // modifier permanently unreachable.
  next = runMoraleChecks({ ...next, phase: "cleanup" }, config, turn);
  next = endStaleMelees(next, config);
  recordStep(config, next, turn, "cleanup", "Clean-up");

  // Clean-up: markers go, losses are counted for next turn's initiative.
  const eliminated = (side: Side) =>
    forceElementsOf(next, side).filter((fe) => fe.combatStrength <= 0).length;

  return {
    ...clearAllMarkers({ ...next, phase: "cleanup" }, config.ruleset.modules.closeCombat),
    turn: turn + 1,
    phase: "command",
    sides: {
      blue: {
        ...next.sides.blue,
        transmissionsLastTurn: next.sides.blue.transmissions,
        eliminatedLastTurn: eliminated("blue"),
      },
      red: {
        ...next.sides.red,
        transmissionsLastTurn: next.sides.red.transmissions,
        eliminatedLastTurn: eliminated("red"),
      },
    },
  };
}

export interface GameOutcome {
  state: GameState;
  turns: number;
  /** Who won, per rules/victory.ts — objectives first, attrition last. */
  winner: Side | null;
  /** How the game STOPPED. Not the same question as who won. */
  reason: "annihilation" | "turnLimit";
  /** How well they won, and on which clause. */
  verdict: Verdict;
}

/** Run to a conclusion. This is what the experiment harness calls. */
export async function runGame(initial: GameState, config: GameConfig): Promise<GameOutcome> {
  let state = initial;
  let turns = 0;

  while (turns < config.maxTurns) {
    const blueAlive = forceElementsOf(state, "blue").some((fe) => fe.combatStrength > 0);
    const redAlive = forceElementsOf(state, "red").some((fe) => fe.combatStrength > 0);
    if (!blueAlive || !redAlive) {
      const verdict = judgeVictory(state, config.ruleset);
      return { state, turns, winner: verdict.winner, reason: "annihilation", verdict };
    }
    state = await runTurn(state, config);
    turns += 1;
  }

  // A turn limit is not a draw, and it is not an arithmetic comparison either.
  // It is judged: the ground first, then whether the enemy is still a force,
  // and only then what is left in the field. See rules/victory.ts.
  const verdict = judgeVictory(state, config.ruleset);
  return { state, turns, winner: verdict.winner, reason: "turnLimit", verdict };
}
