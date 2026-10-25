// ── world/command/seat.ts ────────────────────────────────────────────────
// Commander seats (MULTI_DOMAIN_PLAN.md §4.4.3): the ONLY way any commander
// — Claude, the heuristic, or (W15) a person — touches the simulation.
//
//   inbox()    only what has ARRIVED at this HQ over its networks
//   cop()      this HQ's fused COP, never the truth
//   send()     goes out on this HQ's nets: delay, loss and jamming apply
//
// Consequences built in now: reports reach a commander only if they arrive;
// orders leave as messages and a unit acts on them only when it receives
// them; the HQ can be hit, and the seat then moves to the alternate after a
// delay, during which it sees nothing new.

import type { Side } from "../../../lib/state";
import type { RtState, UnitOrders } from "../../engine/types";
import type { Headquarters, NetMessage, PlanAnswer, PlanRequest, WorldTrack } from "../types";

export interface CommanderSeat {
  hq: string;
  side: Side;
  kind: Headquarters["seat"];
  inbox(): NetMessage[];
  cop(): Record<string, WorldTrack>;
  /** Its subordinates, as it knows them: their last reports. */
  subordinates(): { id: string; lastReport?: NetMessage }[];
}

/** The seat at an HQ, read from the state as it stands. */
export function seatAt(state: RtState, hqId: string): CommanderSeat | undefined {
  const world = state.world;
  const hq = world?.hqs[hqId];
  if (!world || !hq) return undefined;
  const byId = new Map(world.messages.map((m) => [m.id, m]));
  const inbox = () => hq.inbox.map((id) => byId.get(id)).filter((m): m is NetMessage => m != null);
  return {
    hq: hqId,
    side: hq.side,
    kind: hq.seat,
    inbox,
    // A destroyed post sees nothing new until the seat has moved.
    cop: () => (hq.down ? {} : hq.cop),
    subordinates: () =>
      hq.subordinates.map((id) => {
        const last = inbox()
          .filter((m) => m.from === id)
          .sort((a, b) => b.dueAt - a.dueAt)[0];
        return { id, ...(last ? { lastReport: last } : {}) };
      }),
  };
}

/**
 * Who answers a plan request: the heuristic answers at once inside the
 * world; Claude (and later a person) through the runner, asynchronously,
 * the clock waiting at `readyAt` if the answer is late.
 */
export interface PlannerSeat {
  readonly name: string;
  plan(state: RtState, request: PlanRequest, seat: CommanderSeat): Promise<PlanAnswer>;
}

/** Orders as a plan answer carries them, for a planner that only writes UnitOrders. */
export function answerFromOrders(by: PlanAnswer["by"], plan: string, orders: Record<string, UnitOrders>, warnings: string[] = []): PlanAnswer {
  return { by, plan, orders, warnings };
}
