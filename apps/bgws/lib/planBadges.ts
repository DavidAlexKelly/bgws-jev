// ── bgws/lib/planBadges.ts ─────────────────────────────────────────────────
// What an element has been told to do, as a glyph beside its counter.
//
// WHY THIS REPLACED THE APP-6 TACTICAL GRAPHICS
//
// The plan overlay used @acc/app6d to draw doctrinal graphics — an axis of
// advance, a withdrawal arc, a main attack. They are correct, and they were
// the wrong tool here, for one structural reason:
//
//   A DOCTRINAL GRAPHIC DESCRIBES GROUND. A BADGE DESCRIBES A UNIT.
//
// An axis of advance spans origin to objective and says something about the
// terrain between them. On a 10 km board with two dozen counters, a dozen
// overlapping axes say very little about which COUNTER is doing what — and
// "which counter is doing what" is the question a player actually asks while
// looking at a pending turn. A glyph pinned to the counter answers it at a
// glance and can overlap nothing but itself.
//
// ⚠ THE VOCABULARY IS THE ENGINE'S, NOT DOCTRINE'S.
//
// The APP-6 catalogue offers ~35 tactical tasks. BGWS has five ActionKinds
// and four modifiers, and bridging the two meant inventing distinctions the
// rules cannot make. These badges map ONE-TO-ONE onto fields of ActionOption,
// so every glyph on screen is traceable to a rule:
//
//   kind "move"                    ->  advance
//   kind "move" + retreat          ->  withdraw      (9.3.6)
//   kind "move" + onContact press  ->  press on      (7.1.3)
//   kind "fire"                    ->  fire
//   kind "fire" + indirect         ->  indirect fire (9.2.2)
//   kind "fire" + smoke            ->  smoke
//   kind "assault"                 ->  assault       (9.3)
//   kind "hold"                    ->  hold
//
// A glyph appearing that is not in that table means the table is wrong, not
// the engine.

import type { PlanGeometry, PlanLine } from "./planLines";
import type { Side } from "./state";

export interface PlanBadge {
  /** A single character, drawn beside the counter. */
  glyph: string;
  /** The words, for the tooltip and the legend. */
  label: string;
  /** Movement sorts before engagement, so a bound reads before its shot. */
  order: number;
}

export interface ElementBadges {
  actorId: string;
  side: Side;
  badges: PlanBadge[];
}

// ⚠ DELIBERATELY BASIC-MULTILINGUAL-PLANE GLYPHS.
//
// Arrows and geometric shapes render in every system UI font. The tempting
// characters — crossed sabres, an artillery piece, a smoke puff — live in
// higher planes or in emoji fonts, and render as a tofu box on whatever
// machine the demo is given on. A glyph nobody can see is worse than a plain
// arrow, so the MEANING is carried by the tooltip and the legend, and the
// glyph only has to be distinct.
const ADVANCE: PlanBadge = { glyph: "\u2192", label: "advance", order: 0 };
const PRESS_ON: PlanBadge = { glyph: "\u21d2", label: "press on through contact", order: 0 };
const WITHDRAW: PlanBadge = { glyph: "\u2190", label: "withdraw", order: 0 };
const FIRE: PlanBadge = { glyph: "\u2731", label: "fire", order: 1 };
const INDIRECT: PlanBadge = { glyph: "\u2312", label: "indirect fire", order: 1 };
const SMOKE: PlanBadge = { glyph: "\u25cc", label: "smoke", order: 1 };
const ASSAULT: PlanBadge = { glyph: "\u2716", label: "assault", order: 2 };
const HOLD: PlanBadge = { glyph: "\u25a3", label: "hold", order: 3 };
// ⚠ A DIFFERENT THING FROM HOLD, AND THE DISTINCTION IS THE WHOLE POINT.
// `hold` is a decision: this element was told to stand still and it cost an
// activation to say so. `uncommitted` is the absence of a decision — usually
// deliberate, because under a command cap the right move is to leave elements
// out rather than spend capacity ordering them to do nothing. A hollow glyph
// for the hollow case.
const UNCOMMITTED: PlanBadge = { glyph: "\u25cb", label: "not committed this turn", order: 4 };

/** Every badge this module can produce, for the map legend. */
export const PLAN_BADGE_LEGEND: PlanBadge[] = [
  ADVANCE,
  PRESS_ON,
  WITHDRAW,
  FIRE,
  INDIRECT,
  SMOKE,
  ASSAULT,
  HOLD,
  UNCOMMITTED,
];

/** The badge for one ordered line. */
export function badgeFor(line: PlanLine): PlanBadge {
  if (line.kind === "move") {
    if (line.retreat) return WITHDRAW;
    return line.onContact === "press" ? PRESS_ON : ADVANCE;
  }
  if (line.kind === "assault") return ASSAULT;
  if (line.smoke) return SMOKE;
  if (line.indirect) return INDIRECT;
  return FIRE;
}

/**
 * One entry per ordered element, with its badges.
 *
 * An element can be told to do more than one thing — move then engage is a
 * legal pair — so badges are a list, deduplicated and sorted so movement
 * always reads before the shot that follows it.
 *
 * Takes the GEOMETRY rather than the PlannedTurn because the geometry has
 * already been through the fog-of-war gate: an element the current viewpoint
 * may not see has no lines, and so gets no badge either. Applying that rule
 * again here would mean two copies of it, free to disagree.
 */
export function planBadgesFor(geometry: PlanGeometry): Map<string, ElementBadges> {
  const byActor = new Map<string, ElementBadges>();

  const add = (actorId: string, side: Side, badge: PlanBadge) => {
    const existing = byActor.get(actorId);
    if (!existing) {
      byActor.set(actorId, { actorId, side, badges: [badge] });
      return;
    }
    // A combined fire mission produces one line per participant; two rounds
    // from the same element should still read as one "fire".
    if (!existing.badges.some((one) => one.glyph === badge.glyph)) {
      existing.badges.push(badge);
    }
  };

  for (const line of geometry.lines) add(line.actorId, line.side, badgeFor(line));
  // Holds have no geometry — they are the element deliberately standing still,
  // and they matter precisely because they look identical to an unordered one.
  // Which is why the unordered ones now get a glyph of their own rather than
  // being left off the map to be mistaken for an oversight.
  for (const badge of geometry.badges) {
    add(badge.actorId, badge.side, badge.kind === "hold" ? HOLD : UNCOMMITTED);
  }

  for (const entry of byActor.values()) {
    entry.badges.sort((a, b) => a.order - b.order || a.glyph.localeCompare(b.glyph));
  }
  return byActor;
}
