// ── world/domains/support.ts ─────────────────────────────────────────────
// Abstracted domains as support services (MULTI_DOMAIN_PLAN.md §5.2): no
// entities on the board, but the domain's effect arrives with a response
// time, an availability and an effect, so a small battle feels the wider war.
//
//   fires      an off-board battery answers calls for fire (fires.ts)
//   air        close air support on call: a strike on a unit in contact,
//              cancelled while an air-defence threat is known near the target
//   drones     an ISR overflight now and then reports what it sees near the front
//   space      imagery every few hours, of the enemy as it stood, delivered late
//   logistics  a resupply drop at the unit after the response time
//
// Each side gets one service per abstracted domain when the world is made.

import { SIDES, opposing, type Side } from "../../../lib/state";
import { weaponSpec } from "../../catalogue";
import { registerSystem } from "../step";
import { onCreateWorld } from "../create";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { distanceM, offsetBy, bearingDeg } from "../geo";
import { launch } from "../munitions";
import { refill, setResupplyHandler } from "./landExt";
import type { SupportService } from "../types";

const DEFAULTS: Record<SupportService["domain"], Omit<SupportService, "id" | "side" | "used" | "hour" | "pending">> = {
  fires: { domain: "fires", label: "General-support artillery", responseMin: 8, perHour: 6 },
  air: { domain: "air", label: "Close air support on call", responseMin: 20, perHour: 2, cancelIfAdWithinM: 10_000 },
  drones: { domain: "drones", label: "ISR drone overflight", responseMin: 10, perHour: 3 },
  space: { domain: "space", label: "Satellite imagery", responseMin: 45, perHour: 0.25 },
  logistics: { domain: "logistics", label: "Resupply drop", responseMin: 120, perHour: 4 },
};

onCreateWorld(20, (world, _state, config) => {
  const scope = config.scope;
  if (!scope) return;
  for (const side of SIDES) {
    for (const domain of Object.keys(DEFAULTS) as SupportService["domain"][]) {
      if (scope.domains[domain] !== "abstracted") continue;
      world.support.push({ ...DEFAULTS[domain], id: `${side}-${domain}`, side, used: 0, hour: 0, pending: [] });
    }
  }
});

function service(c: WorldCtx, side: Side, domain: SupportService["domain"]): SupportService | undefined {
  return c.world.support.find((s) => s.side === side && s.domain === domain);
}

/** Take one use of a service this hour, if any is left; returns false when used up. */
export function useService(c: WorldCtx, s: SupportService, entry: SupportService["pending"][number]): boolean {
  const hour = Math.floor(c.time / 3600);
  const fresh = s.hour === hour ? s : { ...s, hour, used: 0 };
  if (fresh.used + 1 > Math.max(1, fresh.perHour)) return false;
  const i = c.world.support.indexOf(s);
  c.world.support[i] = { ...fresh, used: fresh.used + 1, pending: [...fresh.pending, entry] };
  return true;
}

// Abstracted logistics: a resupply drop after the response time.
setResupplyHandler((c, unitId) => {
  const side = c.fe(unitId)?.side;
  if (!side) return;
  if (c.mode("logistics") === "simulated" && SIMULATED_RESUPPLY) return SIMULATED_RESUPPLY(c, unitId);
  const s = service(c, side, "logistics");
  if (!s) return;
  if (useService(c, s, { at: c.time, dueAt: c.time + s.responseMin * 60, requestedBy: unitId })) {
    c.log({ side, domain: "logistics", kind: "request", text: `${unitId} asks for resupply: a drop in about ${s.responseMin} min`, ids: [unitId] });
  }
});

let SIMULATED_RESUPPLY: ((c: WorldCtx, unitId: string) => void) | null = null;
/** The logistics domain, when simulated, takes resupply requests itself. */
export function setSimulatedResupply(fn: (c: WorldCtx, unitId: string) => void): void {
  SIMULATED_RESUPPLY = fn;
}

/** Ask for close air support on an enemy in contact with this unit. */
export function requestCas(c: WorldCtx, unitId: string, targetId: string): boolean {
  const side = c.fe(unitId)?.side;
  if (!side) return false;
  const s = service(c, side, "air");
  if (!s) return false;
  return useService(c, s, { at: c.time, dueAt: c.time + s.responseMin * 60, requestedBy: unitId, targetId });
}

function deliver(c: WorldCtx): void {
  for (let i = 0; i < c.world.support.length; i++) {
    const s = c.world.support[i];
    const due = s.pending.filter((p) => p.dueAt <= c.time);
    if (!due.length) continue;
    c.world.support[i] = { ...s, pending: s.pending.filter((p) => p.dueAt > c.time) };
    for (const p of due) {
      if (s.domain === "logistics") {
        if (!c.landAlive(p.requestedBy)) continue;
        refill(c, p.requestedBy);
        c.log({ side: s.side, domain: "logistics", kind: "resupplied", text: `${p.requestedBy} resupplied by drop`, ids: [p.requestedBy] });
      } else if (s.domain === "air") {
        casStrike(c, s, p.requestedBy, p.targetId);
      }
    }
  }
}

function casStrike(c: WorldCtx, s: SupportService, requester: string, targetId?: string): void {
  const target = targetId ? c.fe(targetId) : undefined;
  if (!target || !c.landAlive(targetId!)) return;
  // Cancelled if the side knows of enemy air defence near the target.
  const ad = Object.values(c.world.tracks[s.side]).find((t) => /air defence|SAM/i.test(t.thought) && distanceM(t.at, target.position) <= (s.cancelIfAdWithinM ?? 0));
  if (ad) {
    c.log({ side: s.side, domain: "air", kind: "casCancelled", text: `CAS for ${requester} cancelled: air defence known near the target`, ids: [requester] });
    return;
  }
  const from = offsetBy(target.position, bearingDeg(target.position, c.world.startLines[s.side]), 7000);
  launch(c, { launcherId: `support:${s.id}`, owner: s.side, weaponId: "agm", from, fromAltM: 3000, targetId: targetId!, aimAt: target.position });
  launch(c, { launcherId: `support:${s.id}`, owner: s.side, weaponId: "agm", from, fromAltM: 3000, targetId: targetId!, aimAt: target.position });
  c.log({ side: s.side, domain: "air", kind: "cas", text: `close air support strikes ${targetId} for ${requester}`, ids: [requester, targetId!], at: target.position });
}

/** An ISR overflight or an imagery pass reports the enemy it sees, on the land net, from a unit near the front. */
function overflights(c: WorldCtx): void {
  for (const s of c.world.support) {
    if (s.domain !== "drones" && s.domain !== "space") continue;
    const everyS = 3600 / Math.max(0.01, s.perHour);
    if (c.time === 0 || c.time % Math.round(everyS) !== 0) continue;
    const enemy = opposing(s.side);
    const own = Object.keys(c.game.forceElements).filter((id) => c.fe(id)!.side === s.side && c.landAlive(id)).sort();
    if (!own.length) continue;
    const reporter = own[0];
    const radius = s.domain === "drones" ? wfig("world/domains/support.ISR_RADIUS_M") : Infinity;
    const lateS = s.responseMin * 60;
    for (const id of Object.keys(c.game.forceElements).sort()) {
      const fe = c.fe(id)!;
      if (fe.side !== enemy || !c.landAlive(id)) continue;
      const nearFront = own.some((o) => distanceM(c.fe(o)!.position, fe.position) <= radius);
      if (!nearFront || !c.chance(s.domain === "space" ? 1 - c.world.env.cloudCover * 0.8 : 0.6)) continue;
      c.land.radio({
        kind: "contact",
        from: reporter,
        to: "all",
        text: `${s.label}: ${fe.label} (${id}) seen`,
        contact: { enemyId: id, level: "partial", at: fe.position, seenAt: c.time - (s.domain === "space" ? lateS : 0), errorM: s.domain === "space" ? 100 : 40, moving: false },
      });
    }
    c.log({ side: s.side, domain: s.domain, kind: "overflight", text: `${s.label} reports` });
  }
}

registerSystem({
  name: "support",
  order: 130,
  when: (c) => c.world.support.length > 0,
  run(c) {
    deliver(c);
    overflights(c);
  },
});

void weaponSpec;
