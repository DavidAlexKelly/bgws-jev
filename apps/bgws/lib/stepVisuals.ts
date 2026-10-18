// ── bgws/lib/stepVisuals.ts ────────────────────────────────────────────────
// Turning one step of a turn into something you can look at.
//
// Two jobs, both about the same complaint: a timeline that says "B1 fires on
// R1" and stops there is a claim without its evidence. The same sentence
// covers a certainty botched and a long chance that landed, and the difference
// between those two is the entire teaching value of a wargame.
//
//   describeRoll  — the dice, the modifiers, the total and what it beat.
//   fireLinesFor  — who shot at whom, and what happened, as map geometry.
//
// Kept out of the screen so both can be tested without a browser, and so a
// second surface (a replay viewer, a report) can draw the same thing without
// copying the interpretation.

import type { LatLng } from "./board";
import type { GameState, Side } from "./state";
import type { ResolutionEvent } from "../rules/events";
import type { TurnStep } from "../rules/turnLoop";

/**
 * What a shot did, read from its EFFECTS rather than from its label.
 *
 * ⚠ DELIBERATELY NOT PARSED FROM THE NARRATIVE. Result strings are written for
 * people and change when someone improves the wording; effects are what the
 * engine actually applied. A picture drawn from the prose would drift from the
 * game silently, which is the one failure a visualisation must not have.
 */
export type FireOutcome = "destroyed" | "hit" | "suppressed" | "noEffect";

/**
 * What a shot did to ONE target.
 *
 * `destroyed` is read from the state the step recorded, not from the effects:
 * the effects say "-3 Combat Strength", and whether that was the last three
 * points is a fact about the board rather than about the shot. Taking it from
 * the recorded state means the picture agrees with the game by construction.
 */
export function outcomeOf(
  event: ResolutionEvent,
  state?: GameState,
  targetId?: string,
): FireOutcome {
  const effects = event.effects ?? [];
  const hurt = effects.some(
    (effect) => effect.kind === "combatStrength" && effect.delta < 0,
  );

  if (targetId && state) {
    const target = state.forceElements[targetId];
    const eliminated =
      effects.some((effect) => effect.kind === "eliminated" && effect.feId === targetId) ||
      (hurt && target !== undefined && target.combatStrength <= 0);
    if (eliminated) return "destroyed";
  }

  if (hurt) return "hit";
  if (effects.some((effect) => effect.kind === "morale")) return "suppressed";
  return "noEffect";
}

/** One shot, assault or mortar mission, as a line on the map. */
export interface FireLine {
  id: string;
  from: LatLng;
  to: LatLng;
  /** The firing side, so the line can be drawn in its colour. */
  side: Side;
  kind: ResolutionEvent["kind"];
  outcome: FireOutcome;
  /** Short words for the map: "2 hits", "suppressed", "no effect". */
  note: string;
}

const ENGAGEMENTS: ReadonlySet<string> = new Set([
  "directFire",
  "indirectFire",
  "assault",
]);

function hitsIn(event: ResolutionEvent): number {
  return (event.effects ?? [])
    .filter((effect) => effect.kind === "combatStrength" && effect.delta < 0)
    .length;
}

/** Plain words for what happened at the receiving end. */
export function describeOutcome(
  event: ResolutionEvent,
  state?: GameState,
  targetId?: string,
): string {
  const outcome = outcomeOf(event, state, targetId);
  if (outcome === "destroyed") return "destroyed";
  if (outcome === "hit") {
    const hits = hitsIn(event);
    return hits > 1 ? `${hits} hits` : "hit";
  }
  if (outcome === "suppressed") return "suppressed";
  return "no effect";
}

/**
 * The engagements in this step, as lines between the elements involved.
 *
 * `visible` is the set of elements the current viewpoint may see. A line is
 * drawn only when BOTH ends are visible: half a line leaking out of the fog
 * would tell a player where an unsighted enemy is, which is exactly what the
 * fog is for. The umpire's view passes every id and sees everything.
 */
export function fireLinesFor(
  step: Pick<TurnStep, "state" | "events">,
  visible: ReadonlySet<string>,
): FireLine[] {
  const lines: FireLine[] = [];
  const positionOf = (state: GameState, id: string): LatLng | null =>
    state.forceElements[id]?.position ?? null;

  for (const [index, event] of (step.events ?? []).entries()) {
    if (!ENGAGEMENTS.has(event.kind)) continue;

    for (const actorId of event.actorIds) {
      const from = positionOf(step.state, actorId);
      if (!from || !visible.has(actorId)) continue;

      for (const targetId of event.targetIds) {
        const to = positionOf(step.state, targetId);
        if (!to || !visible.has(targetId)) continue;

        lines.push({
          id: `${index}:${actorId}:${targetId}`,
          from,
          to,
          side: step.state.forceElements[actorId].side,
          kind: event.kind,
          // Per TARGET, not per event: one burst can destroy one element and
          // merely shake another, and a single colour for both would be a lie
          // about at least one of them.
          outcome: outcomeOf(event, step.state, targetId),
          note: describeOutcome(event, step.state, targetId),
        });
      }
    }
  }

  return lines;
}

/** A die face as its pip character, because a wargame should show its dice. */
const PIPS = ["\u2680", "\u2681", "\u2682", "\u2683", "\u2684", "\u2685"];

export function pipsFor(dice: readonly number[]): string {
  return dice.map((die) => PIPS[die - 1] ?? String(die)).join(" ");
}

/**
 * The roll, the modifiers and the total, in one line.
 *
 * Every modifier is NAMED. That is the whole point of the modifier list in the
 * event: a total with no breakdown tells a player they lost without telling
 * them what beat them, and the named DRMs are how a rule teaches.
 */
export function describeRoll(event: ResolutionEvent): string {
  const parts: string[] = [];
  if (event.roll) {
    const dice = event.roll.dice ?? [];
    parts.push(`${pipsFor(dice)} ${dice.join("+")} = ${event.roll.total}`);
  }
  for (const modifier of event.modifiers ?? []) {
    if (modifier.value === 0) continue;
    parts.push(`${modifier.source} ${modifier.value > 0 ? "+" : ""}${modifier.value}`);
  }
  parts.push(`total ${event.total}`);
  if (event.result) parts.push(`→ ${event.result}`);
  return parts.join(" · ");
}

/**
 * The elements a viewpoint may be shown, WRECKS INCLUDED.
 *
 * ⚠ THIS IS WHY THE KILLING SHOT HAD NO LINE. The counter layer quite
 * reasonably draws only living elements, and the fire lines were filtered
 * through the same set — so the one shot a player most wants to see, the one
 * that destroyed something, was the one shot that could never be drawn. The
 * target had already left the list by the time the line was built.
 *
 * A destroyed element is still a fact about the board: it is where the wreck
 * is. Fog still applies — a side sees its own elements and whatever it has
 * sighted — and the umpire sees everything.
 */
export function visibleIdsFor(state: GameState, viewpoint: Side | "both"): Set<string> {
  if (viewpoint === "both") return new Set(Object.keys(state.forceElements));

  const ids = new Set<string>();
  for (const fe of Object.values(state.forceElements)) {
    if (fe.side === viewpoint) ids.add(fe.id);
  }
  for (const [id, level] of Object.entries(state.sighting[viewpoint] ?? {})) {
    if (level !== "none") ids.add(id);
  }
  return ids;
}

/** Elements that have been destroyed, so the map can show a wreck. */
export function wrecksIn(
  state: GameState,
  visible: ReadonlySet<string>,
): { id: string; label: string; side: Side; position: LatLng }[] {
  return Object.values(state.forceElements)
    .filter((fe) => fe.combatStrength <= 0 && visible.has(fe.id))
    .map((fe) => ({ id: fe.id, label: fe.label, side: fe.side, position: fe.position }));
}

/** A step as a viewpoint is allowed to see it. */
export interface VisibleStep {
  /** Index in the FULL list, so selecting one still addresses the real step. */
  index: number;
  step: TurnStep;
  /** The label this viewpoint may read, which is not always the real one. */
  label: string;
}

/**
 * The steps a viewpoint may be shown.
 *
 * ⚠ THE TIMELINE LEAKED THE WHOLE TURN. The map layers were careful about fog
 * and the step LIST was not: in "blue eyes only" it listed every red action,
 * labelled, with red's dice attached. The umpire's view is the honest one; the
 * per-side views exist so a student can be shown what THEY knew, and a
 * single-side view that quietly shows everything is worse than no view at all.
 *
 * Three rules:
 *   - Phase steps (no side) are shown: "Sighting", "Clean-up", "Turn 3 begins"
 *     are facts about the clock, not about the enemy.
 *   - Own-side steps are shown in full.
 *   - An enemy step is shown only if the acting element was VISIBLE at that
 *     moment, and then only as far as the eye would tell. A march label
 *     carries the plan ("continues to the objective, 4,100 m to go"), and a
 *     plan is not observable — so everything after the first bracket is cut.
 */
export function stepsVisibleTo(
  steps: readonly TurnStep[],
  viewpoint: Side | "both",
): VisibleStep[] {
  const out: VisibleStep[] = [];

  for (const [index, step] of steps.entries()) {
    if (viewpoint === "both" || step.side === undefined) {
      out.push({ index, step, label: step.label });
      continue;
    }
    if (step.side === viewpoint) {
      out.push({ index, step, label: step.label });
      continue;
    }
    // An enemy's DECISION is not observable, only what it leads to. "R1
    // held fire" shown to blue would reveal an overwatch blue cannot see.
    if (step.decision) continue;

    const actorId = step.actorId;
    if (!actorId) continue;
    if (!visibleIdsFor(step.state, viewpoint).has(actorId)) continue;

    const bracket = step.label.indexOf(" (");
    out.push({
      index,
      step,
      label: bracket > 0 ? step.label.slice(0, bracket) : step.label,
    });
  }

  return out;
}

/**
 * May this viewpoint see the route an element is following?
 *
 * A ROUTE IS A PLAN, NOT AN OBSERVABLE FACT. You can watch an enemy troop
 * move; you cannot see where it intends to be in four turns. So own side and
 * the umpire only — drawing a sighted enemy's route would hand over its
 * intentions, which is a larger leak than showing its counter.
 */
export function maySeeRoute(side: Side, viewpoint: Side | "both"): boolean {
  return viewpoint === "both" || side === viewpoint;
}

/**
 * The rolls a viewpoint may read.
 *
 * You know the arithmetic of an engagement you were IN — you were shot at, or
 * you did the shooting. The dice behind two enemy elements resolving something
 * between themselves are the umpire's business.
 */
export function eventsVisibleTo(
  step: Pick<TurnStep, "state" | "events">,
  viewpoint: Side | "both",
): ResolutionEvent[] {
  const events = step.events ?? [];
  if (viewpoint === "both") return events;
  const mine = (id: string): boolean => step.state.forceElements[id]?.side === viewpoint;
  return events.filter(
    (event) => event.actorIds.some(mine) || event.targetIds.some(mine),
  );
}

/** What a decider last chose for one element, for a marker on its counter. */
export interface DecisionMark {
  actorId: string;
  /** Short enough to sit beside a counter: "hold 22%", "press", "fire B1". */
  text: string;
  /** The full step label, for the tooltip. */
  title: string;
  /** Jev could not decide and the rule did. Drawn differently. */
  fallback: boolean;
}

/**
 * The latest in-the-moment decision per element, up to a point in the turn.
 *
 * `upTo` is a step index; -1 means the whole turn. Only steps the viewpoint
 * may see count — the same rule as the timeline — so a side's own map never
 * carries a mark that tells it what the enemy decided.
 */
export function decisionMarksFor(
  steps: readonly TurnStep[],
  upTo: number,
  viewpoint: Side | "both",
): Map<string, DecisionMark> {
  const marks = new Map<string, DecisionMark>();
  const last = upTo < 0 ? steps.length - 1 : Math.min(upTo, steps.length - 1);

  for (const { index, step } of stepsVisibleTo(steps, viewpoint)) {
    if (index > last) break;
    const decision = step.decision;
    if (!decision || !step.actorId) continue;
    const words = decision.mark ?? decision.chosenId;
    const short = words.length > 18 ? `${words.slice(0, 17)}\u2026` : words;
    marks.set(step.actorId, {
      actorId: step.actorId,
      text: decision.p != null && !decision.fallback ? `${short} ${Math.round(decision.p * 100)}%` : short,
      title: step.label,
      fallback: decision.fallback != null,
    });
  }
  return marks;
}
