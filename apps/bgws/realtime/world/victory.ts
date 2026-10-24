// ── realtime/world/victory.ts ────────────────────────────────────────────
// Victory beyond the land breakpoint (§5.3, §5.4): air superiority over an
// area, a target set destroyed, sea control, ships and cargo through, or a
// joint operation's aims taken together. Each condition is judged from the
// truth every few seconds; one that must be held starts a clock when it is
// first met and resets if it is lost. The first side to meet one of its
// conditions wins, and the engine ends the game on the world's verdict.

import { distanceM } from "../../lib/board";
import { opposing, SIDES } from "../../lib/state";
import { hasRecord, recordFor } from "../catalogue";
import type { WorldCtx } from "./context";
import { registerSystem } from "./step";
import type { VictoryCondition } from "./types";

const AD = /sam|shorad|gbad/i;

/** How far a condition has got: met (true now), and a line for the feed. */
export function judge(c: WorldCtx, v: VictoryCondition): { met: boolean; text: string } {
  switch (v.kind) {
    case "airSuperiority": {
      const enemy = opposing(v.side);
      const air = Object.values(c.world.entities).filter((e) => e.owner === enemy && e.domain === "air" && e.status === "active" && hasRecord(e.archetype) && recordFor(e.archetype).body.crew > 0 && distanceM(e.pos, v.at) <= v.radiusM);
      const ad = Object.keys(c.world.land).filter((id) => c.sideOf(id) === enemy && c.landAlive(id) && AD.test(c.world.land[id].archetype) && c.world.land[id].emcon !== "silent" && distanceM(c.fe(id)!.position, v.at) <= v.radiusM);
      return { met: air.length === 0 && ad.length === 0, text: `${air.length} enemy aircraft and ${ad.length} radiating air-defence sites over the area` };
    }
    case "targetSet": {
      const gone = v.targets.filter((id) => (c.fe(id) ? !c.landAlive(id) : (c.entity(id)?.status ?? "destroyed") === "destroyed" || (c.entity(id)?.hp ?? 0) <= 0));
      return { met: gone.length >= Math.ceil(v.targets.length * v.share), text: `${gone.length} of ${v.targets.length} targets destroyed` };
    }
    case "seaControl": {
      const enemy = opposing(v.side);
      const ships = Object.values(c.world.entities).filter((e) => e.owner === enemy && (e.domain === "surface" || e.domain === "subsurface") && e.status === "active" && distanceM(e.pos, v.at) <= v.radiusM);
      const own = Object.values(c.world.entities).filter((e) => e.owner === v.side && e.domain === "surface" && e.status === "active" && distanceM(e.pos, v.at) <= v.radiusM);
      return { met: ships.length === 0 && own.length > 0, text: `${own.length} own and ${ships.length} enemy vessels in the area` };
    }
    case "through": {
      const key = `through:${v.side}:${v.label ?? v.ids.join(",")}`;
      for (const id of v.ids) {
        const e = c.entity(id);
        const at = e?.status === "active" ? e.pos : c.fe(id) && c.landAlive(id) ? c.fe(id)!.position : undefined;
        if (at && distanceM(at, v.at) <= v.radiusM && c.world.memo[`${key}:${id}`] == null) c.world.memo[`${key}:${id}`] = c.time;
      }
      const n = v.ids.filter((id) => c.world.memo[`${key}:${id}`] != null).length;
      return { met: n >= v.count, text: `${n} of ${v.count} through` };
    }
    case "all": {
      const parts = v.of.map((p) => judge(c, p));
      return { met: parts.every((p) => p.met), text: parts.map((p) => p.text).join("; ") };
    }
  }
}

function holdFor(v: VictoryCondition): number {
  return v.kind === "airSuperiority" || v.kind === "seaControl" ? v.holdS : 0;
}

registerSystem({
  name: "victory",
  order: 98,
  when: (c) => (c.world.victory?.length ?? 0) > 0 && !c.world.verdict,
  run(c) {
    if (c.time % 10 !== 0) return;
    for (const side of SIDES) {
      (c.world.victory ?? []).forEach((v, i) => {
        if (v.side !== side || c.world.verdict) return;
        const key = `victory:${i}`;
        const { met, text } = judge(c, v);
        if (!met) {
          if (c.world.memo[key] != null) c.log({ side, domain: "victory", kind: "lost", text: `${v.label ?? v.kind}: no longer met (${text})` });
          delete c.world.memo[key];
          return;
        }
        if (c.world.memo[key] == null) {
          c.world.memo[key] = c.time;
          if (holdFor(v) > 0) c.log({ side, domain: "victory", kind: "met", text: `${v.label ?? v.kind}: met (${text}); held for ${Math.round(holdFor(v) / 60)} min it wins` });
        }
        if (c.time - c.world.memo[key] >= holdFor(v)) {
          c.world.verdict = { winner: side, reason: `${v.label ?? v.kind} (${text})` };
          c.log({ side, domain: "victory", kind: "won", text: `${side} wins: ${v.label ?? v.kind} (${text})` });
        }
      });
    }
  },
});
