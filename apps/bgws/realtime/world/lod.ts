// ── realtime/world/lod.ts ────────────────────────────────────────────────
// Level of detail by scope (MULTI_DOMAIN_PLAN.md §4.2). A ten-day operation
// cannot lead every platoon by hand across hundreds of kilometres, so:
//
//   AGGREGATED   a formation (an HQ's units) whose every unit is far from the
//                enemy moves as one: the lead unit's orders, each member
//                keeping its place in the formation; the rules decide for it,
//                not Jev. It disaggregates — every unit its own again — when
//                any of its units comes within the threshold of the enemy.
//   FAR          with Jev leading near contact only, a unit beyond that
//                distance is decided by the rules.
//
// Distances are from the truth: level of detail is how the simulation spends
// its effort, not something either side knows. Tactical scopes never
// aggregate; a game without a world is untouched.

import type { LatLng } from "../../lib/board";
import type { RtState } from "../engine/types";
import type { WorldCtx } from "./context";
import { wfig } from "./figures";
import { distanceM } from "./geo";
import { registerSystem } from "./step";

/** Does the rule decider decide for this unit (aggregated, or far from contact under "Jev near contact")? */
export function rulesOnly(state: RtState, unitId: string): boolean {
  return state.world?.land[unitId]?.lod != null;
}

const metres = (from: LatLng, to: LatLng) => {
  const k = 111_320;
  return { eastM: (to.lng - from.lng) * k * Math.cos((from.lat * Math.PI) / 180), northM: (to.lat - from.lat) * k };
};
const shifted = (at: LatLng, eastM: number, northM: number): LatLng => ({ lat: at.lat + northM / 111_320, lng: at.lng + eastM / (111_320 * Math.cos((at.lat * Math.PI) / 180)) });

function nearestEnemyM(c: WorldCtx, id: string): number {
  const fe = c.fe(id)!;
  let best = Infinity;
  for (const other of Object.keys(c.game.forceElements)) {
    const o = c.fe(other)!;
    if (o.side === fe.side || !c.landAlive(other)) continue;
    best = Math.min(best, distanceM(fe.position, o.position));
  }
  return best;
}

function lod(c: WorldCtx): void {
  const scope = c.config.scope;
  if (!scope) return;
  const alive = Object.keys(c.world.land).filter((id) => c.landAlive(id)).sort();
  const dist = new Map(alive.map((id) => [id, nearestEnemyM(c, id)]));
  // Formations: an HQ's units.
  const byHq = new Map<string, string[]>();
  for (const id of alive) {
    const hq = c.world.land[id].hq;
    if (hq) byHq.set(hq, [...(byHq.get(hq) ?? []), id]);
  }
  const aggregated = new Set<string>();
  if (scope.detail.aggregateAwayFromContact) {
    for (const [hq, members] of [...byHq.entries()].sort(([a], [b]) => a.localeCompare(b))) {
      if (members.length < 2) continue;
      const was = members.some((id) => c.world.land[id].lod === "aggregated");
      const closest = Math.min(...members.map((id) => dist.get(id)!));
      // Hysteresis: aggregate beyond one distance, come apart inside a nearer one.
      const stay = was ? closest > wfig("world/lod.DISAGGREGATE_M") : closest > wfig("world/lod.AGGREGATE_M");
      if (!stay) {
        if (was) {
          for (const id of members) c.setLand(id, { lod: undefined, formation: undefined });
          c.log({ side: c.fe(members[0])!.side, domain: "lod", kind: "disaggregate", text: `${hq}'s formation comes apart into its ${members.length} units: the enemy is ${Math.round(closest / 1000)} km away`, ids: members });
        }
        continue;
      }
      const lead = members[0];
      for (const id of members) {
        aggregated.add(id);
        if (!was) c.setLand(id, { lod: "aggregated", formation: { lead, ...metres(c.fe(lead)!.position, c.fe(id)!.position) } });
      }
      if (!was) c.log({ side: c.fe(lead)!.side, domain: "lod", kind: "aggregate", text: `${hq}'s ${members.length} units move as one formation, ${Math.round(closest / 1000)} km from the enemy`, ids: members });
      follow(c, members);
    }
  }
  // Jev near contact only: the rest are the rules'.
  for (const id of alive) {
    if (aggregated.has(id)) continue;
    const far = scope.detail.jev === "nearContact" && dist.get(id)! > wfig("world/lod.JEV_NEAR_M");
    const want = far ? "far" : undefined;
    if (c.world.land[id].lod !== want) c.setLand(id, { lod: want, formation: undefined });
  }
}

/** Members of an aggregated formation take the lead's orders, keeping their place. */
function follow(c: WorldCtx, members: string[]): void {
  const lead = c.land.units[members[0]];
  for (const id of members.slice(1)) {
    const f = c.world.land[id].formation;
    if (!f) continue;
    const unit = c.land.units[id];
    if (lead.order.kind === "move" || lead.order.kind === "withdraw") {
      const to = shifted(lead.order.to, f.eastM, f.northM);
      const now = unit.order;
      if ((now.kind === "move" || now.kind === "withdraw") && distanceM(now.to, to) < 200) continue;
      c.land.order(id, lead.order.kind === "move" ? { kind: "move", to, mode: lead.order.mode } : { kind: "withdraw", to });
    } else if (unit.order.kind !== lead.order.kind && (lead.order.kind === "hold" || lead.order.kind === "overwatch")) {
      c.land.order(id, { kind: lead.order.kind });
    }
  }
}

registerSystem({
  name: "lod",
  order: 2,
  when: (c) => c.config.scope != null && (c.config.scope.detail.aggregateAwayFromContact || c.config.scope.detail.jev === "nearContact"),
  run(c) {
    if (c.time % 30 === 0 || c.time === 1) lod(c);
  },
});
