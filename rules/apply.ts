// ── bgws/rules/apply.ts ────────────────────────────────────────────────────
// The only place game state changes.
//
// Resolvers return effects; this applies them. One narrow door, for three
// reasons: a replay can be produced by applying the same effects in the same
// order, a bug in state handling has one place to be, and an effect that
// nothing applies is a resolver lying about what it did.
//
// Immutable: every application returns a new state. The cost is irrelevant at
// a battlegroup's scale and it makes "what did this turn change?" a diff.

import type { GameState, Marker, Morale, SightingLevel } from "../lib/state";
import type { StateDelta } from "./events";

export function applyEffects(state: GameState, effects: readonly StateDelta[]): GameState {
  let next = state;
  for (const effect of effects) next = applyEffect(next, effect);
  return next;
}

function applyEffect(state: GameState, effect: StateDelta): GameState {
  switch (effect.kind) {
    case "combatStrength": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      return withForceElement(state, {
        ...fe,
        // Strength floors at zero: a unit is eliminated, not negative, and a
        // negative strength would quietly invert every odds ratio it appears in.
        combatStrength: Math.max(0, fe.combatStrength + effect.delta),
      });
    }

    case "morale": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      return withForceElement(state, { ...fe, morale: effect.to as Morale });
    }

    case "marker": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      const marker = effect.marker as Marker;
      const markers = effect.added
        ? fe.markers.includes(marker)
          ? fe.markers
          : [...fe.markers, marker]
        : fe.markers.filter((m) => m !== marker);
      return withForceElement(state, { ...fe, markers });
    }

    case "position": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      return withForceElement(state, {
        ...fe,
        position: { lat: effect.lat, lng: effect.lng },
      });
    }

    case "sighting": {
      return {
        ...state,
        sighting: {
          ...state.sighting,
          [effect.viewer]: {
            ...state.sighting[effect.viewer],
            [effect.feId]: effect.to as SightingLevel,
          },
        },
      };
    }

    case "smoke": {
      return {
        ...state,
        smoke: [
          ...(state.smoke ?? []),
          { id: effect.id, position: { lat: effect.lat, lng: effect.lng }, placedTurn: effect.turn },
        ],
      };
    }

    case "route": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      // Clearing is `route: null`, not an absent field: "no longer marching"
      // has to be expressible as an effect, or a route could never be
      // abandoned by anything the replay could reproduce.
      const next = { ...fe };
      if (effect.route) next.route = effect.route;
      else delete next.route;
      return withForceElement(state, next);
    }

    case "facing": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      return withForceElement(state, { ...fe, facing: effect.to });
    }

    case "concealed": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      return withForceElement(state, { ...fe, concealed: effect.to });
    }

    case "eliminated": {
      const fe = state.forceElements[effect.feId];
      if (!fe) return state;
      // Kept in the state rather than deleted: the log refers to it, the
      // after-action review needs it, and the fog-of-war projection already
      // filters zero-strength elements out of both sides' views.
      return withForceElement(state, { ...fe, combatStrength: 0 });
    }

    case "initiative":
      return { ...state, initiative: effect.side };

    default:
      return state;
  }
}

function withForceElement(
  state: GameState,
  fe: GameState["forceElements"][string],
): GameState {
  return {
    ...state,
    forceElements: { ...state.forceElements, [fe.id]: fe },
  };
}

/**
 * Clean-up (8.0). Most markers are a within-turn thing; two are not.
 *
 * ⚠ THIS USED TO WIPE EVERYTHING, and 8.0 is explicit that it must not:
 *
 *   "1. Remove all MOVED, FIRED and SMOKE markers from the map.
 *    2. Remove REORG markers if they were placed as a result of New Orders
 *       this Turn, or an Assault last Turn (ones placed following an Assault
 *       this Turn remain in place). MELEE markers remain in place."
 *
 * Wiping MELEE meant close combat never survived the turn that started it,
 * so 9.3.8's whole mechanic — two forces fixed in place until one breaks —
 * could not happen, and `attackerAlreadyInMelee` was a modifier no sequence
 * of play could reach.
 *
 * REORG is a two-stage marker for the same reason: placed this turn it
 * survives, so that it restricts the element NEXT turn ("This requires a full
 * subsequent Turn"), and is removed at the clean-up after that.
 *
 * `keepPersistent` is false when the closeCombat module is off, which
 * restores the old wipe-everything behaviour exactly — so the module can be
 * measured against what came before it.
 */
export function clearAllMarkers(state: GameState, keepPersistent = false): GameState {
  const forceElements = Object.fromEntries(
    Object.entries(state.forceElements).map(([id, fe]) => {
      if (!keepPersistent) return [id, { ...fe, markers: [] }];

      const markers: Marker[] = [];
      // Locked in close combat until one side is eliminated or retreats.
      if (fe.markers.includes("melee")) markers.push("melee");
      // Placed this turn, so it bites next turn and is gone the turn after.
      if (fe.markers.includes("reorgPlacedThisTurn")) markers.push("reorg");

      return [id, { ...fe, markers }];
    }),
  );
  // 8.0 step 1 lists SMOKE alongside MOVED and FIRED: a cloud lasts the turn
  // it was fired and no longer. Cleared unconditionally, because smoke that
  // outlived clean-up would go on blinding people for the rest of the game.
  return { ...state, forceElements, smoke: [] };
}
