// ── bgws/rules/events.ts ───────────────────────────────────────────────────
// The log. It is the product, not the exhaust.
//
// A wargame's output is not who won. It is the record of what was decided,
// under what information, and what happened as a result — which is what an
// after-action review discusses and what tells us whether a rule earned its
// place.
//
// So the log holds TWO kinds of entry, and the second is the one most engines
// forget:
//
//   ResolutionEvent — a die roll and its consequences. Auditable: every
//                     modifier is named, so "why did that hit?" has an answer.
//   DecisionEvent   — what the options were and which was taken. Without this
//                     we can measure that a rule changed an OUTCOME, but not
//                     whether it changed a CHOICE, and the second is the
//                     interesting one.
//
// Both carry the ruleset they were produced under. A log that cannot say which
// rules applied is not evidence.

import type { DiceRoll } from "./dice";
import type { Phase, Side } from "../lib/state";
import type { PlannedRoute } from "../lib/routePlan";

/** A named modifier. The name is not decoration: it is how a result is explained. */
export interface Modifier {
  source: string;
  value: number;
}

export type ResolutionKind =
  | "directFire"
  | "indirectFire"
  | "assault"
  | "sighting"
  | "morale"
  | "rally"
  | "initiative"
  | "move"
  | "elimination";

/** What a resolution did to the game. Applied by the engine, replayable. */
export type StateDelta =
  | { kind: "combatStrength"; feId: string; delta: number }
  | { kind: "morale"; feId: string; to: string }
  | { kind: "marker"; feId: string; marker: string; added: boolean }
  | { kind: "position"; feId: string; lat: number; lng: number }
  /** Which way an element is pointing, from the bearing of its last move. */
  | { kind: "facing"; feId: string; to: number }
  /** A march set, trimmed as it is walked, or abandoned. See lib/routePlan.ts. */
  | { kind: "route"; feId: string; route: PlannedRoute | null }
  | { kind: "sighting"; viewer: Side; feId: string; to: string }
  /**
   * A counter flipped to or from its "?" side (2.1.15).
   *
   * Nothing could flip one before, so a Concealed FE stayed Concealed for the
   * whole game however much it fired — which made concealment a starting
   * condition rather than a state, and made the Attempt Sighting interrupt
   * pointless because success could not be recorded.
   */
  | { kind: "concealed"; feId: string; to: boolean }
  /** Smoke laid on an area of the map (9.2.2.4). Not attached to an element. */
  | { kind: "smoke"; id: string; lat: number; lng: number; turn: number }
  | { kind: "eliminated"; feId: string }
  | { kind: "initiative"; side: Side };

export interface ResolutionEvent {
  type: "resolution";
  seq: number;
  turn: number;
  phase: Phase;
  kind: ResolutionKind;
  rulesetId: string;
  actorIds: string[];
  targetIds: string[];
  roll?: DiceRoll;
  modifiers: Modifier[];
  /** Roll plus modifiers — the number actually looked up. */
  total?: number;
  /** Which table was consulted, by name, for traceability. */
  table?: string;
  /** The table's answer, in its own vocabulary. */
  result: string;
  effects: StateDelta[];
  /** Prose for the turn report. Generated from the fields, never instead of them. */
  narrative?: string;
}

/** One option that was available to a commander at a decision point. */
export interface DecisionOption {
  id: string;
  summary: string;
  /** Whatever the commander used to rank it, for later analysis. */
  score?: number;
}

export interface DecisionEvent {
  type: "decision";
  seq: number;
  turn: number;
  phase: Phase;
  rulesetId: string;
  side: Side;
  /** The force element being commanded, where the decision is about one. */
  actorId?: string;
  question: string;
  options: DecisionOption[];
  chosenId: string;
  /** Who chose: a person, the scripted bot, a language model, or a decision model. */
  chosenBy: "human" | "heuristic" | "llm" | "jev";
  /** The model's stated reasoning, where there was one. Never trusted as fact. */
  rationale?: string;
  /**
   * The decision model's probability for each option, by option id.
   *
   * Kept because a choice taken at 0.51 and one taken at 0.97 are different
   * facts about a commander, and only the second is a conviction.
   */
  probabilities?: Record<string, number>;
  /** How concentrated those probabilities were, 0–1. */
  confidence?: number;
  /** Round trip to the model, in milliseconds. */
  latencyMs?: number;
  /** What the call cost, in US dollars, where the provider said. */
  costUsd?: number;
  /** Set when the model could not decide and a rule did instead. */
  fallback?: "timeout" | "error" | "lowConfidence" | "vetoed";
}

export type GameEvent = ResolutionEvent | DecisionEvent;

/**
 * An append-only log with a sequence number.
 *
 * Deliberately not an array the engine pushes to directly: the sequence number
 * has to be authoritative, and an event that skipped it would break replay
 * ordering in a way nothing would notice until an after-action review looked
 * wrong.
 */
/**
 * An event before the log has numbered it.
 *
 * Written as a union of two Omits rather than `Omit<GameEvent, "seq">`, which
 * looks equivalent and is not: Omit over a union collapses it to the keys the
 * members share, so the discriminated union disappears and `side` stops
 * existing. tsc caught it; the tests did not, because at runtime the object
 * was always fine.
 */
export type NewEvent = Omit<ResolutionEvent, "seq"> | Omit<DecisionEvent, "seq">;

export class EventLog {
  private events: GameEvent[] = [];

  append(event: Omit<ResolutionEvent, "seq">): ResolutionEvent;
  append(event: Omit<DecisionEvent, "seq">): DecisionEvent;
  append(event: NewEvent): GameEvent {
    const withSeq = { ...event, seq: this.events.length } as GameEvent;
    this.events.push(withSeq);
    return withSeq;
  }

  all(): readonly GameEvent[] {
    return this.events;
  }

  /**
   * Drop everything logged after `length`, returning the log to an earlier
   * point.
   *
   * ⚠ THE ONLY LEGITIMATE CALLER IS DISCARDING A PLAN. Planning a turn
   * resolves rally, initiative and sighting before anyone is asked for
   * orders, and those append real events. Throwing the plan away leaves the
   * board untouched but WOULD leave those entries behind, so the log would
   * describe a turn beginning two or three times and an initiative roll that
   * never governed anything. Rewinding keeps the log describing what actually
   * happened.
   *
   * Safe for `seq` because append numbers events by position: after a rewind
   * to n, the next event is seq n again, with no gap and no duplicate.
   *
   * ⚠ DO NOT USE IT TO UNDO A FOUGHT TURN. Events are the only record that a
   * shot was taken; state has already moved on and cannot be rewound with it.
   */
  rewindTo(length: number): void {
    const target = Math.max(0, Math.floor(length));
    // Never grows the log -- there is nothing to grow it back with, and a
    // caller asking for that has misunderstood something.
    if (target >= this.events.length) return;
    this.events.length = target;
  }

  ofTurn(turn: number): GameEvent[] {
    return this.events.filter((e) => e.turn === turn);
  }

  resolutions(): ResolutionEvent[] {
    return this.events.filter((e): e is ResolutionEvent => e.type === "resolution");
  }

  decisions(): DecisionEvent[] {
    return this.events.filter((e): e is DecisionEvent => e.type === "decision");
  }

  /** Serialisable, for export and for a replay's expected output. */
  toJSON(): GameEvent[] {
    return this.events;
  }
}

/**
 * Did a named modifier ever actually change anything?
 *
 * The experiment harness's blunt instrument: a DRM that never appears, or
 * always appears with the same value on both sides, is a rule with nothing to
 * say. Counting it is how a mechanic argues for its place.
 */
export function modifierUsage(log: EventLog): Record<string, { count: number; values: number[] }> {
  const usage: Record<string, { count: number; values: number[] }> = {};
  for (const event of log.resolutions()) {
    for (const modifier of event.modifiers) {
      const entry = usage[modifier.source] ?? { count: 0, values: [] };
      entry.count += 1;
      if (!entry.values.includes(modifier.value)) entry.values.push(modifier.value);
      usage[modifier.source] = entry;
    }
  }
  return usage;
}

/**
 * How often a commander had a real choice.
 *
 * A decision point with one option is not a decision, and a game made of them
 * is a cutscene. This is the measure of whether the rules produce agency.
 */
export function decisionBreadth(log: EventLog): {
  total: number;
  withRealChoice: number;
  meanOptions: number;
} {
  const decisions = log.decisions();
  if (decisions.length === 0) return { total: 0, withRealChoice: 0, meanOptions: 0 };
  const withRealChoice = decisions.filter((d) => d.options.length > 1).length;
  const meanOptions =
    decisions.reduce((sum, d) => sum + d.options.length, 0) / decisions.length;
  return { total: decisions.length, withRealChoice, meanOptions };
}
