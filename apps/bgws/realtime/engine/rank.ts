// Element order: the order every "for each element" loop in the engine walks in.
// (Moved out of engine.ts so picture.ts can use it without a circular import.)

import type { GameState } from "../../lib/state";

/**
 * Each element's position in `game.forceElements`, the order every "for each
 * element" loop walks in. Lookups below go through what a unit knows (a few
 * entries) instead of asking about every enemy, and sort by this so their
 * results come out in exactly the order the full walk would give.
 *
 * Elements are never added or removed during a game, so one rank holds for a
 * whole tick; `tick` works it out once and passes it in. Other callers get it
 * cached per `forceElements` object.
 */
export interface ElementRank {
  /** Element ids in `forceElements` order. */
  readonly ids: readonly string[];
  readonly index: ReadonlyMap<string, number>;
}
const rankCache = new WeakMap<GameState["forceElements"], ElementRank>();
export function elementRank(game: GameState): ElementRank {
  let rank = rankCache.get(game.forceElements);
  if (!rank) {
    const ids = Object.keys(game.forceElements);
    rank = { ids, index: new Map(ids.map((id, i) => [id, i])) };
    rankCache.set(game.forceElements, rank);
  }
  return rank;
}
