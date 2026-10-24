// ── world/command/heuristicPlanner.ts ────────────────────────────────────
// The heuristic planner: what any HQ's seat falls back on (§4.4.2 point 6),
// and what plans when no Claude planner is configured. It works only from
// the HQ's COP and its subordinates' last reports, as Claude would.
//
// A land HQ weighs its strength against what it knows of the enemy near the
// objective. Strong enough, it attacks: manoeuvre units advance on the
// objective on a frontage, one held back as the reserve, fire support moving
// up into range, engineers following the lead. Too weak, it defends where it
// stands. An HQ with HQs under it hands each a task (guidance) and lets it
// plan its own units' orders.

import type { LatLng } from "../../../lib/board";
import type { ForceElement } from "../../../lib/state";
import { distanceM } from "../../../lib/board";
import { hasRecord, recordFor } from "../../catalogue";
import type { RtState, UnitOrders } from "../../engine/types";
import { wfig } from "../figures";
import { offsetBy, bearingDeg } from "../geo";
import type { Guidance, Headquarters, PlanAnswer, WorldTrack } from "../types";

type Role = "manoeuvre" | "fires" | "engineer" | "support" | "post";

function roleOf(state: RtState, id: string): Role {
  const archetype = state.world?.land[id]?.archetype ?? "";
  if (archetype.startsWith("generic/cp-")) return "post";
  if (!hasRecord(archetype)) return "manoeuvre";
  const rec = recordFor(archetype);
  if (rec.weapons.some((w) => ["mortar", "howitzer", "rocket"].includes(w.kind))) return "fires";
  if (archetype === "generic/engineerSection") return "engineer";
  if (["generic/mbt", "generic/ifv", "generic/infantrySection", "generic/atgmTeam", "generic/recce"].includes(archetype)) return "manoeuvre";
  return "support";
}

function orders(state: RtState, phases: UnitOrders["phases"], task: string, intent: string, extra: Partial<UnitOrders> = {}): UnitOrders {
  return { task, phases, phase: 0, intent, urgency: "whenAble", roe: "withinShortRange", onContact: "engage", boundaries: [], by: "heuristic", issuedAt: state.time, ...extra };
}

/** The enemy's strength as the COP suggests it near a point: each land track counted as a platoon like ours. */
function enemyNear(cop: Record<string, WorldTrack>, at: LatLng, radiusM: number, perTrack: number): number {
  return Object.values(cop).filter((t) => t.domain === "land" && distanceM(t.at, at) <= radiusM).length * perTrack;
}

/**
 * The simple opening orders, made sensible for a world game: the
 * everyone-advances heuristic is right for manoeuvre units, but drone teams,
 * EW, air defence, artillery and command posts hold where they were placed
 * and do their own work from there.
 */
export function holdSupport(state: RtState): Record<string, UnitOrders> {
  const out: Record<string, UnitOrders> = {};
  for (const id of Object.keys(state.world?.land ?? {}).sort()) {
    if (roleOf(state, id) === "manoeuvre" || roleOf(state, id) === "engineer") continue;
    if ((state.game.forceElements[id]?.combatStrength ?? 0) <= 0) continue;
    out[id] = orders(state, [{ label: "work from this position", order: { kind: "hold" } }], "support from here", "support the force from where it stands", { onContact: "avoid" });
  }
  return out;
}

export function heuristicPlan(state: RtState, hq: Headquarters): PlanAnswer {
  const world = state.world!;
  const game = state.game;
  const objective: LatLng | undefined = hq.guidance?.objective ?? game.objectives?.[hq.side];
  const subs = hq.subordinates;
  const units = subs.filter((id) => (game.forceElements[id]?.combatStrength ?? 0) > 0 && state.units[id]);
  const subHqs = subs.filter((id) => world.hqs[id] && !world.hqs[id].down);
  const out: Record<string, UnitOrders> = {};
  const post = game.forceElements[hq.postId];
  const anchor = post?.position ?? (units[0] ? game.forceElements[units[0]].position : objective);
  if (!objective || !anchor) return { by: "heuristic", plan: "hold: no objective", orders: {} };

  const own = units.reduce((s, id) => s + game.forceElements[id].combatStrength, 0);
  const per = units.length ? own / units.length : 8;
  const enemy = enemyNear(hq.cop, objective, 2500, per);
  const ratio = own / Math.max(1, enemy);
  // An attack culminates when its losses pass the point a commander halts at, whatever the odds [ref, doctrine].
  const fleet = subs.filter((id) => state.units[id]);
  const total = fleet.reduce((s, id) => s + state.units[id].vehicles.total, 0);
  const lost = total ? 1 - fleet.reduce((s, id) => s + state.units[id].vehicles.fit, 0) / total : 0;
  const culminated = lost >= wfig("world/command/heuristicPlanner.CULMINATION_LOSS") && distanceM(anchor, objective) > 300;
  const attack = !culminated && (enemy === 0 || ratio >= 1.2);
  const intent = culminated
    ? `the attack has culminated at ${Math.round(lost * 100)}% losses: hold and consolidate`
    : attack
      ? `take the objective and hold it (${hq.echelon} attack, ${Math.round(ratio * 10) / 10}:1)`
      : `hold where we stand until the odds improve (${Math.round(ratio * 10) / 10}:1)`;

  // HQs below get a task, not orders: they plan their own.
  const guidance: Guidance = {
    issuedAt: state.time,
    by: hq.id,
    aims: [intent],
    priorities: attack ? ["the objective", "fire support to the main effort"] : ["hold", "counter-battery"],
    apportionment: {},
    roe: hq.guidance?.roe ?? { weaponsFree: true, escalationLimit: "conventional" },
    measures: hq.guidance?.measures ?? [],
    ...(attack ? { objective } : {}),
    ...(subHqs[0] ? { mainEffort: subHqs[0] } : {}),
  };

  const manoeuvre = units.filter((id) => roleOf(state, id) === "manoeuvre").sort();
  const reserve = attack && manoeuvre.length >= 3 ? manoeuvre[manoeuvre.length - 1] : undefined;
  const lead = manoeuvre.filter((id) => id !== reserve);
  const axis = bearingDeg(anchor, objective);
  // A frontage across the objective, 300 m between units.
  lead.forEach((id, i) => {
    const fe = game.forceElements[id];
    if (!attack) {
      out[id] = orders(state, [{ label: "hold this ground on overwatch", order: { kind: "overwatch" } }], "hold", intent);
      return;
    }
    const offset = (i - (lead.length - 1) / 2) * 300;
    const target = offsetBy(objective, (axis + 90) % 360, offset);
    const enemyClose = Object.values(hq.cop).some((t) => t.domain === "land" && distanceM(t.at, fe.position) < 3000);
    out[id] = orders(
      state,
      [
        { label: "advance on the objective", order: { kind: "move", to: target, mode: enemyClose ? "bound" : "tactical" } },
        { label: "hold the objective on overwatch", order: { kind: "overwatch" } },
      ],
      "attack: take the objective",
      intent,
    );
  });
  if (reserve) {
    const behind = offsetBy(objective, (axis + 180) % 360, 1800);
    out[reserve] = orders(state, [{ label: "follow as the reserve", order: { kind: "move", to: behind, mode: "tactical" } }, { label: "wait as the reserve", order: { kind: "hold" } }], "reserve", `${intent}; be ready to reinforce`, { onContact: "observe" });
  }
  for (const id of units) {
    const role = roleOf(state, id);
    if (role === "manoeuvre") continue;
    const fe: ForceElement = game.forceElements[id];
    if (role === "fires") {
      // Up into range of the objective, no closer than needed.
      const rec = recordFor(world.land[id].archetype);
      const reach = Math.max(...rec.weapons.map((w) => w.maxRangeM));
      const d = distanceM(fe.position, objective);
      const phases: UnitOrders["phases"] =
        attack && d > reach * 0.7
          ? [{ label: "move up into range", order: { kind: "move", to: offsetBy(objective, (axis + 180) % 360, reach * 0.5), mode: "tactical" } }, { label: "in position: answer calls for fire", order: { kind: "hold" } }]
          : [{ label: "in position: answer calls for fire", order: { kind: "hold" } }];
      out[id] = orders(state, phases, "fire support", intent, { onContact: "avoid" });
    } else if (role === "engineer") {
      const follow = offsetBy(objective, (axis + 180) % 360, 1200);
      out[id] = orders(state, attack ? [{ label: "follow the lead, ready to breach", order: { kind: "move", to: follow, mode: "tactical" } }, { label: "ready to breach", order: { kind: "hold" } }] : [{ label: "prepare obstacles", order: { kind: "hold" } }], "mobility support", intent, { onContact: "avoid" });
    } else if (role === "post") {
      const behind = offsetBy(objective, (axis + 180) % 360, 2500);
      out[id] = orders(state, attack && distanceM(fe.position, objective) > 4000 ? [{ label: "follow the attack", order: { kind: "move", to: behind, mode: "tactical" } }, { label: "command", order: { kind: "hold" } }] : [{ label: "command", order: { kind: "hold" } }], "command", intent, { onContact: "avoid", roe: "ifFiredUpon" });
    } else {
      out[id] = orders(state, [{ label: "cover the force", order: { kind: "hold" } }], "support the force", intent, { onContact: "avoid" });
    }
  }
  return {
    by: "heuristic",
    plan: `${hq.id}: ${intent}${reserve ? `; reserve ${reserve}` : ""}`,
    orders: out,
    guidance,
    ...(reserve ? { reserve } : {}),
  };
}
