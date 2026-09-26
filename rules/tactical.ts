// ── bgws/rules/tactical.ts ─────────────────────────────────────────────────
// Decisions taken AT THE MOMENT, inside the resolution of a turn.
//
// A commander plans the turn. Inside it, the engine used to settle every
// in-the-moment choice by a rule declared in advance: whether an element in
// overwatch shoots at the thing crossing its arc (`willReact` over a
// StandingEngagement), and whether a mover presses on through contact
// (`onContact`, fixed on the option before the move started). Both comments
// say why — a model call per opportunity was too slow and too expensive.
//
// A decision model like Jev changes that arithmetic: one round trip of
// 70–500 ms and a fraction of a cent answers every reactor at once. So these
// moments become questions, put to whoever is deciding for that side, with the
// facts as they stand when the moment actually arises.
//
// THE CONTRACT IS THE SAME ONE THE COMMANDERS HAVE
// -------------------------------------------------
// The engine works out who COULD react — range, line of sight, ammunition,
// markers, weapon, fog of war — and a decider only chooses among those. It
// cannot add a reactor the rules did not offer, and whatever it returns that
// is not on the list is dropped. So a confused decider costs a shot, never a
// rule.
//
// ABSENT MEANS TODAY'S RULES
// --------------------------
// `GameConfig.tactical` is optional, per side. With nothing configured the
// engine takes exactly the path it always took — same rolls, same order, same
// log — which is what keeps the calibration and replay tests meaningful.

import type { ActionOption } from "./commander";
import type { DecisionOption } from "./events";
import type { StandingEngagement } from "./ruleset";
import type { GameState, Side } from "../lib/state";
import type { PhaseConfig, ArcRound } from "./turnLoop";

/**
 * What the side's commander said it wanted, for a decider to act within.
 *
 * Present in the orders sequence, where a commander states a plan; absent in
 * the activation sequence, where nobody does.
 */
export interface CommanderIntent {
  /** The side's plan for the turn, in the commander's words. */
  plan?: string;
  /** What each element was ordered to do this turn, by element id. */
  orders?: Record<string, { summary: string; why?: string }>;
}

/** One element the rules say could answer this action. */
export interface ReactionCandidate {
  reactorId: string;
  rangeM: number;
  /** The capability it would fire with. */
  capability: string;
  /** Its declared engagement rule, or the scenario default. */
  engage: StandingEngagement;
  /** What the declared rule alone would have done. The fallback. */
  ruleSaysReact: boolean;
}

export interface ReactionMoment {
  state: GameState;
  config: PhaseConfig;
  turn: number;
  round: ArcRound;
  /** The side that is REACTING — the decider being asked. */
  side: Side;
  /** The enemy element whose action is being answered. */
  actorId: string;
  /** Whether that element is shooting at this side (an assault or a shot). */
  wasFiredUpon: boolean;
  /** Eligible reactors, in the order the rules would take them. */
  candidates: ReactionCandidate[];
  /** At most this many may fire. */
  maxReactors: number;
  intent?: CommanderIntent;
}

/**
 * Why a move has stopped to ask.
 *
 *   contact    it has just sighted an enemy it had not seen (7.1.3)
 *   underFire  it was shot at as it set off, and can still go on
 *   exposed    it is walking into an identified enemy's sight and range
 *   setback    a friend close by has been destroyed or broken this turn
 */
export type MoveTrigger = "contact" | "underFire" | "exposed" | "setback";

export interface ContactMoment {
  state: GameState;
  config: PhaseConfig;
  turn: number;
  /** The moving side — the decider being asked. */
  side: Side;
  /** The move that has just been cut short. */
  option: ActionOption;
  actorId: string;
  /** Enemies this move has just made contact with. */
  newContacts: string[];
  /** Metres still to go to the ordered destination. */
  remainingM: number;
  /** What the commander pre-committed to on the option, if anything. */
  preferred: "halt" | "press";
  /** What stopped the move. Contact, unless one of the other triggers fired. */
  trigger: MoveTrigger;
  /** The trigger in words: "fired on by R1", "enters R2's sight at 1,400 m". */
  detail?: string;
  /** The nearest cover it could break for from here, if there is any. */
  cover?: { metres: number; ground: string };
  intent?: CommanderIntent;
}

/**
 * How a decision was reached, for the log.
 *
 * Carried back to the engine rather than logged by the decider, so the
 * decision lands in the event log in sequence with the resolutions it caused.
 */
export interface TacticalTrace {
  actorId?: string;
  question: string;
  options: DecisionOption[];
  chosenId: string;
  /**
   * Who actually decided. "llm" when Jev was unsure and the question was
   * escalated to the side's language model; "heuristic" when the rule did.
   */
  chosenBy: "heuristic" | "jev" | "llm";
  rationale?: string;
  /** Per-option probability, where the decider has one. */
  probabilities?: Record<string, number>;
  confidence?: number;
  latencyMs?: number;
  /** Cost of the request, carried on ONE trace per request so totals add up. */
  costUsd?: number;
  /** Set when the decider could not answer and the rule decided instead. */
  fallback?: "timeout" | "error" | "lowConfidence" | "vetoed";
  /** A few words for a marker beside the counter: "holds fire", "presses on". */
  mark?: string;
}

/**
 * One element that has orders left to carry out this turn, as it stands now.
 *
 * `options` are generated from the CURRENT board, not the one the orders were
 * written against — that is the point of asking at the moment.
 */
export interface ActivationCandidate {
  actorId: string;
  options: ActionOption[];
  /** The option the commander ordered, if it is still on offer. */
  orderedOptionId?: string;
  /** The order as given, in words, even when it is no longer possible. */
  orderedSummary?: string;
}

/**
 * "Whose turn is it, and what do they do?" — the activation, decided when it
 * comes rather than when the turn was planned.
 *
 * The commander's plan fixes WHICH elements are committed (command capacity
 * is spent at planning). The decider chooses the order they act in and may
 * adapt what each does to what has happened since — a different target, a
 * halt, a hold — from the options the rules offer now.
 */
export interface ActivationMoment {
  state: GameState;
  config: PhaseConfig;
  turn: number;
  side: Side;
  candidates: ActivationCandidate[];
  intent?: CommanderIntent;
}

/** Who on the watching side attempts to sight a Concealed element that activated. */
export interface ObserverMoment {
  state: GameState;
  config: PhaseConfig;
  turn: number;
  /** The WATCHING side — the decider being asked. */
  side: Side;
  /** The Concealed enemy element that is activating. */
  actorId: string;
  /** Who has a line to it, nearest first — the rule would take the first. */
  observers: { observerId: string; rangeM: number; recce: boolean }[];
  intent?: CommanderIntent;
}

/**
 * A choice among options the engine generated, with nothing pre-declared.
 *
 * Used where the engine used to pick by heuristic because nobody had been
 * asked: what a reserve does at the end of its move, when its commander did
 * not say.
 */
export interface OptionMoment {
  state: GameState;
  config: PhaseConfig;
  turn: number;
  side: Side;
  question: string;
  options: ActionOption[];
  /** Whether declining all of them is a legal answer. */
  allowPass: boolean;
  intent?: CommanderIntent;
}

export interface ReactionVerdict {
  /** Reactors to fire, best first. Anything not a candidate is ignored. */
  reactorIds: string[];
  traces: TacticalTrace[];
}

export interface ContactVerdict {
  /** Carry on to the destination. */
  press: boolean;
  /** Break off to the nearest cover instead (only when some was offered). */
  cover?: boolean;
  traces: TacticalTrace[];
}

/**
 * Whoever makes a side's in-the-moment calls.
 *
 * Both methods are async because a model is. Both must RESOLVE — never
 * reject — and fall back to the rule themselves if they cannot answer, since
 * the engine is in the middle of an action when it asks.
 */
export interface TacticalDecider {
  readonly name: string;
  decideReactions(moment: ReactionMoment): Promise<ReactionVerdict>;
  decideContact(moment: ContactMoment): Promise<ContactVerdict>;
  /** Optional: absent means the nearest observer looks, as the rule reads it. */
  chooseObserver?(
    moment: ObserverMoment,
  ): Promise<{ observerId: string | null; traces: TacticalTrace[] }>;
  /**
   * Optional: choose the next activation from the commander's remaining
   * orders. `undefined` means "carry out the next order as written".
   */
  chooseActivation?(
    moment: ActivationMoment,
  ): Promise<{ pick?: { actorId: string; optionId: string }; traces: TacticalTrace[] }>;
  /**
   * Optional: ask ahead about reactions that are likely to be needed, in one
   * request, so the answers are waiting when the moment comes. A moment that
   * turns out differently from the prefetched one is simply asked again.
   */
  prefetchReactions?(moments: ReactionMoment[]): Promise<void>;
  /** Optional: absent means the engine's heuristic picks, as before. */
  /**
   * `optionId` is the choice; `null` is a deliberate pass; `undefined` means
   * no opinion (unreachable, unsure), and the engine's own default decides.
   */
  chooseOption?(
    moment: OptionMoment,
  ): Promise<{ optionId: string | null | undefined; traces: TacticalTrace[] }>;
}

/**
 * The rules as they were, expressed as a decider.
 *
 * Useful as a fallback inside a smarter decider, and as a control: a game
 * with `ruleDecider` configured must differ from one with nothing configured
 * only in the continuation walk after a contact (see resolveMoveLive).
 */
export const ruleDecider: TacticalDecider = {
  name: "rules",
  async decideReactions(moment) {
    return {
      reactorIds: moment.candidates
        .filter((candidate) => candidate.ruleSaysReact)
        .map((candidate) => candidate.reactorId),
      traces: [],
    };
  },
  async decideContact(moment) {
    // Only contact was ever a reason to stop. For the others the rule is what
    // the engine always did: carry on.
    return {
      press: moment.trigger === "contact" ? moment.preferred === "press" : true,
      traces: [],
    };
  },
};
