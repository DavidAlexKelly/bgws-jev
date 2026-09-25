// ── bgws/rules/orders.ts ───────────────────────────────────────────────────
// ORDERS: a whole side's intentions for a turn, decided in one go.
//
// WHY THIS EXISTS ALONGSIDE THE ACTIVATION LOOP
//
// `runTurn` uses alternating activation: it asks a commander for ONE action
// across its entire force, asks the other side, and repeats until neither can
// act. That is a good fit for a bot, which answers instantly, and every
// measured result in reports/ was produced that way.
//
// It is a poor fit for a model. A twelve-element engagement takes 20-50
// activations a turn, so alternating activation means 20-50 model calls per
// turn — slow, expensive, and with no point at which a commander sees its
// whole force and forms a plan.
//
// An ORDERS phase asks each side once: here is everything you can see, here
// is every legal option for every element you own, give me your intentions.
// Two calls a turn for the Action-Reaction Round, and two more for the
// Counteraction Round — see CounteractionRequest for why the second pair is
// worth its cost — and the commander gets to think about its force as a force
// rather than one vehicle at a time.
//
// THE SAME RULES, DIFFERING ONLY IN WHO CHOOSES AND WHEN. Same initiative,
// same sighting, same resolvers, same ARC rounds, same morale, same clean-up.
// A game played either way is played under the same rules.
//
// ⚠ THIS FILE USED TO SAY "`runTurn` IS DELIBERATELY UNTOUCHED", and that was
// right until it wasn't. The reasoning was that every number in reports/ had
// been measured through `runTurn`, so changing its phase structure would
// silently invalidate them. What it missed is that a mechanic implemented in
// ONLY this sequence is invisible to the harness, which measures the other
// one — so Reactive Fire and the Counteraction Round were unmeasurable for as
// long as the rule was honoured. Both now live in turnLoop.ts and are called
// from both sequences, and the report was regenerated rather than trusted.
// Keeping a sequence pristine is worth less than being able to measure it.

import { distanceM } from "../lib/board";
import { projectForSide } from "../lib/fogOfWar";
import type { GameState, Side } from "../lib/state";
import { forceElementsOf, hasMarker, opposing } from "../lib/state";
import { applyEffects, clearAllMarkers } from "./apply";
import type { ActionOption, FormationPreference } from "./commander";
import { resolveInitiative } from "./resolvers";
import {
  activationBudget,
  attemptSightingInterruptLive,
  chooseOptionLive,
  counteractionFireOptionsFor,
  endStaleMelees,
  mayProceed,
  nominateReserves,
  optionsFor,
  reserveLimitFor,
  reserveMoveOptionsFor,
  runCounteractionRound,
  clearRoutesOnContact,
  runMoraleChecks,
  runRally,
  reactiveFireLive,
  resolveAssaultActionLive,
  resolveMoveLive,
  type SideIntents,
  runSighting,
  willReact,
  type PhaseConfig,
  type StandingOrder,
  type StandingOrders,
} from "./turnLoop";

// Reactive Fire and its rules of engagement are SHARED with the activation
// sequence and live in turnLoop.ts — see the ARC section there for why. These
// re-exports keep `import { willReact } from "./orders"` working, because the
// orders phase is where a commander declares them and so is where a reader
// looks for them.
export { willReact };
export type { StandingOrder, StandingOrders };

/** One element's intention for this turn. */
export interface Intent {
  actorId: string;
  /** Must be the id of an option that was OFFERED. Anything else is dropped. */
  optionId: string;
  rationale?: string;
}

/**
 * A whole side's plan for the Action-Reaction Round.
 *
 * WHY RULES OF ENGAGEMENT ARE DECLARED IN ADVANCE RATHER THAN ASKED FOR.
 *
 * A reaction is a decision, so it could be put to the commander at the moment
 * it arises. That would mean a model call per opportunity — ten or twenty a
 * turn — and the orders phase exists precisely to avoid that.
 *
 * Declaring instead is also better doctrine, and it is what BGWS does: an FE
 * may only Reactive Fire under one of eleven Order Verbs, into a TAI assigned
 * to it in the Preparation Phase (7.1.3). A commander does not phone each tank
 * as the enemy crosses its arc; it writes the Sync Matrix beforehand and lives
 * with it.
 */
export interface Orders {
  side: Side;
  intents: Intent[];
  /** Rules of engagement for the turn. Absent elements use the ruleset default. */
  standingOrders?: StandingOrder[];
  /**
   * Elements held back as Reserve this turn (2.1.13).
   *
   * Capped at a third of the force by `reserveLimit`; anything beyond the cap
   * is dropped rather than rejected, because over-nominating is a planning
   * error and not an attempt to cheat.
   */
  reserves?: string[];
  /** The commander's overall intent, for the log. Never acted on mechanically. */
  plan?: string;
  /**
   * Set when the commander could not answer at all — the model was
   * unreachable, or its reply could not be read.
   *
   * A FIELD RATHER THAN A PROSE CONVENTION, because it is measured. A trial
   * has to separate "the model played badly" from "the model never answered",
   * and telling those apart by matching on the wording of `plan` would make
   * the experiment's headline number depend on a string nobody thought of as
   * an interface.
   */
  failure?: string;
}

/**
 * What a side is offered in the Counteraction Round (7.2).
 *
 * ⚠ THIS IS A SECOND COMMANDER CALL, AND IT IS WORTH IT.
 *
 * The orders phase exists to keep a turn at two model calls rather than fifty.
 * The Counteraction Round takes it to four, because the round's entire value
 * is that it is decided AFTER the Action-Reaction Round has been fought. A
 * reserve committed before seeing what happened is not a reserve, it is just
 * a late start; and Counteraction Fire chosen in advance cannot answer what
 * the enemy actually did. Four calls a turn is the honest price of the C.
 */
export interface CounteractionRequest {
  side: Side;
  turn: number;
  view: ReturnType<typeof projectForSide>;
  /** Reserve Moves available now, by element. Empty when nothing is in reserve. */
  reserveMoves: Record<string, ActionOption[]>;
  /** Counteraction Fire available now, by element — anything that has not fired. */
  fireOptions: Record<string, ActionOption[]>;
}

export interface CounteractionOrders {
  side: Side;
  /**
   * Option ids to take, in preference order. Anything not currently on offer
   * is skipped; running out is a Pass, and by rule a Pass is final.
   */
  optionIds: string[];
  plan?: string;
}

/** Everything a side is offered this turn, grouped by the element that owns it. */
export interface OrdersRequest {
  side: Side;
  turn: number;
  /** Fog-of-war filtered. The same projection the rules use. */
  view: ReturnType<typeof projectForSide>;
  /** Element id to its legal options. Elements with nothing to do are omitted. */
  optionsByElement: Record<string, ActionOption[]>;
  /** How many elements this side may commit this turn, or Infinity. */
  activationBudget: number;
  /** How many elements may be held in Reserve this turn (2.1.13). Zero when off. */
  reserveLimit: number;
}

/**
 * A commander that plans a whole turn.
 *
 * Separate interface from `Commander` rather than an addition to it, so the
 * harness's commanders keep working unchanged and a planner cannot be passed
 * where a per-activation decider is expected.
 */
export interface OrdersCommander {
  readonly kind: "human" | "heuristic" | "llm" | "jev";
  readonly name: string;
  planTurn(request: OrdersRequest): Promise<Orders>;
  /**
   * Plan the Counteraction Round, asked after the first round has been fought.
   *
   * Optional so that a commander written before the C existed still compiles
   * and still plays — it simply never commits a reserve and never takes a
   * second-round shot, which is a legible way to lose rather than a crash.
   */
  planCounteraction?(request: CounteractionRequest): Promise<CounteractionOrders>;
}

/** Every legal option this turn, grouped by element. */
export function ordersRequestFor(
  state: GameState,
  side: Side,
  config: PhaseConfig,
): OrdersRequest {
  const optionsByElement: Record<string, ActionOption[]> = {};

  for (const fe of forceElementsOf(state, side)) {
    if (fe.combatStrength <= 0) continue;
    const options = optionsFor(state, fe, config);
    // "hold" is always offered, so an element with only that has no decision
    // to make. Omitting it keeps a prompt about the units that matter.
    if (options.filter((option) => option.kind !== "hold").length === 0) continue;
    optionsByElement[fe.id] = options;
  }

  return {
    side,
    turn: state.turn,
    view: projectForSide(state, side),
    optionsByElement,
    activationBudget: activationBudget(state, side, config),
    reserveLimit: reserveLimitFor(state, side, config),
  };
}

/** Everything a side may do in the Counteraction Round, as it stands now. */
export function counteractionRequestFor(
  state: GameState,
  side: Side,
  config: PhaseConfig,
): CounteractionRequest {
  const reserveMoves: Record<string, ActionOption[]> = {};
  const fireOptions: Record<string, ActionOption[]> = {};

  for (const fe of forceElementsOf(state, side)) {
    const moves = reserveMoveOptionsFor(state, fe, config);
    if (moves.length > 0) reserveMoves[fe.id] = moves;
    const fires = counteractionFireOptionsFor(state, fe, config);
    if (fires.length > 0) fireOptions[fe.id] = fires;
  }

  return {
    side,
    turn: state.turn,
    view: projectForSide(state, side),
    reserveMoves,
    fireOptions,
  };
}

/**
 * Keep only intentions that were actually offered, within the command budget.
 *
 * THIS IS THE GUARD AGAINST A MODEL INVENTING A MOVE. `optionsFor` decides
 * what is legal; anything not in it is dropped rather than attempted, so a
 * hallucinated order becomes a unit that did nothing — visible in the log —
 * instead of a rule violation the engine tries to honour.
 */
export function validateOrders(
  orders: Orders,
  request: OrdersRequest,
): { accepted: Intent[]; rejected: { intent: Intent; reason: string }[] } {
  const accepted: Intent[] = [];
  const rejected: { intent: Intent; reason: string }[] = [];
  /** What each element has already been ordered to do, in order. */
  const givenTo = new Map<string, ActionOption[]>();

  for (const intent of orders.intents) {
    const options = request.optionsByElement[intent.actorId];
    if (!options) {
      rejected.push({ intent, reason: "no such element, or it had nothing to do" });
      continue;
    }

    const chosen = options.find((option) => option.id === intent.optionId);
    if (!chosen) {
      rejected.push({ intent, reason: "not a legal option for that element" });
      continue;
    }

    const already = givenTo.get(intent.actorId) ?? [];

    // MOVE THEN ENGAGE IS LEGAL, and this validator used to reject it.
    //
    // The rules allow an element to move and then fire in the same turn —
    // that is exactly how the firerMoved modifier became reachable, at a
    // penalty. The orders validator accepted one intent per element, which
    // was quietly more restrictive than the rules it was meant to enforce.
    //
    // Found by a model: GPT-5.2, told to be aggressive, ordered one platoon
    // to close AND engage. It was right and the validator was wrong.
    if (already.length > 0) {
      const onlyMovedSoFar = already.length === 1 && already[0].kind === "move";
      if (!onlyMovedSoFar) {
        rejected.push({ intent, reason: "already ordered this turn" });
        continue;
      }
      if (chosen.kind === "move") {
        rejected.push({ intent, reason: "an element may only move once a turn" });
        continue;
      }
    }

    // Command capacity counts ELEMENTS COMMITTED, not orders issued — the
    // same rule the activation loop uses. A move-then-fire pair costs one.
    if (already.length === 0 && givenTo.size >= request.activationBudget) {
      rejected.push({ intent, reason: "beyond this turn's command capacity" });
      continue;
    }

    givenTo.set(intent.actorId, [...already, chosen]);
    accepted.push(intent);
  }

  return { accepted, rejected };
}

export interface OrdersTurnResult {
  state: GameState;
  orders: Record<Side, Orders>;
  rejected: Record<Side, { intent: Intent; reason: string }[]>;
  /** What each side did with the Counteraction Round, where it was played. */
  counteraction?: Record<Side, CounteractionOrders>;
}

export interface OrdersTurnConfig extends PhaseConfig {
  commanders: Record<Side, OrdersCommander>;
}

/**
 * A turn that has been PLANNED but not yet fought.
 *
 * ⚠ WHY THE TURN IS CUT HERE AND NOWHERE ELSE.
 *
 * The play screen wants to show a commander's intentions on the map before
 * committing to them — "generate orders", look at it, "execute orders". That
 * needs a turn in two halves, and there is exactly one place the cut can go.
 *
 * Rally, initiative and sighting all run BEFORE anyone is asked for orders,
 * and they decide what is offered: who has rallied is who can act, and what
 * has been sighted is what can be shot at. So they belong to the planning
 * half, even though they are resolution rather than decision. Cutting before
 * them would mean asking a commander to plan against a board that had not yet
 * had its sighting resolved — a different and much worse game.
 *
 * Everything in here is what the execution half used to hold in local
 * variables. It is data rather than a closure so that a caller can hold it,
 * render it, and hand it back later — see lib/liveGame.ts.
 */
export interface PlannedTurn {
  turn: number;
  /**
   * The board as the commanders saw it when they planned.
   *
   * Post-rally, post-initiative, post-sighting and post-reserve-nomination.
   * NOT the state at the start of the turn, and the difference matters: this
   * is the state the accepted orders were checked against.
   */
  state: GameState;
  /** What each side was offered. The execution half looks options up in here. */
  requests: Record<Side, OrdersRequest>;
  /** What each side asked for, before validation. */
  orders: Record<Side, Orders>;
  /** Intentions that survived validation, in the order they were given. */
  accepted: Record<Side, Intent[]>;
  /** Intentions that were dropped, and why. Shown before execution, not after. */
  rejected: Record<Side, { intent: Intent; reason: string }[]>;
  /** Rules of engagement for the turn, indexed for the reaction step. */
  standing: StandingOrders;
  /** Who acts first. Kept because the C needs it as well as the execution loop. */
  initiativeWinner: Side;
}

/**
 * One turn, driven by orders.
 *
 * Same phases as `runTurn` and in the same sequence. Both sides are asked for
 * orders BEFORE either is executed, which is the point: neither commander
 * sees the other's intentions, so the turn is genuinely simultaneous in
 * planning even though resolution is sequenced by initiative.
 *
 * ⚠ KEPT AS A COMPOSITION OF THE TWO HALVES RATHER THAN REIMPLEMENTED.
 *
 * Every number in reports/ was measured through this function, and the
 * harness, the trial and the replay all still call it. Defining it as
 * `execute(plan(x))` means the plan/execute split cannot have changed what a
 * game does — there is no second copy of the sequence to drift from this one.
 * rules/orders.test.ts asserts the equivalence on a fixed seed.
 */
export async function runOrdersTurn(
  state: GameState,
  config: OrdersTurnConfig,
): Promise<OrdersTurnResult> {
  return executePlannedTurn(await planOrdersTurn(state, config), config);
}

/**
 * Ask both sides for their intentions, and stop.
 *
 * Resolves rally, initiative and sighting — which is what makes the options
 * legal — then asks each commander once and validates the answers. Nothing is
 * executed: no fire, no movement, no morale. The board it returns is the board
 * the orders were planned against.
 */
export async function planOrdersTurn(
  state: GameState,
  config: OrdersTurnConfig,
): Promise<PlannedTurn> {
  const turn = state.turn;

  // Command Sub-phase (5.1), step 1: RALLY (5.2). Both sequences of play run
  // it, because a mechanic that lives in one and not the other is how the two
  // drift apart — and this is the sequence the play screen and the LLM
  // commanders use, which is where "waiting to rally" was observed.
  const rallied = runRally({ ...state, phase: "command" }, config, turn);
  config.onStep?.({ turn, phase: "command", label: `Turn ${turn} begins`, state: rallied });

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
  config.onStep?.({ turn, phase: "arcAction", label: "Sighting", state: next });

  // Both sides plan against the SAME state, in parallel. Sequencing the two
  // calls would let the second commander plan against a board the first had
  // already changed, which is not what either of them was told.
  const requests: Record<Side, OrdersRequest> = {
    blue: ordersRequestFor(next, "blue", config),
    red: ordersRequestFor(next, "red", config),
  };
  const [blueOrders, redOrders] = await Promise.all([
    config.commanders.blue.planTurn(requests.blue),
    config.commanders.red.planTurn(requests.red),
  ]);

  const orders: Record<Side, Orders> = { blue: blueOrders, red: redOrders };

  // Rules of engagement, indexed for the reaction step. Declared once in the
  // orders phase rather than asked for per opportunity — see Orders.
  const standing: StandingOrders = {
    blue: new Map((blueOrders.standingOrders ?? []).map((o) => [o.actorId, o])),
    red: new Map((redOrders.standingOrders ?? []).map((o) => [o.actorId, o])),
  };

  // Command Sub-phase: who is held back (2.1.13). Nominated from the orders,
  // capped at a third of the force by nominateReserves.
  next = nominateReserves(next, "blue", config, blueOrders.reserves);
  next = nominateReserves(next, "red", config, redOrders.reserves);

  const checked = {
    blue: validateOrders(blueOrders, requests.blue),
    red: validateOrders(redOrders, requests.red),
  };

  return {
    turn,
    state: next,
    requests,
    orders,
    accepted: { blue: checked.blue.accepted, red: checked.red.accepted },
    rejected: { blue: checked.blue.rejected, red: checked.red.rejected },
    standing,
    initiativeWinner: initiative.winner,
  };
}

/**
 * Fight a turn that has already been planned.
 *
 * ⚠ THIS STILL ASKS THE COMMANDERS SOMETHING, and it has to.
 *
 * The Counteraction Round (7.2) is a second decision taken with second-round
 * information — that is its entire value, and it cannot be pre-planned. So
 * "execute" is not purely deterministic when the `counteraction` module is on:
 * it makes one further call per side, part-way through. See
 * CounteractionRequest.
 */
export async function executePlannedTurn(
  planned: PlannedTurn,
  config: OrdersTurnConfig,
): Promise<OrdersTurnResult> {
  const { turn, requests, orders, standing } = planned;
  let next = planned.state;
  const intents = intentsFrom(planned);

  // Executed alternately in initiative order, so having the initiative is
  // worth something: your orders land first.
  const order: Side[] =
    planned.initiativeWinner === "blue" ? ["blue", "red"] : ["red", "blue"];
  const queues: Record<Side, Intent[]> = {
    blue: [...planned.accepted.blue],
    red: [...planned.accepted.red],
  };

  while (queues.blue.length > 0 || queues.red.length > 0) {
    for (const side of order) {
      const intent = queues[side].shift();
      if (!intent) continue;

      const options = requests[side].optionsByElement[intent.actorId] ?? [];
      const chosen = options.find((option) => option.id === intent.optionId);
      if (!chosen) continue;

      // Re-check legality against the CURRENT state: an order given at the
      // start of the turn may have been made impossible by the other side's
      // orders landing first. That is the cost of planning blind, and it is
      // the mechanic, not a bug.
      const actor = next.forceElements[intent.actorId];
      if (!actor || actor.combatStrength <= 0) continue;
      if (hasMarker(actor, "fired") || hasMarker(actor, "held")) continue;
      const stillLegal = optionsFor(next, actor, config).some(
        (option) => option.id === intent.optionId,
      );
      if (!stillLegal) continue;

      config.log.append({
        type: "decision",
        turn,
        phase: "arcAction",
        rulesetId: config.ruleset.id,
        side,
        actorId: intent.actorId,
        question: "orders for the turn",
        options,
        chosenId: intent.optionId,
        chosenBy: config.commanders[side].kind,
        rationale: intent.rationale,
      });

      // 7.1, in the order it lists them. One enemy element may try to make
      // out what is activating (10.0), which can reveal it and so bring it
      // within reach of the Reactive Fire that follows.
      next = await attemptSightingInterruptLive(next, intent.actorId, side, config, turn, intents);

      // An assault runs its own sequence (9.3.4): Surprise, then Reactive
      // Fire from outside the objective and Defensive Fire from on it.
      if (chosen.kind === "assault") {
        next = await resolveAssaultActionLive(
          next,
          chosen,
          config,
          turn,
          standing,
          "actionReaction",
          "arcAction",
          intents,
        );
        next = applyEffects(next, [
          { kind: "marker", feId: intent.actorId, marker: "activated", added: true },
        ]);
        continue;
      }

      // THE R (7.1.3), BEFORE THE MOVE IT INTERRUPTS. An answer that Disrupts
      // or Breaks the mover stops the move happening at all.
      const interruptible = chosen.kind === "move";
      if (interruptible) {
        next = await reactiveFireLive(
          { ...next, phase: "arcReaction" },
          intent.actorId,
          side,
          config,
          turn,
          standing,
          "actionReaction",
          undefined,
          undefined,
          intents,
        );
        next = { ...next, phase: "arcAction" };
      }

      const stopped = interruptible && !mayProceed(next, intent.actorId);
      if (!stopped) next = await resolveMoveLive(next, chosen, config, turn, {}, intents);

      next = applyEffects(next, [
        { kind: "marker", feId: intent.actorId, marker: "activated", added: true },
        // An element shot to a standstill has still spent its activation.
        ...(stopped
          ? [{ kind: "marker" as const, feId: intent.actorId, marker: "held", added: true }]
          : []),
      ]);
    }
  }

  // ── The Counteraction Round (7.2). THE C ────────────────────────────────
  //
  // Asked for separately and AFTER the first round, because that is the only
  // thing that makes a reserve a reserve — see CounteractionRequest.
  const counteraction: Record<Side, CounteractionOrders> = {
    blue: { side: "blue", optionIds: [] },
    red: { side: "red", optionIds: [] },
  };

  if (config.ruleset.modules.counteraction) {
    const cRequests: Record<Side, CounteractionRequest> = {
      blue: counteractionRequestFor(next, "blue", config),
      red: counteractionRequestFor(next, "red", config),
    };
    const [blueC, redC] = await Promise.all([
      planCounteractionFor(config.commanders.blue, cRequests.blue),
      planCounteractionFor(config.commanders.red, cRequests.red),
    ]);
    counteraction.blue = blueC;
    counteraction.red = redC;

    // One queue per side, consumed in the commander's stated preference order.
    const queue: Record<Side, string[]> = {
      blue: [...blueC.optionIds],
      red: [...redC.optionIds],
    };

    next = await runCounteractionRound(
      next,
      config,
      turn,
      planned.initiativeWinner,
      standing,
      async (current, forSide, options, stage) => {
        const wanted = queue[forSide];
        const index = wanted.findIndex((id) => options.some((option) => option.id === id));

        if (index >= 0) {
          const [id] = wanted.splice(index, 1);
          return options.find((option) => option.id === id) ?? null;
        }

        // A reserve that has already committed to moving is not "passing" in
        // the sense 7.2.2 means — that rule is about declining to open fire.
        // Declining to shoot at the end of a move it chose to make would be
        // an odd way to lose a tank, so the follow-up falls back to the
        // heuristic rather than to nothing.
        //
        // With a decider configured, it is asked instead: this is exactly the
        // kind of in-the-moment call it exists for, and the heuristic was only
        // ever standing in because nobody had been asked.
        if (stage === "reserveFollowUp") {
          const live = await chooseOptionLive(
            current,
            forSide,
            options,
            "reserve at the end of its move: fire, assault, or nothing?",
            config,
            turn,
            true,
            intents,
          );
          return live === undefined ? bestOf(current, options) : live;
        }

        // Nothing the commander asked for is available: Pass. Final by rule.
        return null;
      },
      intents,
    );
  }

  next = runMoraleChecks({ ...next, phase: "cleanup" }, config, turn);
  next = endStaleMelees(next, config);
  config.onStep?.({ turn, phase: "cleanup", label: "Clean-up", state: next });

  const eliminated = (side: Side) =>
    forceElementsOf(next, side).filter((fe) => fe.combatStrength <= 0).length;

  return {
    state: {
      ...clearAllMarkers(
        { ...next, phase: "cleanup" },
        config.ruleset.modules.closeCombat,
      ),
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
    },
    orders,
    rejected: planned.rejected,
    counteraction,
  };
}

/**
 * What each commander said it meant to do, in the form a TacticalDecider reads.
 *
 * Only ACCEPTED intents are passed down: an element whose order was dropped
 * was not ordered to do anything, and telling its decider otherwise would be
 * inventing an intent the commander never successfully gave.
 */
function intentsFrom(planned: PlannedTurn): SideIntents {
  const intentFor = (side: Side) => {
    const byElement: Record<string, { summary: string; why?: string }> = {};
    for (const intent of planned.accepted[side]) {
      const option = planned.requests[side].optionsByElement[intent.actorId]?.find(
        (candidate) => candidate.id === intent.optionId,
      );
      if (!option) continue;
      const earlier = byElement[intent.actorId];
      byElement[intent.actorId] = {
        summary: earlier ? `${earlier.summary}, then ${option.summary}` : option.summary,
        why: intent.rationale ?? earlier?.why,
      };
    }
    return { plan: planned.orders[side].plan, orders: byElement };
  };
  return { blue: intentFor("blue"), red: intentFor("red") };
}

/**
 * Ask a commander to plan the Counteraction Round, or fall back.
 *
 * A commander that does not implement it passes, which is a legal and legible
 * way to play the round badly rather than a reason to crash a game in
 * progress. Same reasoning as an unreachable model holding position.
 */
async function planCounteractionFor(
  commander: OrdersCommander,
  request: CounteractionRequest,
): Promise<CounteractionOrders> {
  if (!commander.planCounteraction) {
    return {
      side: request.side,
      optionIds: [],
      plan: `${commander.name} does not plan the counteraction round — passed`,
    };
  }
  return commander.planCounteraction(request);
}

/**
 * The heuristic's pick from a list of counteraction options.
 *
 * Same priorities as everywhere else: finish what is already in contact,
 * otherwise shoot the nearest thing, otherwise move.
 */
function bestOf(state: GameState, options: ActionOption[]): ActionOption | null {
  if (options.length === 0) return null;

  const assault = options.find((option) => option.kind === "assault");
  if (assault) return assault;

  const fire = options.filter((option) => option.kind === "fire");
  const candidates = fire.length > 0 ? fire : options;

  let best: ActionOption | null = null;
  let bestRange = Number.POSITIVE_INFINITY;
  for (const option of candidates) {
    const actor = option.actorId ? state.forceElements[option.actorId] : undefined;
    const target = option.targetId ? state.forceElements[option.targetId] : undefined;
    if (!actor || !target) continue;
    const range = distanceM(actor.position, target.position);
    if (range < bestRange) {
      bestRange = range;
      best = option;
    }
  }
  return best ?? candidates[0];
}

/**
 * An orders commander that plans with the existing heuristic scoring.
 *
 * Exists so the play screen is usable before any model is published, and as
 * the baseline an LLM has to beat. It orders every element it can, best
 * option first, until the command budget runs out.
 *
 * ⚠ IT NOW DECLARES ROE AND NOMINATES A RESERVE, AND IT HAS TO.
 *
 * While it did neither, every heuristic game — which is every game in
 * reports/ — ran on `defaultEngage` with nothing in reserve. The mechanics
 * were therefore exercised at exactly one setting, by nobody's choice, and a
 * mechanic that only a model can vary cannot be A/B'd against the baseline.
 * What it does here is deliberately simple, because a yardstick whose
 * behaviour nobody can predict is not one.
 */
export function heuristicOrdersCommander(
  side: Side,
  /**
   * Keep the force together so it can mass (9.2.1). See FormationPreference.
   *
   * Optional so that an experiment can run the scattering version as a
   * control — which is the only way to show that keeping formation is what
   * changed a result, rather than something else that moved at the same time.
   */
  formation?: FormationPreference,
): OrdersCommander {
  return {
    kind: "heuristic",
    name: formation ? `formation-orders-${side}` : `heuristic-orders-${side}`,
    async planTurn(request) {
      const intents: Intent[] = [];

      // ONE OBJECTIVE PER GROUP OF NEIGHBOURS.
      //
      // This is the whole of the formation logic and it is deliberately
      // crude. Elements that are already co-located are clustered, and every
      // element in a cluster is pointed at the SAME enemy — the one nearest
      // the cluster as a whole. Left to themselves they each chose their own
      // nearest contact, which pulled a troop apart within two turns and put
      // Combined Fire permanently out of reach.
      const focus = formation ? clusterFocus(request, formation) : new Map<string, string>();

      for (const [actorId, options] of Object.entries(request.optionsByElement)) {
        // Same preference order the per-activation heuristic uses: engage what
        // you can reach, otherwise close, otherwise hold.
        const fire = options.filter((option) => option.kind === "fire");
        const assault = options.filter((option) => option.kind === "assault");
        const move = options.filter((option) => option.kind === "move");

        // Prefer going in, or firing, TOGETHER (9.2.1, 9.3). The fire table
        // is spaced to reward concentration; a baseline that always engaged
        // singly would never test that.
        const combinedAssault = assault.find((option) => (option.actorIds?.length ?? 1) > 1);
        const combinedFire = fire.filter((option) => (option.actorIds?.length ?? 1) > 1);

        // Where this element's neighbours are going. Anything aimed at the
        // cluster's chosen enemy is preferred over the same kind of action
        // aimed somewhere else.
        const aimedAt = focus.get(actorId);
        const onFocus = (candidates: ActionOption[]) =>
          aimedAt ? candidates.filter((option) => option.targetId === aimedAt) : [];

        const best =
          onFocus(assault).find((option) => (option.actorIds?.length ?? 1) > 1) ??
          combinedAssault ??
          onFocus(assault)[0] ??
          assault[0] ??
          nearest(request, actorId, onFocus(combinedFire)) ??
          nearest(request, actorId, combinedFire) ??
          nearest(request, actorId, onFocus(fire)) ??
          nearest(request, actorId, fire) ??
          onFocus(move)[0] ??
          move[0] ??
          options.find((option) => option.kind === "hold");
        if (best) intents.push({ actorId, optionId: best.id, rationale: "heuristic orders" });
      }

      // ── Command capacity (5.1) ──────────────────────────────────────────
      //
      // ⚠ THIS COMMANDER USED TO IGNORE THE BUDGET ENTIRELY, AND THE
      // VALIDATOR CLEANED UP AFTER IT BY TRUNCATING.
      //
      // It issued an intent for every element it could see, `validateOrders`
      // accepted the first `activationBudget` of them and refused the rest
      // with "beyond this turn's command capacity". The surviving orders were
      // therefore whichever elements happened to come first out of
      // `Object.entries` — which is insertion order, which is the order the
      // force list happens to declare them in.
      //
      // That is the opposite of the mechanic. `activationBudget` exists so
      // that "a side with more sub-units than command capacity has to CHOOSE
      // which ones fight, which is a DECISION rather than a die roll". Nobody
      // was choosing. And because this is the commander every sweep arm runs,
      // the `commandActivations` row was measuring where the clip landed
      // rather than what a commander preferred.
      //
      // ⚠ ONLY WHEN THE BUDGET IS FINITE. Execution walks `accepted` IN
      // ORDER, alternating sides, so re-sorting changes outcomes even when
      // nothing is dropped. Sorting unconditionally would move every number
      // in reports/ for a module that was not even switched on. With the
      // module off the budget is Infinity and this is a no-op, so the only
      // arm that moves is the one that was wrong.
      if (Number.isFinite(request.activationBudget)) {
        intents.splice(
          0,
          intents.length,
          ...rankByCommandValue(intents, request).slice(0, request.activationBudget),
        );
      }

      // ROE: anything being ordered to close is going to be busy, so it holds
      // its overwatch to short range; anything with nothing to do this turn is
      // watching, so it engages whatever it can reach. That is the simplest
      // rule that makes the setting depend on the situation rather than being
      // a constant, which is what the sweep needs in order to see it at all.
      const ordered = new Set(intents.map((intent) => intent.actorId));
      const standingOrders: StandingOrder[] = request.view.own.map((fe) => ({
        actorId: fe.id,
        engage: ordered.has(fe.id) ? "withinShortRange" : "always",
      }));

      // Reserve: the rearmost eligible elements, up to the cap. Held back
      // means not ordered forward, so only unordered elements are offered.
      const reserves = request.view.own
        .filter((fe) => !ordered.has(fe.id))
        .slice(0, Math.max(0, request.reserveLimit))
        .map((fe) => fe.id);

      return {
        side: request.side,
        intents,
        standingOrders,
        reserves,
        plan: "engage what can be reached, close otherwise",
      };
    },

    async planCounteraction(request) {
      // Commit every reserve that can move, then take every second-round shot
      // on offer. It never passes, which is the same documented weakness the
      // per-activation heuristic has and the same thing a model might beat.
      const optionIds = [
        ...Object.values(request.reserveMoves).map((options) => options[0]?.id),
        ...Object.values(request.fireOptions).map((options) => options[0]?.id),
      ].filter((id): id is string => id != null);

      return {
        side: request.side,
        optionIds,
        plan: "commit the reserve, then fire with anything that has not",
      };
    },
  };
}

/**
 * Order intents by how much they are worth spending command capacity on.
 *
 * ⚠ THE RANKING IS THE DECISION THE MECHANIC EXISTS TO FORCE, so it is
 * deliberately simple enough to predict and argue with rather than tuned.
 * Four rules, in order:
 *
 *  1. ACTING BEATS NOT ACTING. A `hold` costs a full activation — both this
 *     path and the per-activation loop mark the element `activated` — and
 *     buys nothing. Spending one of two activations on standing still is the
 *     single worst thing a constrained commander can do, so holds sort last
 *     and are the first thing dropped.
 *  2. TOGETHER BEATS SEPARATELY. The fire columns are spaced so that
 *     concentration is the correct play; an activation that joins a combined
 *     action buys more table than one that acts alone.
 *  3. SHOOTING BEATS MOVING, because a move that is not followed by fire
 *     changes nothing this turn, and under a tight budget there may be no
 *     next turn for that element.
 *  4. TIES KEEP THEIR ORIGINAL ORDER. A stable sort, so that where the
 *     ranking has no opinion the behaviour is the one already measured.
 *
 * This is a yardstick, not a general — the same standard the rest of this
 * commander is held to. A model is expected to beat it.
 */
function rankByCommandValue(intents: Intent[], request: OrdersRequest): Intent[] {
  const value = (intent: Intent): number => {
    const option = request.optionsByElement[intent.actorId]?.find(
      (candidate) => candidate.id === intent.optionId,
    );
    if (!option) return 0;

    const combined = (option.actorIds?.length ?? 1) > 1;
    switch (option.kind) {
      case "assault":
        return combined ? 6 : 5;
      case "fire":
        return combined ? 4 : 3;
      case "move":
        return 2;
      default:
        // hold and pass: an activation spent achieving nothing.
        return 1;
    }
  };

  return intents
    .map((intent, index) => ({ intent, index, value: value(intent) }))
    .sort((a, b) => b.value - a.value || a.index - b.index)
    .map((entry) => entry.intent);
}

/**
 * Which enemy each element should be pointed at, so neighbours point at the
 * same one.
 *
 * Greedy single-link clustering at the co-location distance: walk the force,
 * and put each element with the first cluster it is already within range of.
 * Crude on purpose — a proper clustering would be less predictable and this
 * is a yardstick, not a general.
 *
 * Returns element id to target id. Elements with nothing in sight are absent.
 */
function clusterFocus(
  request: OrdersRequest,
  formation: FormationPreference,
): Map<string, string> {
  const clusters: (typeof request.view.own)[] = [];

  for (const fe of request.view.own) {
    const existing = clusters.find((cluster) =>
      cluster.some((member) => distanceM(member.position, fe.position) <= formation.coLocatedM),
    );
    if (existing) existing.push(fe);
    else clusters.push([fe]);
  }

  const focus = new Map<string, string>();

  for (const cluster of clusters) {
    const centre = {
      lat: cluster.reduce((sum, fe) => sum + fe.position.lat, 0) / cluster.length,
      lng: cluster.reduce((sum, fe) => sum + fe.position.lng, 0) / cluster.length,
    };

    // The enemy nearest the cluster as a whole, not nearest any one member —
    // which is the difference between a troop and three tanks.
    let target: string | undefined;
    let bestRange = Number.POSITIVE_INFINITY;
    for (const contact of request.view.contacts) {
      const range = distanceM(centre, contact.position);
      if (range < bestRange) {
        bestRange = range;
        target = contact.id;
      }
    }

    if (target) for (const fe of cluster) focus.set(fe.id, target);
  }

  return focus;
}

/** The closest target among a set of fire options. */
function nearest(
  request: OrdersRequest,
  actorId: string,
  options: ActionOption[],
): ActionOption | undefined {
  const actor = request.view.own.find((fe) => fe.id === actorId);
  if (!actor || options.length === 0) return undefined;

  let best: ActionOption | undefined;
  let bestRange = Number.POSITIVE_INFINITY;
  for (const option of options) {
    const target = request.view.contacts.find((c) => c.id === option.targetId);
    if (!target) continue;
    const range = distanceM(actor.position, target.position);
    if (range < bestRange) {
      bestRange = range;
      best = option;
    }
  }
  return best ?? options[0];
}

/** Who is still in the fight. Used by the live game to decide when to stop. */
export function sidesAlive(state: GameState): Record<Side, boolean> {
  return {
    blue: forceElementsOf(state, "blue").some((fe) => fe.combatStrength > 0),
    red: forceElementsOf(state, "red").some((fe) => fe.combatStrength > 0),
  };
}

/** The side still standing, or null if both or neither are. */
export function winnerOf(state: GameState): Side | null {
  const alive = sidesAlive(state);
  if (alive.blue && alive.red) return null;
  if (alive.blue) return "blue";
  if (alive.red) return "red";
  return null;
}

export { opposing };
