// ── realtime/world/standalone.ts ─────────────────────────────────────────
// A world context outside the tick: for applying a decision's answer, which
// the runner does between ticks (as it does for land decisions), and for
// setting a game up. It lends the same LandView a tick does, over copies of
// the state, and gives back the new state and any events it raised for the
// runner to hold.

import { knownTo, routed, sendMessage } from "../engine/engine";
import type { MessageDraft } from "../engine/comms";
import type { RtConfig, RtEvent, RtState, RtUnit } from "../engine/types";
import { WorldCtx, type LandView } from "./context";
import { finishWorld } from "./step";
import type { WorldLogItem } from "./types";

export interface StandaloneResult {
  state: RtState;
  events: RtEvent[];
  /** Feed lines it made. */
  log: WorldLogItem[];
}

/** Run `fn` against a world context over `state`, and collect what it changed. */
export function withWorldCtx(state: RtState, config: RtConfig, fn: (c: WorldCtx) => void): StandaloneResult {
  if (!state.world) return { state, events: [], log: [] };
  let game = state.game;
  const units: Record<string, RtUnit> = { ...state.units };
  const events: RtEvent[] = [];
  const radio: MessageDraft[] = [];
  const time = state.time;
  const view: LandView = {
    time,
    config,
    rng: config.rng,
    get game() {
      return game;
    },
    units,
    setFe(id, patch) {
      game = { ...game, forceElements: { ...game.forceElements, [id]: { ...game.forceElements[id], ...patch } } };
    },
    setUnit(id, patch) {
      units[id] = { ...units[id], ...patch };
    },
    order(id, order, extra = {}) {
      const fe = game.forceElements[id];
      if (!fe || !units[id]) return;
      units[id] = { ...units[id], ...extra, order: routed(order, fe, config), bound: undefined, laying: undefined };
    },
    addUnit(fe, unit) {
      game = { ...game, forceElements: { ...game.forceElements, [fe.id]: fe } };
      units[fe.id] = unit;
    },
    shots: [],
    emit(unitId, kind, detail, severe = false, extra = {}) {
      if ((game.forceElements[unitId]?.combatStrength ?? 0) <= 0) return;
      events.push({ time, unitId, kind, detail, severe, ...extra });
    },
    radio(draft) {
      radio.push(draft);
    },
    known: (id, enemyId) => knownTo(game, { game, units, reports: state.reports }, id, enemyId),
    prev: state,
  };
  const c = new WorldCtx(state.world, view);
  fn(c);
  let next: RtState = { ...state, game, units, world: finishWorld(c) };
  for (const draft of radio) next = sendMessage(next, draft, config);
  return { state: next, events, log: c.newLog };
}
