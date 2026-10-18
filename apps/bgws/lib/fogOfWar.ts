// ── bgws/lib/fogOfWar.ts ───────────────────────────────────────────────────
// What one side actually knows.
//
// WHY THIS IS CODE AND NOT A PROMPT
// ---------------------------------
// When an LLM plays a side, the only thing stopping it reading the opposing
// ORBAT is what it is given. "Do not use information about unsighted enemy
// units" is not a control — it is a request, and it fails silently and
// invisibly: the model plays slightly too well and nobody can point at the
// moment it cheated.
//
// So the projection is a function, it runs before anything reaches a prompt,
// and the AI's view of the game is built from the same object a human player's
// screen is. BGWS 1.0 calls fog of war and friction essential elements; this
// is where that is enforced rather than asserted.
//
// It is equally the right shape for the human UI: a Blue player's board should
// render this, not the full state with a filter applied at the last moment.

import type {
  ForceElement,
  GameState,
  Marker,
  Side,
  SightingLevel,
} from "./state";
import { forceElementsOf, opposing, sightingOf } from "./state";
import type { LatLng } from "./board";

/**
 * An enemy Force Element as it appears to the observing side.
 *
 * Everything absent is absent on purpose. Combat Strength, Troop Quality,
 * ammunition, morale and orders are the opponent's private state; a player who
 * could read them would not be playing the same game.
 */
export interface ObservedForceElement {
  id: string;
  side: Side;
  sighting: Exclude<SightingLevel, "none">;
  position: LatLng;
  /** Only at a full sighting is the counter's identity legible. */
  label?: string;
  sidc?: string;
  /** Markers an observer can see happen: it moved, it fired. */
  observedMarkers: Marker[];
}

export interface SideView {
  side: Side;
  turn: number;
  phase: GameState["phase"];
  initiative: Side | null;
  /** Own FEs, complete. */
  own: ForceElement[];
  /** Enemy FEs, only as far as they have been sighted. */
  contacts: ObservedForceElement[];
  /** Own transmissions and chits. The opponent's are not knowable. */
  transmissions: number;
  transmissionsLastTurn: number;
  chitsHeld: number;
}

/** Markers that are an observable event rather than private bookkeeping. */
const OBSERVABLE_MARKERS: ReadonlySet<Marker> = new Set(["moved", "fired", "melee"]);

/**
 * Project the game state into what one side can see.
 *
 * Rules applied, each of which is a way the naive version leaks:
 *
 *  1. An unsighted enemy FE is ABSENT, not hidden — no id, no placeholder.
 *     A list of "unknown" entries still tells you how many there are.
 *  2. A Dummy is projected exactly like a real FE. If it were flagged, or
 *     even distinguishable by a missing field, it would stop being a dummy.
 *  3. A partial sighting yields a position and nothing else. BGWS lets a
 *     partial contact be engaged with indirect fire but not identified, and
 *     that is the whole value of the distinction.
 *  4. Eliminated FEs are gone from both views.
 *  5. A mounted FE is not separately visible: its carrier is the contact.
 */
export function projectForSide(state: GameState, side: Side): SideView {
  const enemy = opposing(side);
  const own = forceElementsOf(state, side).filter((fe) => fe.combatStrength > 0);

  const contacts: ObservedForceElement[] = [];
  for (const fe of forceElementsOf(state, enemy)) {
    if (fe.combatStrength <= 0) continue;
    // A section inside its carrier is not a separate contact.
    if (fe.mountedIn) continue;

    const sighting = sightingOf(state, side, fe.id);
    if (sighting === "none") continue;
    // Concealed outranks a stale sighting: a unit that has gone to ground is
    // off the board again until something re-sights it.
    if (fe.concealed && sighting !== "full") continue;

    const observed: ObservedForceElement = {
      id: fe.id,
      side: fe.side,
      sighting,
      position: fe.position,
      observedMarkers: fe.markers.filter((m) => OBSERVABLE_MARKERS.has(m)),
    };

    if (sighting === "full") {
      observed.label = fe.label;
      observed.sidc = fe.sidc;
    }

    contacts.push(observed);
  }

  const sideState = state.sides[side];
  return {
    side,
    turn: state.turn,
    phase: state.phase,
    initiative: state.initiative,
    own,
    contacts,
    transmissions: sideState.transmissions,
    transmissionsLastTurn: sideState.transmissionsLastTurn,
    chitsHeld: sideState.chitsHeld,
  };
}

/**
 * The umpire's view: everything, unfiltered.
 *
 * Separate function rather than a flag on the one above, so that "give me the
 * whole state" can never be reached by passing the wrong argument — which is
 * precisely the bug this module exists to prevent.
 */
export function projectForUmpire(state: GameState): GameState {
  return state;
}

/**
 * Serialise a side's view for a language model.
 *
 * Compact on purpose: the logic client truncates context, and a model reasons
 * better about twenty lines than about a JSON dump. Positions are rounded to
 * five decimals — about a metre, far finer than any BGWS distance test — so
 * the model cannot mistake spurious precision for significance.
 */
export function describeForPrompt(view: SideView): string {
  const lines: string[] = [
    `TURN ${view.turn} · ${view.phase}${
      view.initiative ? ` · initiative: ${view.initiative}` : ""
    }`,
    `YOU ARE ${view.side.toUpperCase()}. Everything below is what you can see. ` +
      `Enemy force elements you have not sighted are not listed, and their ` +
      `absence is not evidence that they are not there.`,
    "",
    "YOUR FORCE ELEMENTS:",
  ];

  for (const fe of view.own) {
    const capabilities = fe.capabilities
      .map((c) => `${c.kind}<=${c.maxRangeM}m`)
      .join(" ");
    lines.push(
      `- ${fe.id} ${fe.label} [${fe.moveType}] TQ${fe.troopQuality} ` +
        `CS${fe.combatStrength}/${fe.combatStrengthStart} ${fe.morale}` +
        (fe.markers.length ? ` {${fe.markers.join(",")}}` : "") +
        ` at ${round5(fe.position.lat)},${round5(fe.position.lng)}` +
        (capabilities ? ` · ${capabilities}` : ""),
    );
  }

  lines.push("", "CONTACTS:");
  if (view.contacts.length === 0) {
    lines.push("- none sighted");
  }
  for (const contact of view.contacts) {
    lines.push(
      `- ${contact.id} ${contact.label ?? "unidentified"} (${contact.sighting}) ` +
        `at ${round5(contact.position.lat)},${round5(contact.position.lng)}` +
        (contact.observedMarkers.length
          ? ` {${contact.observedMarkers.join(",")}}`
          : ""),
    );
  }

  lines.push(
    "",
    `TRANSMISSIONS: ${view.transmissions} this turn, ` +
      `${view.transmissionsLastTurn} last turn · EW chits held: ${view.chitsHeld}`,
  );

  return lines.join("\n");
}

function round5(value: number): number {
  return Math.round(value * 1e5) / 1e5;
}
