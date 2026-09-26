// ── bgws/rules/jevState.ts ─────────────────────────────────────────────────
// What Jev is told, for each kind of question.
//
// TWO RULES, BOTH INHERITED FROM THE LLM COMMANDER
// ------------------------------------------------
// 1. THE ENGINE DOES THE ARITHMETIC. Jev judges; it does not compute. So the
//    state carries the answers to "how likely is this shot to do anything?"
//    as numbers, worked out exactly from the same fire table the dice will be
//    read against. Asking a decision model to multiply modifiers would be
//    asking it to be bad at the one thing the engine is certain about.
//
// 2. NOTHING A SIDE COULD NOT KNOW. Everything about the enemy comes through
//    the fog-of-war projection or is deliberately blinded — the odds are
//    worked out against the enemy AS SEEN: its morale assumed steady, and its
//    armour unknown unless it has been fully identified. A decider handed the
//    true odds would be reading the opponent's hidden state through a number.
//
// Every function here is pure, and its output is meant to be READ — the
// fastest way to find out why a tank held fire is to look at what it was told.

import { bearingDeg, distanceM, type LatLng } from "../lib/board";
import { projectForSide, type ObservedForceElement, type SideView } from "../lib/fogOfWar";
import { inCover } from "../lib/proceduralTerrain";
import { sightingOf, type ForceElement, type GameState, type Side } from "../lib/state";
import type { ResolutionEvent } from "./events";
import { lineOfSight } from "../lib/lineOfSight";
import type { ActionOption } from "./commander";
import type { Rng } from "./dice";
import { resolveDirectFire, type FireContext } from "./resolvers";
import type { RuleSet } from "./ruleset";
import type {
  ActivationMoment,
  CommanderIntent,
  ContactMoment,
  ObserverMoment,
  OptionMoment,
  ReactionMoment,
} from "./tactical";
import { isFlankShot, smokeOnLine, weaponFor, type PhaseConfig } from "./turnLoop";

// ── Odds ───────────────────────────────────────────────────────────────────

export interface FireOdds {
  /** Probability the shot does nothing at all. */
  pNoEffect: number;
  /** Probability it suppresses without a hit. */
  pSuppress: number;
  /** Probability of at least one hit. */
  pHit: number;
  /** Expected hits. One hit costs the target Combat Strength and morale. */
  expectedHits: number;
}

const HITS: Record<string, number> = { oneHit: 1, twoHits: 2, threeHits: 3 };

/**
 * The exact odds of a direct-fire shot, by enumerating the dice.
 *
 * Fire is one 2D6 roll against a table, so there are 36 outcomes and each is
 * equally likely. Resolving the shot once per outcome, with a generator that
 * returns that outcome, gives the distribution with no sampling error and no
 * second copy of the fire rules to drift out of step with the first. The
 * game's own generator is never touched.
 */
export function fireOdds(
  firers: readonly ForceElement[],
  target: ForceElement,
  context: FireContext,
  ruleset: RuleSet,
): FireOdds {
  let none = 0;
  let suppress = 0;
  let hit = 0;
  let hits = 0;

  for (let a = 1; a <= 6; a += 1) {
    for (let b = 1; b <= 6; b += 1) {
      const fixed: Rng = {
        seed: "odds",
        cursor: 0,
        d6: () => ({ dice: [a], total: a, cursor: 0 }),
        d66: () => ({ dice: [a, b], total: a + b, cursor: 0 }),
        int: () => 0,
        pick: <T,>(items: readonly T[]) => items[0],
      };
      const result = resolveDirectFire(firers, target, context, ruleset, fixed, 0, "arcReaction")
        .event.result;
      const count = HITS[result] ?? 0;
      if (count > 0) {
        hit += 1;
        hits += count;
      } else if (result === "suppress") {
        suppress += 1;
      } else {
        none += 1;
      }
    }
  }

  const round2 = (x: number) => Math.round(x * 100) / 100;
  return {
    pNoEffect: round2(none / 36),
    pSuppress: round2(suppress / 36),
    pHit: round2(hit / 36),
    expectedHits: round2(hits / 36),
  };
}

/**
 * The enemy as the observing side can judge it.
 *
 * Morale is not observable, so it is assumed steady. Armour is only known once
 * the counter has been identified; before that it is unknown, and unknown
 * armour fails open in the penetration rule — which is also how a real crew
 * would estimate a shot at something it cannot make out.
 */
function asObserved(target: ForceElement, observed: ObservedForceElement | undefined): ForceElement {
  const identified = observed?.sighting === "full";
  return {
    ...target,
    morale: "good",
    markers: observed ? [...observed.observedMarkers] : [],
    armour: identified ? target.armour : undefined,
    armourMm: identified ? target.armourMm : undefined,
    combatStrength: target.combatStrengthStart,
  };
}

// ── Building blocks ────────────────────────────────────────────────────────

const round0 = (x: number) => Math.round(x);
const compass = (deg: number) =>
  ["N", "NE", "E", "SE", "S", "SW", "W", "NW"][Math.round((((deg % 360) + 360) % 360) / 45) % 8];

/** The ground under a point, as the side standing on it would describe it. */
export function groundAt(config: PhaseConfig, point: LatLng) {
  return {
    terrain: config.terrain.classify(point),
    inCover: inCover(config.terrain, point),
    heightM: round0(config.terrain.groundHeightM(point)),
  };
}

/** One of my own elements, completely — it is mine. */
export function ownBrief(fe: ForceElement, config: PhaseConfig) {
  return {
    id: fe.id,
    unit: fe.label,
    moveType: fe.moveType,
    strength: `${fe.combatStrength}/${fe.combatStrengthStart}`,
    troopQuality: fe.troopQuality,
    morale: fe.morale,
    markers: fe.markers,
    ...(fe.ammo ? { ammo: fe.ammo } : {}),
    weapons: fe.capabilities.map((c) => `${c.kind} to ${c.maxRangeM} m`),
    ...groundAt(config, fe.position),
  };
}

/** An enemy as seen from one of mine. */
function contactFrom(contact: ObservedForceElement, from: LatLng, config: PhaseConfig) {
  return {
    id: contact.id,
    identified: contact.sighting === "full",
    unit: contact.label ?? "unidentified",
    rangeM: round0(distanceM(from, contact.position)),
    bearing: compass(bearingDeg(from, contact.position)),
    lineOfSight: lineOfSight(config.terrain, { from, to: contact.position }).visible,
    seen: contact.observedMarkers,
    inCover: inCover(config.terrain, contact.position),
  };
}

/** How exposed one of my elements is: known enemies that can see it. */
function exposure(fe: ForceElement, view: SideView, config: PhaseConfig) {
  const watching = view.contacts.filter(
    (contact) =>
      distanceM(contact.position, fe.position) <= 3000 &&
      lineOfSight(config.terrain, { from: contact.position, to: fe.position }).visible,
  );
  return {
    knownEnemiesWithLineOfSight: watching.length,
    nearestKnownEnemyM: view.contacts.length
      ? round0(Math.min(...view.contacts.map((c) => distanceM(c.position, fe.position))))
      : null,
  };
}

// ── The wider situation of one element ─────────────────────────────────────

/**
 * What one of my elements faces, beyond what it can see this instant.
 *
 * Five things a crew would know and the earlier state left out:
 *   objective  how far, which way, and whether it is already held
 *   support    friends close enough to fire or assault together (9.2.1)
 *   threats    what each IDENTIFIED enemy in sight would do to it, as odds —
 *              the mirror of oddsIfFiring, and the half of the trade that was
 *              missing. Unidentified contacts are not scored: their weapons
 *              are exactly what nobody knows yet.
 *   height     relative to the nearest known enemy, because being above
 *              someone is most of what a good position is
 */
export function situationOf(
  fe: ForceElement,
  state: GameState,
  view: SideView,
  config: PhaseConfig,
) {
  const objective = state.objectives?.[fe.side];
  const objectiveM = objective ? distanceM(fe.position, objective) : undefined;

  const support = view.own
    .filter(
      (friend) =>
        friend.id !== fe.id && distanceM(friend.position, fe.position) <= config.ruleset.coLocatedM,
    )
    .map((friend) => friend.id);

  const threats = view.contacts
    .filter((contact) => contact.sighting === "full")
    .map((contact) => ({ contact, rangeM: distanceM(contact.position, fe.position) }))
    .filter(({ rangeM }) => rangeM <= 3000)
    .sort((a, b) => a.rangeM - b.rangeM)
    .slice(0, 4)
    .flatMap(({ contact, rangeM }) => {
      const enemy = state.forceElements[contact.id];
      if (!enemy) return [];
      if (!lineOfSight(config.terrain, { from: contact.position, to: fe.position }).visible) return [];
      const weapon = weaponFor(enemy, fe, rangeM);
      if (!weapon) return [];
      const odds = fireOdds(
        [asObserved(enemy, contact)],
        fe,
        {
          rangeM,
          maxRangeM: weapon.maxRangeM,
          penetrationMm: weapon.penetrationMm,
          munition: weapon.munition,
          topAttack: weapon.topAttack,
          targetInCover: inCover(config.terrain, fe.position),
          flank: isFlankShot([enemy], fe, config.ruleset),
          smoke: smokeOnLine(state, contact.position, fe.position, config.ruleset),
        },
        config.ruleset,
      );
      return [{ from: contact.id, rangeM: round0(rangeM), pHitOnYou: odds.pHit, expectedHits: odds.expectedHits }];
    });

  const nearest = view.contacts
    .map((contact) => ({ contact, rangeM: distanceM(contact.position, fe.position) }))
    .sort((a, b) => a.rangeM - b.rangeM)[0];

  return {
    ...(objective && objectiveM != null
      ? {
          objective: {
            distanceM: round0(objectiveM),
            bearing: compass(bearingDeg(fe.position, objective)),
            onIt: objectiveM <= config.ruleset.victory.holdWithinM,
          },
        }
      : {}),
    canActTogetherWith: support,
    ...(threats.length ? { threatsToYou: threats } : {}),
    ...(nearest
      ? {
          heightAboveNearestEnemyM: round0(
            config.terrain.groundHeightM(fe.position) -
              config.terrain.groundHeightM(nearest.contact.position),
          ),
        }
      : {}),
  };
}

/**
 * The last few exchanges this side took part in, as this side saw them.
 *
 * Jev keeps nothing between calls, so without this every question is asked
 * of a commander with no memory of the shot that just missed. Built from the
 * log, and FOG-SAFE: an enemy this side has not sighted is "unseen", never
 * named, even when the log knows perfectly well who it was.
 */
export function recentEvents(state: GameState, config: PhaseConfig, side: Side, limit = 6) {
  const name = (id: string) => {
    const fe = state.forceElements[id];
    if (!fe) return id;
    if (fe.side === side) return id;
    return sightingOf(state, side, id) === "none" ? "unseen enemy" : id;
  };
  const involved = (event: ResolutionEvent) =>
    [...event.actorIds, ...event.targetIds].some(
      (id) => state.forceElements[id]?.side === side,
    );

  return config.log
    .all()
    .filter(
      (event): event is ResolutionEvent =>
        event.type === "resolution" &&
        (event.kind === "directFire" || event.kind === "indirectFire" || event.kind === "assault"),
    )
    .filter(involved)
    .slice(-limit)
    .map((event) => ({
      turn: event.turn,
      what: event.kind,
      by: event.actorIds.map(name),
      at: event.targetIds.map(name),
      result: event.result,
    }));
}

function intentFor(intent: CommanderIntent | undefined, feId: string) {
  if (!intent) return undefined;
  const order = intent.orders?.[feId];
  return {
    ...(intent.plan ? { plan: intent.plan } : {}),
    ...(order ? { thisElementsOrder: order.summary, why: order.why } : {}),
  };
}

// ── Reactive fire ──────────────────────────────────────────────────────────

/**
 * The state for "does this element snap-fire at that mover, now?"
 *
 * One shared picture of the moment plus one entry per eligible reactor, each
 * with the odds IT would get — so the same request can ask every reactor
 * without repeating the board six times.
 */
export function reactionState(moment: ReactionMoment) {
  const { state, config, side } = moment;
  const view = projectForSide(state, side);
  const actor = state.forceElements[moment.actorId];
  const observed = view.contacts.find((contact) => contact.id === moment.actorId);
  const perceived = actor ? asObserved(actor, observed) : undefined;

  const reactors = moment.candidates.map((candidate) => {
    const fe = state.forceElements[candidate.reactorId];
    const weapon = fe && actor ? weaponFor(fe, actor, candidate.rangeM) : undefined;
    const odds =
      fe && actor && perceived && weapon
        ? fireOdds(
            [fe],
            perceived,
            {
              rangeM: candidate.rangeM,
              maxRangeM: weapon.maxRangeM,
              penetrationMm: weapon.penetrationMm,
              munition: weapon.munition,
              topAttack: weapon.topAttack,
              targetInCover: inCover(config.terrain, actor.position),
              flank: isFlankShot([fe], actor, config.ruleset),
              smoke: smokeOnLine(state, fe.position, actor.position, config.ruleset),
              snapShot: true,
              counteractionFire: moment.round === "counteraction" ? true : undefined,
            },
            config.ruleset,
          )
        : undefined;

    return {
      ...(fe ? ownBrief(fe, config) : { id: candidate.reactorId }),
      rangeToTargetM: candidate.rangeM,
      weapon: candidate.capability,
      rulesOfEngagement: candidate.engage,
      rulesOfEngagementSays: candidate.ruleSaysReact ? "fire" : "hold",
      ...(odds ? { oddsIfFiring: odds } : {}),
      ...(fe ? exposure(fe, view, config) : {}),
      ...(fe ? situationOf(fe, state, view, config) : {}),
      ...(intentFor(moment.intent, candidate.reactorId)
        ? { orders: intentFor(moment.intent, candidate.reactorId) }
        : {}),
    };
  });

  return {
    situation: "An enemy element is acting inside your arc. Decide who fires on it now.",
    turn: moment.turn,
    round: moment.round === "counteraction" ? "counteraction (second round)" : "action-reaction",
    you: side,
    ...(moment.intent?.plan ? { commandersPlan: moment.intent.plan } : {}),
    enemy: {
      ...(observed && actor ? contactFrom(observed, actor.position, config) : { id: moment.actorId }),
      firingOnYourSide: moment.wasFiredUpon,
      ...(actor ? { ground: groundAt(config, actor.position) } : {}),
    },
    rules: {
      maxReactors: moment.maxReactors,
      costOfReacting:
        "A reacting element is marked FIRED: it cannot act again this turn, and " +
        "firing reveals a concealed position. A snap shot is worse than deliberate fire.",
      hitsStopMovers:
        "If the mover is Disrupted or Broken by reactive fire, its move does not happen.",
    },
    reactors,
    recentEvents: recentEvents(state, config, side),
    otherKnownEnemies: view.contacts
      .filter((contact) => contact.id !== moment.actorId)
      .map((contact) => ({
        id: contact.id,
        unit: contact.label ?? "unidentified",
        nearestOfYoursM: round0(
          Math.min(...view.own.map((fe) => distanceM(fe.position, contact.position))),
        ),
      })),
  };
}

// ── Contact ────────────────────────────────────────────────────────────────

/** The state for "you have just run into the enemy mid-move: halt or press on?" */
export function contactState(moment: ContactMoment) {
  const { state, config, side } = moment;
  const view = projectForSide(state, side);
  const mover = state.forceElements[moment.actorId];
  const destination = moment.option.destination;

  const newly = moment.newContacts
    .map((id) => view.contacts.find((contact) => contact.id === id))
    .filter((contact): contact is ObservedForceElement => contact != null);

  const why: Record<typeof moment.trigger, string> = {
    contact: "Your element was moving and has just made contact with the enemy. It has halted.",
    underFire: "Your element was fired on as it set off. It can still move.",
    exposed: "Your element is moving into an identified enemy's sight and range.",
    setback: "A friend close to your element has been destroyed or broken this turn.",
  };
  return {
    situation: `${why[moment.trigger]} Decide whether it carries on, halts, or breaks for cover.`,
    trigger: moment.trigger,
    ...(moment.detail ? { detail: moment.detail } : {}),
    ...(moment.cover ? { nearestCover: moment.cover } : { nearestCover: "none within reach" }),
    turn: moment.turn,
    you: side,
    ...(moment.intent?.plan ? { commandersPlan: moment.intent.plan } : {}),
    mover: mover
      ? {
          ...ownBrief(mover, config),
          ...exposure(mover, view, config),
          ...situationOf(mover, state, view, config),
          order: moment.option.summary,
          commanderPreferredOnContact: moment.preferred,
          ...(intentFor(moment.intent, moment.actorId)
            ? { orders: intentFor(moment.intent, moment.actorId) }
            : {}),
        }
      : { id: moment.actorId },
    remainingMoveM: round0(moment.remainingM),
    ...(destination ? { destinationGround: groundAt(config, destination) } : {}),
    newContacts: mover ? newly.map((contact) => contactFrom(contact, mover.position, config)) : [],
    recentEvents: recentEvents(state, config, side),
    otherKnownEnemies: mover
      ? view.contacts
          .filter((contact) => !moment.newContacts.includes(contact.id))
          .map((contact) => contactFrom(contact, mover.position, config))
      : [],
  };
}

// ── Sighting interrupt ─────────────────────────────────────────────────────

/**
 * The state for "who tries to make out the concealed thing that just moved?"
 *
 * The enemy is described only by where the activity was seen and what ground
 * it is on. It is Concealed: its identity is exactly what the attempt is for.
 */
export function observerState(moment: ObserverMoment) {
  const { state, config, side } = moment;
  const actor = state.forceElements[moment.actorId];
  return {
    situation:
      "A concealed enemy element has just acted. One of your elements may try to identify it.",
    turn: moment.turn,
    you: side,
    ...(actor ? { activityAt: groundAt(config, actor.position) } : {}),
    observers: moment.observers.map((observer) => {
      const fe = state.forceElements[observer.observerId];
      return {
        ...(fe ? ownBrief(fe, config) : { id: observer.observerId }),
        rangeM: observer.rangeM,
        recce: observer.recce,
      };
    }),
  };
}

// ── Engine choices nobody was asked about ──────────────────────────────────

/** The state for a choice the engine would otherwise make by heuristic. */
export function optionState(moment: OptionMoment) {
  const view = projectForSide(moment.state, moment.side);
  const actorIds = new Set(moment.options.map((option) => option.actorId).filter(Boolean));
  return {
    situation: moment.question,
    ...(moment.intent?.plan ? { commandersPlan: moment.intent.plan } : {}),
    ...sideState(view, moment.config, moment.state),
    deciding: [...actorIds].map((id) => {
      const order = moment.intent?.orders?.[id as string];
      return { id, ...(order ? { order: order.summary, why: order.why } : {}) };
    }),
  };
}

// ── Activations ────────────────────────────────────────────────────────────

/**
 * The state for "which of your committed elements acts now, and how?"
 *
 * The whole side's picture plus, for each element still holding orders, what
 * it was told to do. The options themselves are the question's criteria, not
 * part of the state, so they are not repeated here.
 */
export function activationState(moment: ActivationMoment) {
  const view = projectForSide(moment.state, moment.side);
  return {
    situation:
      "Your commander planned this turn. The turn is now being fought, one " +
      "activation at a time, alternating with the enemy. Choose which of your " +
      "committed elements acts now, and what it does.",
    ...(moment.intent?.plan ? { commandersPlan: moment.intent.plan } : {}),
    stillToAct: moment.candidates.map((candidate) => ({
      id: candidate.actorId,
      orderedTo: candidate.orderedSummary ?? "no specific order",
      orderStillPossible: candidate.orderedOptionId != null,
      ...(moment.intent?.orders?.[candidate.actorId]?.why
        ? { why: moment.intent.orders[candidate.actorId].why }
        : {}),
    })),
    ...sideState(view, moment.config, moment.state),
  };
}

// ── Whole-side decisions ───────────────────────────────────────────────────

/** A side's picture of the battle, for commanding the whole of it. */
export function sideState(view: SideView, config?: PhaseConfig, state?: GameState) {
  return {
    turn: view.turn,
    you: view.side,
    initiative: view.initiative,
    yourElements: view.own.map((fe) =>
      config
        ? {
            ...ownBrief(fe, config),
            ...exposure(fe, view, config),
            ...(state ? situationOf(fe, state, view, config) : {}),
          }
        : {
            id: fe.id,
            unit: fe.label,
            strength: `${fe.combatStrength}/${fe.combatStrengthStart}`,
            troopQuality: fe.troopQuality,
            morale: fe.morale,
            markers: fe.markers,
          },
    ),
    knownEnemies: view.contacts.map((contact) => ({
      id: contact.id,
      identified: contact.sighting === "full",
      unit: contact.label ?? "unidentified",
      seen: contact.observedMarkers,
      nearestOfYoursM: view.own.length
        ? round0(Math.min(...view.own.map((fe) => distanceM(fe.position, contact.position))))
        : null,
    })),
    ...(state && config ? { recentEvents: recentEvents(state, config, view.side) } : {}),
    note:
      "Enemy elements you have not sighted are not listed; their absence is not " +
      "evidence that they are not there.",
  };
}

/**
 * An option, described well enough to judge.
 *
 * The engine's summary says WHAT the option is. Jev also needs to know what
 * it would be like to have done it: where a move ends up and how exposed that
 * is, how far a shot is. That is added here from the side's own knowledge.
 */
export function describeOption(
  option: ActionOption,
  view: SideView,
  config?: PhaseConfig,
): string {
  const parts = [option.summary];
  const actor = view.own.find((fe) => fe.id === option.actorId);

  if (option.kind === "move" && option.destination && config) {
    const ground = groundAt(config, option.destination);
    parts.push(`ends on ${ground.terrain}${ground.inCover ? " (cover)" : " (in the open)"}`);
    const seenBy = view.contacts.filter(
      (contact) =>
        distanceM(contact.position, option.destination!) <= 3000 &&
        lineOfSight(config.terrain, { from: contact.position, to: option.destination! }).visible,
    ).length;
    if (view.contacts.length > 0) parts.push(`visible to ${seenBy} known enemy there`);
    if (option.onContact) parts.push(`${option.onContact} on contact`);
  }

  if ((option.kind === "fire" || option.kind === "assault") && option.targetId && actor) {
    const target = view.contacts.find((contact) => contact.id === option.targetId);
    if (target) parts.push(`range ${round0(distanceM(actor.position, target.position))} m`);
  }

  if (option.actorIds && option.actorIds.length > 1) {
    parts.push(`combined, ${option.actorIds.length} elements`);
  }

  return parts.join("; ");
}

/**
 * At most `limit` options, keeping every one that engages.
 *
 * Moves are the options that multiply — eight bearings, several distances, a
 * march — and the only ones worth dropping when Jev's 255 limit bites. Firing
 * and assaulting are kept whatever happens, because a decider that was never
 * offered the shot cannot take it.
 */
export function pruneOptions(options: readonly ActionOption[], limit: number): ActionOption[] {
  if (options.length <= limit) return [...options];
  const engaging = options.filter((option) => option.kind !== "move");
  const moves = options.filter((option) => option.kind === "move");
  const room = Math.max(0, limit - engaging.length);
  const stride = moves.length / Math.max(1, room);
  const kept = Array.from({ length: Math.min(room, moves.length) }, (_, i) => moves[Math.floor(i * stride)]);
  return [...engaging.slice(0, limit), ...kept].slice(0, limit);
}

