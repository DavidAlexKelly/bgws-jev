// ── bgws/rules/contact.ts ──────────────────────────────────────────────────
// A move that stops when somebody sees somebody.
//
// ⚠ ELEMENTS USED TO DRIVE STRAIGHT PAST EACH OTHER.
//
// A move was atomic: the loop took the turn's destination and put the element
// there. Nothing happened in between. So two troops whose routes crossed swapped
// positions inside one turn, each arriving behind the other, and only noticed
// at the NEXT turn's sighting sweep — by which time they were past and facing
// the wrong way. On the map it looked like two units ignoring each other at
// 300 m.
//
// That is not a fog-of-war subtlety, it is the central event of a meeting
// engagement. BGWS's own sequence already treats a move as interruptible — the
// R of ARC exists so that movement can be answered — and this is the other
// half of the same idea: the mover itself halts when contact is made.
//
// WHAT THE RULEBOOK ACTUALLY SAYS, AND WHERE WE DIFFER
// ----------------------------------------------------
// 7.1.3 has the interrupt, and it belongs to the OTHER side: an FE in overwatch
// may Reactive Fire at a moving enemy, "the moving FE halts while the DirF is
// resolved", and then "the moving FE may continue its movement (unless it has
// become Disrupted or Broken ...), or it may elect to stop moving at that
// point". There is no rule by which a mover spots an ambush and stops itself.
//
// So the HALT is the rulebook's; the automatic halt was not. What the rulebook
// puts in the moving player's hands — press on or go to ground — this engine
// cannot ask for mid-resolution, because a move resolves synchronously and a
// commander is consulted once per activation. It is therefore PRE-COMMITTED on
// the option, exactly as StandingEngagement pre-commits the reactor's decision
// for the same reason: the same choice, made a moment earlier, binding for the
// move.
//
// WHAT HAPPENS HERE
// -----------------
// The move is walked in steps. At each step, any enemy that this side has NOT
// already sighted gets a sighting attempt the moment it has line of sight —
// using the ruleset's own sighting table, not a geometric shortcut, so cover,
// range and recce all still apply. With `haltOnContact` the first success stops
// the element there and abandons its march; without it the element presses on,
// recording every contact it makes along the way, which is the rulebook's
// "may continue its movement".
//
// WHAT DOES NOT HAPPEN
// --------------------
// An element ALREADY in contact does not halt: it has seen the enemy, the
// commander chose to move anyway, and a rule that stopped it would make
// movement impossible in contact — which BGWS plainly allows. Only NEW
// contacts halt, and each enemy is attempted once per move rather than once per
// step, so a long march does not roll thirty times against the same troop.

import { distanceM, type LatLng } from "../lib/board";
import { interpolate } from "../lib/movement";
import type { ForceElement, GameState } from "../lib/state";
import { forceElementsOf, opposing, sightingOf } from "../lib/state";
import { lineOfSight, type TerrainSampler } from "../lib/lineOfSight";
import { inCover } from "../lib/proceduralTerrain";
import type { ResolutionEvent, StateDelta } from "./events";
import { resolveSighting } from "./resolvers";
import type { RuleSet } from "./ruleset";
import type { Rng } from "./dice";

/** Metres between detection checks along a move. */
export const CONTACT_STEP_M = 200;

/**
 * Cap on checks per move, so a long march cannot make a turn quadratic.
 * 24 × 200 m is 4.8 km, which is more than any single turn's allowance.
 */
const MAX_CHECKS = 24;

export interface ContactWalk {
  /** Where the element actually ends up. */
  end: LatLng;
  /** Distance covered before halting, or the whole move. */
  distanceM: number;
  /** True when the move was cut short by making contact. */
  halted: boolean;
  /** Enemies newly sighted during the move, in the order they were seen. */
  contacts: string[];
  /** Sighting resolutions to log and apply. */
  events: ResolutionEvent[];
  /** The sighting levels this move established, as effects ready to apply. */
  sighted: StateDelta[];
}

/**
 * Walk a move, stopping at first contact.
 *
 * Deterministic: the step positions come from the geometry and the rolls come
 * from the shared generator in a fixed order, so a replay reproduces the halt
 * exactly.
 */
export function walkUntilContact(
  state: GameState,
  actor: ForceElement,
  to: LatLng,
  options: {
    terrain: TerrainSampler;
    ruleset: RuleSet;
    rng: Rng;
    turn: number;
    phase: ResolutionEvent["phase"];
    stepM?: number;
    /**
     * Stop at the first contact, or press on through it?
     *
     * 7.1.3 gives this choice to the moving player. Defaults to stopping,
     * which is the cautious reading and the one a commander that has expressed
     * no preference should get.
     */
    haltOnContact?: boolean;
  },
): ContactWalk {
  const { terrain, ruleset, rng, turn, phase } = options;
  const haltOnContact = options.haltOnContact ?? true;
  const from = actor.position;
  const total = distanceM(from, to);

  const nothing: ContactWalk = {
    end: to,
    distanceM: total,
    halted: false,
    contacts: [],
    events: [],
    sighted: [],
  };
  if (total <= 0) return { ...nothing, end: from, distanceM: 0 };

  const stepM = options.stepM ?? CONTACT_STEP_M;
  const steps = Math.min(MAX_CHECKS, Math.max(1, Math.ceil(total / stepM)));

  // Only enemies this side has NOT already sighted can BE a new contact. The
  // rest are why the commander chose to move in the first place.
  const candidates = forceElementsOf(state, opposing(actor.side)).filter(
    (enemy) => enemy.combatStrength > 0 && sightingOf(state, actor.side, enemy.id) === "none",
  );
  if (candidates.length === 0) return nothing;

  const events: ResolutionEvent[] = [];
  const sighted: ContactWalk["sighted"] = [];
  const attempted = new Set<string>();
  const contacts: string[] = [];

  for (let step = 1; step <= steps; step += 1) {
    const at = interpolate(from, to, step / steps);
    // The element is treated as being at `at` for the purpose of looking: the
    // check is what it would see FROM there, which is the whole point.
    const moving = { ...actor, position: at };

    for (const enemy of candidates) {
      if (attempted.has(enemy.id)) continue;
      if (!lineOfSight(terrain, { from: at, to: enemy.position }).visible) continue;

      attempted.add(enemy.id);
      const outcome = resolveSighting(
        moving,
        enemy,
        { targetInCover: inCover(terrain, enemy.position) },
        ruleset,
        rng,
        turn,
        phase,
        actor.side,
      );
      events.push(outcome.event as ResolutionEvent);

      const level = outcome.effects.find((effect) => effect.kind === "sighting");
      if (level && level.kind === "sighting" && level.to !== "none") {
        sighted.push({ kind: "sighting", viewer: actor.side, feId: enemy.id, to: level.to });
        contacts.push(enemy.id);

        // Pressing on is the rulebook's other branch: the contact is made and
        // recorded, and the element keeps going. Further enemies can still be
        // met further along, which is why the walk continues rather than
        // returning here.
        if (haltOnContact) {
          return {
            end: at,
            distanceM: distanceM(from, at),
            halted: true,
            contacts,
            events,
            sighted,
          };
        }
      }
    }
  }

  // Nobody new was seen: the move completes as ordered, and the failed
  // attempts are still logged — a sighting that was tried and failed is a
  // fact about the turn, and hiding it would make the halt look arbitrary
  // when it does happen.
  return { ...nothing, contacts, events, sighted };
}
