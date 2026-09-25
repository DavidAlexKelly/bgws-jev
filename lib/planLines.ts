// ── bgws/lib/planLines.ts ──────────────────────────────────────────────────
// A PLANNED turn, as geometry to draw.
//
// WHY THIS IS A SEPARATE PURE MODULE
//
// The play screen has a "generate orders" press and an "execute orders"
// press, and between them the orders are on the map. What to draw is entirely
// decided by the rules — an option already carries its destination and its
// route — so working it out is arithmetic, not rendering, and it is testable
// without a map, a browser or a model.
//
// ⚠ FOG OF WAR APPLIES TO PLANS MORE STRICTLY THAN TO ANYTHING ELSE.
//
// A route is a plan, not an observable fact: you can watch an enemy move, you
// cannot see where it means to be in four turns. Drawing both sides' orders
// before either executes would hand each commander the other's intentions for
// free — and the whole reason both sides are asked before either is executed
// is that neither is supposed to have them. So every line here goes through
// `maySeeRoute`, the same gate the existing route layer uses.
//
// WHAT HAS NO GEOMETRY. `hold` and `pass` have nowhere to point, so they are
// returned as badges rather than as lines. Silently dropping them would make
// an element that was deliberately held indistinguishable from one that was
// never ordered — which is exactly the distinction the orders panel exists to
// show.

import type { LatLng } from "./board";
import type { GameState, Side } from "./state";
import { maySeeRoute } from "./stepVisuals";
import type { PlannedTurn } from "../rules/orders";

/** What sort of intention a line represents. */
export type PlanLineKind = "move" | "fire" | "assault";

/** One intention, as a polyline. */
export interface PlanLine {
  /** The element this line belongs to. For combined fire, each participant. */
  actorId: string;
  side: Side;
  kind: PlanLineKind;
  /** The option id, so a click can be traced back to what was ordered. */
  optionId: string;
  /** Points in order, always starting where the element currently stands. */
  points: LatLng[];
  /** Readable summary, straight off the option the commander was offered. */
  label: string;
  /** A Retreat (9.3.6) rather than an advance. Drawn differently. */
  retreat?: boolean;
  /**
   * What this element does if it makes contact while moving (7.1.3).
   *
   * Carried through to the overlay because it is a real pre-committed
   * decision and it changes the picture: an advance that will press on
   * through contact is a different plan from one that will go to ground, and
   * a commander reading the map should be able to tell them apart.
   */
  onContact?: "halt" | "press";
  /** An indirect fire mission (9.2.2) rather than a direct shot. */
  indirect?: boolean;
  /** A smoke mission — no casualties, a cloud. */
  smoke?: boolean;
  /** True when more than one element is acting together (9.2.1, 9.3). */
  combined?: boolean;
  /**
   * This order was asked for and REFUSED — most often for want of command
   * capacity (5.1).
   *
   * ⚠ REFUSALS USED TO BE DRAWN AS NOTHING, WHICH MADE THE COMMAND MECHANIC
   * INVISIBLE AT THE EXACT MOMENT IT BIT. With `commandActivations` on, a
   * side that wants seven things and can afford three has four elements that
   * sit motionless with no line, no badge and no explanation — identical on
   * the map to an element nobody thought about. The cost of the cap was the
   * one thing the player could not see, and the cap is the whole mechanic.
   *
   * Drawn dimmed rather than omitted, because "I wanted to do this and could
   * not afford it" is a different fact from "nothing was planned here", and
   * the difference is what a commander is supposed to be weighing.
   */
  refused?: boolean;
  /** Why it was refused, in the validator's own words. */
  refusedReason?: string;
  /** Which planner produced the route, for a move that has one. */
  planner?: "raster" | "bearings";
}

/** An intention with nowhere to point: an element that is not going anywhere. */
export interface PlanBadge {
  actorId: string;
  side: Side;
  /** Absent for `uncommitted`: there is no option behind a decision not taken. */
  optionId?: string;
  label: string;
  /**
   * Why this element is standing still.
   *
   * `hold`         — ordered to, explicitly. A decision that was taken.
   * `uncommitted`  — offered a decision and never given one.
   *
   * ⚠ THE SECOND CASE WAS INVISIBLE, AND IT IS THE COMMON ONE. `unorderedIn`
   * has computed it since the plan overlay was written, is exported, is
   * tested — and is called by nothing. The only mention of it in the app is a
   * comment in Play.tsx saying the distinction matters.
   *
   * It matters more now than when that was written. A model under a command
   * cap does not order holds, it simply leaves elements out — and it is right
   * not to, because a hold consumes a full activation. So the elements it
   * deliberately kept in hand were drawn exactly like the ones it forgot,
   * which is to say not at all, while its own plan text said "fe-1 and fe-2
   * hold pending contact reports".
   */
  kind: "hold" | "uncommitted";
}

export interface PlanGeometry {
  lines: PlanLine[];
  badges: PlanBadge[];
}

/**
 * Every accepted intention this viewpoint may see, as geometry.
 *
 * Reads from `planned.accepted` rather than `planned.orders`: an order that
 * was rejected is not going to happen, and drawing it would show a plan the
 * engine has already declined to honour. Rejections are reported in words by
 * the orders panel instead, which is where a reader can see the reason.
 */
export function planGeometryFor(
  planned: PlannedTurn,
  viewpoint: Side | "both",
): PlanGeometry {
  const lines: PlanLine[] = [];
  const badges: PlanBadge[] = [];

  for (const side of ["blue", "red"] as Side[]) {
    if (!maySeeRoute(side, viewpoint)) continue;

    // Accepted and refused are drawn from the same geometry, and only the
    // style tells them apart. Refused come last so that where two lines run
    // over each other the real one is on top.
    const drawable = [
      ...planned.accepted[side].map((intent) => ({
        intent,
        refusedReason: undefined as string | undefined,
      })),
      ...planned.rejected[side].map(({ intent, reason }) => ({
        intent,
        refusedReason: reason,
      })),
    ];

    for (const { intent, refusedReason } of drawable) {
      const option = planned.requests[side].optionsByElement[intent.actorId]?.find(
        (candidate) => candidate.id === intent.optionId,
      );
      // A refusal of "not a legal option for that element" has nothing to
      // draw — there is no geometry behind an order the rules never offered.
      if (!option) continue;

      const label = option.summary ?? intent.optionId;
      const refused = refusedReason != null;

      if (option.kind === "hold" || option.kind === "pass") {
        // A REFUSED HOLD IS NOT WORTH SAYING. The element stands still either
        // way, so a badge would report a difference the board does not have.
        if (!refused) {
          badges.push({
            actorId: intent.actorId,
            side,
            optionId: option.id,
            label,
            kind: "hold",
          });
        }
        continue;
      }

      if (option.kind === "move") {
        const actor = planned.state.forceElements[intent.actorId];
        if (!actor) continue;

        // A long march carries its whole route; an ordinary bound carries only
        // where it is going. Prefer the route — it is the ground the element
        // will actually cover, and drawing a straight line instead would show
        // a march through terrain the planner routed around.
        const path =
          option.route && option.route.waypoints.length > 0
            ? option.route.waypoints
            : option.destination
              ? [option.destination]
              : [];
        if (path.length === 0) continue;

        lines.push({
          actorId: intent.actorId,
          side,
          kind: "move",
          optionId: option.id,
          points: [actor.position, ...path],
          label,
          retreat: option.retreat,
          onContact: option.onContact,
          planner: option.route?.planner,
          refused,
          refusedReason,
        });
        continue;
      }

      // Fire and assault point at something. Every participant gets a line,
      // not just the lead: the whole mechanical point of Combined Fire is that
      // several elements converge on one target, and one line from the lead
      // would draw a massed engagement as a single shot.
      const target = option.targetId
        ? planned.state.forceElements[option.targetId]
        : undefined;
      if (!target) continue;

      const participants = option.actorIds?.length
        ? option.actorIds
        : option.actorId
          ? [option.actorId]
          : [];

      for (const participantId of participants) {
        const actor = planned.state.forceElements[participantId];
        if (!actor) continue;
        lines.push({
          actorId: participantId,
          side,
          kind: option.kind === "assault" ? "assault" : "fire",
          optionId: option.id,
          points: [actor.position, target.position],
          label,
          indirect: option.indirect,
          smoke: option.smoke,
          combined: participants.length > 1,
          refused,
          refusedReason,
        });
      }
    }

    // ── Elements left in hand ────────────────────────────────────────────
    //
    // Anything that was offered a decision and given none. Computed here
    // rather than by calling `unorderedIn` because the rule is deliberately
    // stricter: an element whose order was REFUSED already has a ghost line
    // saying what it wanted, and adding a badge on top would report the same
    // element twice with two different explanations.
    const spokenFor = new Set([
      ...planned.accepted[side].map((intent) => intent.actorId),
      ...planned.rejected[side].map(({ intent }) => intent.actorId),
    ]);

    for (const actorId of Object.keys(planned.requests[side].optionsByElement)) {
      if (spokenFor.has(actorId)) continue;
      // A destroyed element is not "uncommitted", it is gone.
      if ((planned.state.forceElements[actorId]?.combatStrength ?? 0) <= 0) continue;
      badges.push({
        actorId,
        side,
        label: "not committed this turn",
        kind: "uncommitted",
      });
    }
  }

  return { lines, badges };
}

/**
 * A one-line summary of what a side has been ordered to do.
 *
 * For the header, where there is room for a count and not for a list.
 */
export function describePlan(planned: PlannedTurn, side: Side): string {
  const accepted = planned.accepted[side].length;
  const rejected = planned.rejected[side].length;
  const failure = planned.orders[side].failure;

  if (failure) return `no orders — ${failure}`;
  if (accepted === 0 && rejected === 0) return "nothing ordered";

  const parts = [`${accepted} ordered`];
  if (rejected > 0) parts.push(`${rejected} refused`);
  return parts.join(", ");
}

/**
 * Elements that were offered a decision and given no order at all.
 *
 * NOT the same as the `hold` badges: those were deliberately told to stand
 * still. These were simply never mentioned, which is what a model's silence
 * looks like on the board, and it is worth being able to tell apart.
 */
export function unorderedIn(
  planned: PlannedTurn,
  state: GameState,
  side: Side,
): string[] {
  const ordered = new Set(planned.accepted[side].map((intent) => intent.actorId));
  return Object.keys(planned.requests[side].optionsByElement).filter(
    (actorId) =>
      !ordered.has(actorId) && (state.forceElements[actorId]?.combatStrength ?? 0) > 0,
  );
}
