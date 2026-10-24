// ── world/domains/c2.ts ──────────────────────────────────────────────────
// Command, control and communications (MULTI_DOMAIN_PLAN.md W4, §4.4).
//
// THE COMMAND TREE. Every HQ is an entity: a command post (a land unit or an
// installation) that can be found, jammed and destroyed, with a seat for its
// commander and staff cells. A scenario gives the tree; a game without one
// gets one HQ per side over all its units.
//
// ORDERS TRAVEL. A plan becomes FRAGOs, one message per subordinate, over
// the HQ's nets. A unit acts on its new orders only when its message
// arrives, and the runner hands them over as orders issued while paused are
// (D0 if it is in a fight). A sub-HQ receives guidance instead, and plans.
//
// PLANNING TAKES SIM-TIME. On its battle rhythm, or when a CCIR trigger fires
// (objective taken or lost, a subordinate ineffective, a major enemy force
// found), an HQ starts a plan; it is ready after the planning time its
// echelon needs. The heuristic writes it then; Claude writes it through the
// runner, the clock waiting at `readyAt` if it is late (seat.ts).
//
// OUT OF COMMS. HQs send their intent down now and then; units report. A
// unit that has heard nothing from its HQ for a while acts on intent through
// its template's `outOfComms` decision.
//
// THE HQ CAN BE HIT. When its post is destroyed, the seat moves — to the
// alternate HQ the orders name, or the senior surviving subordinate — after
// a delay, during which the HQ sees nothing new and sends nothing.

import type { LatLng } from "../../../lib/board";
import { SIDES } from "../../../lib/state";
import type { RtState, UnitOrders } from "../../engine/types";
import type { Echelon } from "../../scope";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { onCreateWorld, type HqSpec } from "../create";
import { wfig } from "../figures";
import { distanceM, inPolygon, offsetBy, bearingDeg } from "../geo";
import { deliveryDelayS, nodeOf, onDelivered, sendNet, stepNet } from "../net";
import { raiseQuestion } from "../questions";
import { registerSystem } from "../step";
import { fuseAll, reportUp, subtree } from "../command/fusion";
import { heuristicPlan } from "../command/heuristicPlanner";
import { setFireRouting } from "./fires";
import type { Headquarters, PlanAnswer, PlanRequest, StructuredOrder, WorldState } from "../types";

// ── The tree ─────────────────────────────────────────────────────────────

const RHYTHM_KEY: Record<Echelon, "world/domains/c2.RHYTHM_COMPANY_S" | "world/domains/c2.RHYTHM_BATTALION_S" | "world/domains/c2.RHYTHM_BRIGADE_S" | "world/domains/c2.RHYTHM_HIGH_S"> = {
  platoon: "world/domains/c2.RHYTHM_COMPANY_S",
  company: "world/domains/c2.RHYTHM_COMPANY_S",
  battalion: "world/domains/c2.RHYTHM_BATTALION_S",
  brigade: "world/domains/c2.RHYTHM_BRIGADE_S",
  division: "world/domains/c2.RHYTHM_HIGH_S",
  corps: "world/domains/c2.RHYTHM_HIGH_S",
  component: "world/domains/c2.RHYTHM_HIGH_S",
  joint: "world/domains/c2.RHYTHM_HIGH_S",
};
const PLAN_KEY: Record<Echelon, "world/domains/c2.PLAN_COMPANY_S" | "world/domains/c2.PLAN_BATTALION_S" | "world/domains/c2.PLAN_BRIGADE_S" | "world/domains/c2.PLAN_HIGH_S"> = {
  platoon: "world/domains/c2.PLAN_COMPANY_S",
  company: "world/domains/c2.PLAN_COMPANY_S",
  battalion: "world/domains/c2.PLAN_BATTALION_S",
  brigade: "world/domains/c2.PLAN_BRIGADE_S",
  division: "world/domains/c2.PLAN_HIGH_S",
  corps: "world/domains/c2.PLAN_HIGH_S",
  component: "world/domains/c2.PLAN_HIGH_S",
  joint: "world/domains/c2.PLAN_HIGH_S",
};
export const rhythmS = (e: Echelon) => wfig(RHYTHM_KEY[e]);
export const planningS = (e: Echelon) => wfig(PLAN_KEY[e]);

function hqFrom(spec: HqSpec): Headquarters {
  return {
    id: spec.id,
    side: spec.side,
    echelon: spec.echelon,
    postId: spec.postId,
    ...(spec.parent ? { parent: spec.parent } : {}),
    subordinates: [...spec.subordinates],
    ...(spec.alternate ? { alternate: spec.alternate } : {}),
    ...(spec.planner ? { planner: spec.planner } : {}),
    seat: spec.seat ?? "heuristic",
    lastPlannedAt: -Infinity,
    nextPlanAt: 0,
    inbox: [],
    cop: {},
    cells: { fires: [], sustainment: [], isr: [] },
  };
}

onCreateWorld(10, (world, state, config, setup) => {
  const specs: HqSpec[] = setup.hqs ?? autoTree(world, state, config.scope?.echelons.highest ?? "company", config.scope?.planners ?? ["tactical"]);
  for (const spec of specs) world.hqs[spec.id] = hqFrom(spec);
  for (const hq of Object.values(world.hqs)) {
    for (const sub of hq.subordinates) if (world.land[sub]) world.land[sub] = { ...world.land[sub], hq: hq.id };
    if (world.land[hq.postId]) world.land[hq.postId] = { ...world.land[hq.postId], hq: hq.parent ?? hq.id };
  }
});

/** No tree in the scenario: one HQ per side, over every unit, in the unit with a command rating (or the first). */
function autoTree(world: WorldState, state: RtState, echelon: Echelon, planners: readonly string[]): HqSpec[] {
  if (echelon === "platoon") return [];
  const out: HqSpec[] = [];
  for (const side of SIDES) {
    const own = Object.values(state.game.forceElements).filter((fe) => fe.side === side && fe.combatStrength > 0).sort((a, b) => a.id.localeCompare(b.id));
    if (!own.length) continue;
    const post =
      own.find((fe) => world.land[fe.id]?.archetype.startsWith("generic/cp-")) ?? own.find((fe) => (fe.commandRating ?? 0) > 0) ?? own[0];
    out.push({
      id: `${side}-hq`,
      side,
      echelon: (["company", "battalion", "brigade", "division"] as Echelon[]).includes(echelon) ? echelon : "company",
      postId: post.id,
      subordinates: own.filter((fe) => fe.id !== post.id).map((fe) => fe.id),
      // A company or a skirmish is commanded by the player's commander, as
      // before: its HQ relays and fuses, but plans only if a planner sits there.
      ...(planners.includes("land") || planners.includes("overall") ? { planner: "land" as const } : {}),
      seat: "heuristic",
    });
  }
  return out;
}

function setHq(c: WorldCtx, id: string, patch: Partial<Headquarters>): void {
  const was = c.world.hqs[id];
  if (was) c.world.hqs[id] = { ...was, ...patch };
}

/** The live HQ commanding this unit. */
export function hqOf(c: Pick<WorldCtx, "world">, unitId: string): Headquarters | undefined {
  const id = c.world.land[unitId]?.hq ?? Object.values(c.world.hqs).find((h) => h.subordinates.includes(unitId))?.id;
  return id ? c.world.hqs[id] : undefined;
}

// ── Messages arriving ────────────────────────────────────────────────────

function toInbox(c: WorldCtx, m: { id: number; to: string }): void {
  for (const hq of Object.values(c.world.hqs)) {
    if (hq.postId === m.to && !hq.down) setHq(c, hq.id, { inbox: [...hq.inbox, m.id].slice(-wfig("world/domains/c2.INBOX_KEEP")) });
  }
}

/** A unit hears from its HQ: it is in comms. */
function heard(c: WorldCtx, unitId: string): void {
  if (c.comps(unitId)) c.setLand(unitId, { lastHqContactAt: c.time });
}

onDelivered("intent", (c, m) => {
  heard(c, m.to);
  // A sub-HQ's post receiving guidance: its HQ takes it, and plans to it.
  const sub = Object.values(c.world.hqs).find((h) => h.postId === m.to && h.parent && c.world.hqs[h.parent]?.postId === m.from);
  if (sub && m.body.guidance) {
    setHq(c, sub.id, { guidance: m.body.guidance as Headquarters["guidance"] });
    startPlan(c, c.world.hqs[sub.id], "new guidance from above");
  }
  // Each unit answers its HQ with a situation report.
  if (c.landAlive(m.to)) {
    const unit = c.land.units[m.to];
    sendNet(c, { side: m.side, kind: "sitrep", from: m.to, to: m.from, text: `${m.to}: ${unit.vehicles.fit}/${unit.vehicles.total} fit, ${unit.cohesion}`, body: { fit: unit.vehicles.fit, total: unit.vehicles.total, cohesion: unit.cohesion }, ...paceFor(c, m.to) });
  }
});
onDelivered("sitrep", (c, m) => toInbox(c, m));
onDelivered("logstat", (c, m) => toInbox(c, m));
onDelivered("spotrep", (c, m) => toInbox(c, m));
onDelivered("request", (c, m) => toInbox(c, m));
onDelivered("bda", (c, m) => toInbox(c, m));
onDelivered("frago", (c, m) => {
  heard(c, m.to);
  const orders = m.body.orders as UnitOrders | undefined;
  if (orders && c.landAlive(m.to)) {
    c.world.orderQueue.push({ unitId: m.to, orders, dueAt: c.time, messageId: m.id, ...(m.body.plan ? { plan: m.body.plan as string } : {}) });
    const structured = m.body.structured as StructuredOrder | undefined;
    if (structured) {
      c.world.structured[m.to] = structured;
      c.setLand(m.to, { emcon: structured.emcon, weaponControl: structured.weaponControl });
    }
  }
});

function paceFor(c: WorldCtx, id: string): { pace?: StructuredOrder["pace"] } {
  const s = c.world.structured[id];
  return s?.pace?.length ? { pace: s.pace } : {};
}

// ── Plans ────────────────────────────────────────────────────────────────

function startPlan(c: WorldCtx, hq: Headquarters, why: string, hasty = false): void {
  if (hq.down || hq.planning) return;
  // A hasty plan (a FRAGO on a crisis) takes minutes, not the full planning time.
  const readyAt = c.time + (hasty ? wfig("world/domains/c2.HASTY_PLAN_S") : planningS(hq.echelon));
  setHq(c, hq.id, { planning: { startedAt: c.time, readyAt, why }, nextPlanAt: c.time + rhythmS(hq.echelon) });
  const request: PlanRequest = { id: c.newNumber(), hq: hq.id, side: hq.side, planner: hq.planner ?? "tactical", seat: hq.seat, raisedAt: c.time, readyAt, why };
  c.world.plans.push(request);
  c.log({ side: hq.side, domain: "c2", kind: "planning", text: `${hq.id} starts planning (${why}); ready in ${Math.round((readyAt - c.time) / 60)} min`, ids: [hq.id] });
}

/** Send a plan out: FRAGOs to units, guidance to sub-HQs, over the nets. */
export function issuePlan(c: WorldCtx, request: PlanRequest, answer: PlanAnswer): void {
  const hq = c.world.hqs[request.hq];
  const i = c.world.plans.findIndex((p) => p.id === request.id);
  if (i >= 0) c.world.plans.splice(i, 1);
  if (!hq) return;
  setHq(c, hq.id, { planning: undefined, lastPlannedAt: c.time });
  if (hq.down) return;
  if (answer.reserve) c.world.memo[`reserveOf:${hq.id}:${answer.reserve}`] = c.time;
  for (const [unitId, orders] of Object.entries(answer.orders)) {
    if (!hq.subordinates.includes(unitId) && unitId !== hq.postId) continue;
    const structured = answer.structured?.[unitId];
    if (unitId === hq.postId) {
      c.world.orderQueue.push({ unitId, orders, dueAt: c.time, messageId: 0, plan: answer.plan });
      continue;
    }
    sendNet(c, { side: hq.side, kind: "frago", from: hq.postId, to: unitId, text: `FRAGO from ${hq.id}: ${orders.task}`, body: { orders, plan: answer.plan, ...(structured ? { structured } : {}) }, ...paceFor(c, hq.postId) });
  }
  for (const sub of hq.subordinates) {
    const subHq = c.world.hqs[sub];
    if (subHq && answer.guidance) sendNet(c, { side: hq.side, kind: "intent", from: hq.postId, to: subHq.postId, text: `guidance from ${hq.id}`, body: { guidance: answer.guidance } });
  }
  c.log({ side: hq.side, domain: "c2", kind: "plan", text: `${hq.id} issues its plan (${answer.by}): ${answer.plan}`, ids: [hq.id] });
}

function hashOf(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return h;
}

/** The reserve the HQ's last plan named, if it is still fresh. */
export function reserveOf(c: WorldCtx, hq: Headquarters): string | undefined {
  const keys = Object.keys(c.world.memo).filter((k) => k.startsWith(`reserveOf:${hq.id}:`));
  const latest = keys.sort((a, b) => c.world.memo[b] - c.world.memo[a])[0];
  const id = latest?.slice(`reserveOf:${hq.id}:`.length);
  return id && c.landAlive(id) && c.world.memo[`committed:${id}`] == null ? id : undefined;
}

function plans(c: WorldCtx): void {
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    if (hq.down || !hq.planner) continue;
    if (!hq.planning && c.time >= hq.nextPlanAt) startPlan(c, hq, hq.lastPlannedAt === -Infinity ? "initial plan" : "battle rhythm");
  }
  // Heuristic seats answer inside the world, at readyAt.
  for (const request of [...c.world.plans]) {
    if (request.seat !== "heuristic" || request.readyAt > c.time) continue;
    const hq = c.world.hqs[request.hq];
    if (!hq) continue;
    issuePlan(c, request, heuristicPlan({ ...c.land.prev, game: c.game, units: c.land.units, world: c.world }, hq));
  }
}

// ── CCIR: what makes an HQ plan again now ────────────────────────────────

function ccir(c: WorldCtx): void {
  for (const id of Object.keys(c.world.hqs).sort()) {
    let hq = c.world.hqs[id];
    if (hq.down || !hq.planner) continue;
    // Heavy losses: the commander decides at once whether the attack goes on — a hasty plan, whatever the gap,
    // dropping a deliberate plan still being made.
    const fleet = hq.subordinates.filter((s) => c.land.units[s]);
    const total = fleet.reduce((s, u) => s + c.land.units[u].vehicles.total, 0);
    const lost = total ? 1 - fleet.reduce((s, u) => s + c.land.units[u].vehicles.fit, 0) / total : 0;
    if (lost >= wfig("world/command/heuristicPlanner.CULMINATION_LOSS") && c.world.memo[`heavyLosses:${hq.id}`] == null) {
      c.mark(`heavyLosses:${hq.id}`);
      if (hq.planning) {
        c.world.plans = c.world.plans.filter((p) => p.hq !== hq.id);
        setHq(c, hq.id, { planning: undefined });
        hq = c.world.hqs[id];
      }
      startPlan(c, hq, `CCIR: ${Math.round(lost * 100)}% losses`, true);
      continue;
    }
    if (hq.planning) continue;
    if (c.time - hq.lastPlannedAt < wfig("world/domains/c2.CCIR_MIN_GAP_S")) continue;
    // A major enemy force found: several new contacts in the COP since the last plan.
    const fresh = Object.values(hq.cop).filter((t) => t.domain === "land" && t.seenAt > hq.lastPlannedAt).length;
    if (fresh >= wfig("world/domains/c2.CCIR_NEW_CONTACTS")) {
      startPlan(c, hq, `CCIR: ${fresh} new enemy contacts`);
      continue;
    }
    // The objective taken.
    const objective = hq.guidance?.objective ?? c.game.objectives?.[hq.side];
    if (objective && c.world.memo[`objTaken:${hq.id}`] == null) {
      const on = hq.subordinates.some((s) => c.landAlive(s) && distanceM(c.fe(s)!.position, objective) <= 300);
      const clear = Object.values(hq.cop).every((t) => t.domain !== "land" || distanceM(t.at, objective) > 500);
      if (on && clear) {
        c.mark(`objTaken:${hq.id}`);
        startPlan(c, hq, "CCIR: objective taken");
      }
    }
  }
}

// ── Heartbeat, out of comms, and the post's own exposure ────────────────

function heartbeats(c: WorldCtx): void {
  const every = wfig("world/domains/c2.HEARTBEAT_S");
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    if (hq.down || (c.time + hashOf(id)) % every !== 0) continue;
    for (const sub of hq.subordinates) {
      const to = c.world.hqs[sub]?.postId ?? sub;
      if (to === hq.postId) continue;
      sendNet(c, { side: hq.side, kind: "intent", from: hq.postId, to, text: `${hq.id}: intent stands`, body: {}, ...paceFor(c, hq.postId) });
    }
  }
}

function outOfComms(c: WorldCtx): void {
  if (c.time % 30 !== 0) return;
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!comps.hq || !c.landAlive(id) || c.world.hqs[comps.hq]?.postId === id) continue;
    const unit = c.land.units[id];
    if (unit.cohesion !== "steady" || !unit.orders) continue;
    if (c.time - comps.lastHqContactAt < wfig("world/domains/c2.OUT_OF_COMMS_S")) continue;
    if (c.time - c.last(`ooc:${id}`) < wfig("world/domains/c2.OUT_OF_COMMS_S")) continue;
    c.mark(`ooc:${id}`);
    c.log({ side: c.fe(id)!.side, domain: "c2", kind: "outOfComms", text: `${id} has heard nothing from ${comps.hq} for ${Math.round((c.time - comps.lastHqContactAt) / 60)} min`, ids: [id] });
    raiseQuestion(c, { agentId: id, side: c.fe(id)!.side, trigger: "outOfComms", about: { hq: comps.hq }, quality: c.fe(id)!.troopQuality });
  }
}

function postExposure(c: WorldCtx): void {
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    const comps = c.comps(hq.postId);
    if (hq.down || !comps) continue;
    if (c.land.units[hq.postId]?.lastMovedAt === c.time && comps.onAirS > 0) c.setLand(hq.postId, { onAirS: 0 });
    if (comps.onAirS < wfig("world/domains/c2.EMIT_LIMIT_S") || c.time - c.last(`emit:${id}`) < 900) continue;
    c.mark(`emit:${id}`);
    raiseQuestion(c, { agentId: hq.postId, side: hq.side, trigger: "emittingTooLong", about: { hq: id }, quality: c.fe(hq.postId)?.troopQuality ?? 4 });
  }
}

// ── Staff cells ──────────────────────────────────────────────────────────

function staff(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    if (hq.down || !c.landAlive(hq.postId)) continue;
    // A subordinate no longer able to fight as it should.
    for (const sub of hq.subordinates) {
      if (!c.landAlive(sub) || c.world.memo[`ineffective:${sub}`] != null) continue;
      const fe = c.fe(sub)!;
      const unit = c.land.units[sub];
      if (fe.combatStrength > fe.combatStrengthStart * 0.5 && unit.cohesion !== "broken") continue;
      c.mark(`ineffective:${sub}`);
      raiseQuestion(c, { agentId: hq.postId, side: hq.side, trigger: "subordinateIneffective", about: { hq: id, unit: sub }, quality: c.fe(hq.postId)!.troopQuality });
      const reserve = reserveOf(c, hq);
      if (reserve) raiseQuestion(c, { agentId: hq.postId, side: hq.side, trigger: "commitReserve", about: { hq: id, unit: sub, reserve }, quality: c.fe(hq.postId)!.troopQuality });
    }
    // More calls for fire waiting than shooters to take them.
    const waiting = c.world.missions.filter((m) => m.side === hq.side && m.state === "requested" && subtree(c, id).includes(m.requestedBy));
    if (waiting.length >= 2 && c.time - c.last(`fires:${id}`) > wfig("world/domains/c2.FIRES_RANK_S")) {
      c.mark(`fires:${id}`);
      raiseQuestion(c, { agentId: hq.postId, side: hq.side, trigger: "fireRequestsCompeting", about: { hq: id }, quality: c.fe(hq.postId)!.troopQuality });
    }
  }
}

// ── The HQ can be hit ────────────────────────────────────────────────────

function posts(c: WorldCtx): void {
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    const alive = c.landAlive(hq.postId) || (c.entity(hq.postId)?.status === "active");
    if (!hq.down && !alive) {
      const movesAt = c.time + wfig("world/domains/c2.SEAT_MOVE_S");
      setHq(c, id, { down: { since: c.time, movesAt }, planning: undefined, cop: {} });
      for (let i = c.world.plans.length - 1; i >= 0; i--) if (c.world.plans[i].hq === id) c.world.plans.splice(i, 1);
      c.log({ side: hq.side, domain: "c2", kind: "hqLost", text: `${id}'s command post (${hq.postId}) is lost; the seat moves in ${Math.round(wfig("world/domains/c2.SEAT_MOVE_S") / 60)} min`, ids: [id, hq.postId] });
      continue;
    }
    if (hq.down && c.time >= hq.down.movesAt) {
      const alt = hq.alternate && c.world.hqs[hq.alternate] && !c.world.hqs[hq.alternate].down ? c.world.hqs[hq.alternate].postId : undefined;
      const senior = hq.subordinates.filter((s) => c.landAlive(s)).sort((a, b) => (c.fe(b)!.commandRating ?? 0) - (c.fe(a)!.commandRating ?? 0) || a.localeCompare(b))[0];
      const post = alt ?? senior;
      if (!post) continue;
      setHq(c, id, { postId: post, down: undefined, subordinates: hq.subordinates.filter((s) => s !== post), nextPlanAt: c.time });
      c.log({ side: hq.side, domain: "c2", kind: "seatMoved", text: `${id}'s seat moves to ${post}`, ids: [id, post] });
    }
  }
}

// ── Structured orders in force: phase triggers, branches, boundaries ─────

function structuredOrders(c: WorldCtx): void {
  if (c.time % 5 !== 0) return;
  for (const [unitId, so] of Object.entries(c.world.structured)) {
    if (!c.landAlive(unitId)) continue;
    const unit = c.land.units[unitId];
    const fe = c.fe(unitId)!;
    if (unit.cohesion !== "steady" || !unit.orders) continue;
    // A boundary polygon: a unit outside it is brought back in.
    if (so.boundary && so.boundary.length >= 3 && !inPolygon(fe.position, so.boundary) && unit.order.kind !== "withdraw") {
      const centre = { lat: so.boundary.reduce((s, p) => s + p.lat, 0) / so.boundary.length, lng: so.boundary.reduce((s, p) => s + p.lng, 0) / so.boundary.length };
      c.land.order(unitId, { kind: "move", to: offsetBy(fe.position, bearingDeg(fe.position, centre), 150), mode: "tactical" });
      c.land.emit(unitId, "blocked", "outside its boundary: moving back in", false, { info: true });
    }
    // The next phase waits for its trigger; a branch replaces the phases when it fires.
    const next = unit.orders.phase + 1;
    const trigger = so.phaseTriggers[next];
    if (trigger && triggered(c, trigger, unitId) && unit.order.phase === unit.orders.phase && unit.orders.phases[next]) {
      c.land.setUnit(unitId, { orders: { ...unit.orders, phase: next } });
      c.land.order(unitId, { ...unit.orders.phases[next].order, phase: next });
      so.phaseTriggers[next] = null;
    }
    for (const branch of so.branches) {
      if (!triggered(c, branch.when, unitId)) continue;
      c.land.setUnit(unitId, { orders: { ...unit.orders, phases: branch.phases, phase: 0 } });
      if (branch.phases[0]) c.land.order(unitId, { ...branch.phases[0].order, phase: 0 });
      c.world.structured[unitId] = { ...so, branches: so.branches.filter((b) => b !== branch), phaseTriggers: branch.phases.map(() => null) };
      c.log({ side: fe.side, domain: "c2", kind: "branch", text: `${unitId} takes branch "${branch.label}"`, ids: [unitId] });
      break;
    }
  }
}

function triggered(c: WorldCtx, t: NonNullable<StructuredOrder["phaseTriggers"][number]>, unitId: string): boolean {
  const fe = c.fe(unitId)!;
  switch (t.kind) {
    case "time":
      return c.time >= t.at;
    case "position":
      return distanceM(fe.position, t.at) <= t.withinM;
    case "enemySeen": {
      const n = Object.keys(c.land.units[unitId].ownSeen).filter((e) => c.landAlive(e) && distanceM(c.fe(e)!.position, fe.position) <= t.withinM).length;
      return n >= t.atLeast;
    }
    case "event":
      if (t.event === "lanesOpen") return c.world.obstacles.some((o) => o.lanes.length > 0);
      if (t.event === "objectiveTaken") return Object.keys(c.world.memo).some((k) => k.startsWith("objTaken:") && c.world.hqs[k.slice(9)]?.side === (t.side ?? fe.side));
      if (t.event === "reserveCommitted") return Object.keys(c.world.memo).some((k) => k.startsWith("committed:"));
      return false;
  }
}

// ── Handlers ─────────────────────────────────────────────────────────────

const hqAbout = (c: WorldCtx, q: { about: Record<string, unknown> }) => c.world.hqs[q.about.hq as string];

registerHandlers({
  "c2.carryOnPhase": { apply: () => {} },
  "c2.actOnIntent": {
    available: (c, q) => {
      const o = c.land.units[q.agentId]?.orders;
      return o != null && o.phase + 1 < o.phases.length;
    },
    apply(c, q) {
      const o = c.land.units[q.agentId].orders!;
      const next = o.phase + 1;
      c.land.setUnit(q.agentId, { orders: { ...o, phase: next } });
      c.land.order(q.agentId, { ...o.phases[next].order, phase: next });
    },
  },
  "c2.holdAndReestablish": { apply: (c, q) => c.land.setUnit(q.agentId, { order: { kind: "hold" }, bound: undefined }) },
  "c2.moveToRegainComms": {
    available: (c, q) => {
      const hq = c.world.hqs[q.about.hq as string];
      return hq != null && nodeOf(c, hq.postId) != null;
    },
    apply(c, q) {
      const hq = c.world.hqs[q.about.hq as string];
      const post = c.where(hq.postId);
      const fe = c.fe(q.agentId);
      if (!post || !fe) return;
      const d = distanceM(fe.position, post);
      c.land.order(q.agentId, { kind: "move", to: offsetBy(post, bearingDeg(post, fe.position), Math.min(d, wfig("world/net.VHF_RANGE_M") * 0.6)), mode: "tactical" });
    },
  },
  "c2.fireFirstRequest": {
    expand(c, q) {
      const hq = hqAbout(c, q);
      if (!hq) return [];
      const mine = new Set(subtree(c, hq.id));
      return c.world.missions
        .filter((m) => m.side === hq.side && m.state === "requested" && mine.has(m.requestedBy))
        .slice(0, 4)
        .map((m) => ({ key: m.id, summary: `fire first for ${m.requestedBy}${m.targetId ? ` on ${m.targetId}` : ""}${m.reason ? ` (${m.reason})` : ""}`, params: { missionId: m.id } }));
    },
    apply(c, _q, option) {
      const i = c.world.missions.findIndex((m) => m.id === option.params?.missionId);
      if (i >= 0) c.world.missions[i] = { ...c.world.missions[i], priority: c.world.missions[i].priority + 5 };
    },
  },
  "c2.commitReserve": {
    available: (c, q) => c.landAlive(q.about.reserve as string),
    describe: (_c, q) => `commit the reserve (${q.about.reserve}) to ${q.about.unit}`,
    apply(c, q) {
      const reserve = q.about.reserve as string;
      const at = c.where(q.about.unit as string) ?? c.where(reserve)!;
      c.land.order(reserve, { kind: "move", to: at, mode: "tactical" });
      c.land.setUnit(reserve, { mission: { task: "take", at, purpose: `reinforce ${q.about.unit}` } });
      c.mark(`committed:${reserve}`);
      c.log({ side: q.side, domain: "c2", kind: "reserve", text: `the reserve (${reserve}) is committed to ${q.about.unit}`, ids: [reserve] });
    },
  },
  "c2.holdReserve": { apply: () => {} },
  "c2.reinforce": {
    available: (c, q) => {
      const hq = hqAbout(c, q);
      return hq != null && reserveOf(c, hq) != null;
    },
    apply(c, q) {
      const hq = hqAbout(c, q);
      const reserve = hq && reserveOf(c, hq);
      if (!reserve) return;
      const at = c.where(q.about.unit as string)!;
      c.land.order(reserve, { kind: "move", to: at, mode: "tactical" });
      c.mark(`committed:${reserve}`);
    },
  },
  "c2.swapWithReserve": {
    available: (c, q) => {
      const hq = hqAbout(c, q);
      return hq != null && reserveOf(c, hq) != null;
    },
    apply(c, q) {
      const hq = hqAbout(c, q);
      const reserve = hq && reserveOf(c, hq);
      const unit = q.about.unit as string;
      if (!reserve || !c.landAlive(unit)) return;
      const there = c.where(unit)!;
      const back = c.where(reserve)!;
      c.land.order(reserve, { kind: "move", to: there, mode: "tactical" });
      c.land.order(unit, { kind: "withdraw", to: back });
      c.mark(`committed:${reserve}`);
    },
  },
  "c2.withdrawIt": {
    available: (c, q) => c.landAlive(q.about.unit as string),
    apply(c, q) {
      const hq = hqAbout(c, q);
      const unit = q.about.unit as string;
      const to = (hq && c.where(hq.postId)) ?? c.world.startLines[q.side];
      c.land.order(unit, { kind: "withdraw", to });
    },
  },
  "c2.displacePost": {
    apply(c, q) {
      const fe = c.fe(q.agentId);
      if (!fe) return;
      c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, (bearingDeg(c.world.startLines[fe.side], fe.position) + 180) % 360, wfig("world/domains/c2.DISPLACE_M")), mode: "march" });
      c.setLand(q.agentId, { onAirS: 0 });
    },
  },
  "c2.alternateBearer": {
    apply(c, q) {
      const was = c.world.structured[q.agentId];
      const pace = ["datalink", "satcomMil", "satcomLeo", "hf", "vhf", "courier"] as StructuredOrder["pace"];
      c.world.structured[q.agentId] = { ...(was ?? { mission: "command", intent: "", phaseTriggers: [], branches: [], measures: [], weaponControl: "tight", emcon: "limited", issuedAt: c.time, by: "rules" }), pace };
      c.setLand(q.agentId, { onAirS: 0, emcon: "limited" });
    },
  },
  "c2.acceptRisk": { apply: () => {} },
});

registerRules({
  outOfComms: (c, q, options) => {
    const unit = c.land.units[q.agentId];
    const o = unit?.orders;
    // Waiting for a word that will not come, or done with what it was told: act on the intent. On the move: carry on.
    const waiting = unit != null && ["hold", "overwatch", "observe"].includes(unit.order.kind);
    return o && (waiting || unit.order.phase !== o.phase) ? prefer(options, "c2.actOnIntent", "c2.carryOnPhase") : prefer(options, "c2.carryOnPhase");
  },
  subordinateIneffective: (c, q, options) => (c.land.units[q.about.unit as string]?.cohesion === "broken" ? prefer(options, "c2.withdrawIt") : prefer(options, "c2.swapWithReserve", "c2.reinforce", "c2.withdrawIt")),
  commitReserve: (_c, _q, options) => prefer(options, "c2.commitReserve"),
  emittingTooLong: (_c, _q, options) => prefer(options, "c2.displacePost", "c2.alternateBearer"),
  fireRequestsCompeting: (c, _q, options) => {
    // The rules rank by priority, then the oldest.
    const ranked = [...options].sort((a, b) => {
      const ma = c.world.missions.find((m) => m.id === a.params?.missionId);
      const mb = c.world.missions.find((m) => m.id === b.params?.missionId);
      return (mb?.priority ?? 0) - (ma?.priority ?? 0) || (ma?.requestedAt ?? 0) - (mb?.requestedAt ?? 0);
    });
    return ranked[0]?.id;
  },
});

// Calls for fire go to the fires cell and the tasking back down, over the nets.
setFireRouting((c, m) => {
  const hq = hqOf(c, m.requestedBy);
  if (!hq) return 0;
  const up = deliveryDelayS(c, m.requestedBy, hq.postId);
  return up == null ? wfig("world/net.COURIER_SETUP_S") : up + deliveryDelayS(c, hq.postId, hq.postId)! + wfig("world/domains/c2.FIRES_CELL_S");
});

registerSystem({
  name: "c2",
  order: 20,
  when: (c) => Object.keys(c.world.hqs).length > 0 || c.world.messages.length > 0,
  run(c) {
    stepNet(c);
    posts(c);
    if (c.time % wfig("world/domains/c2.COP_S") === 0) fuseAll(c);
    reportUp(c, sendNet);
    heartbeats(c);
    outOfComms(c);
    postExposure(c);
    ccir(c);
    plans(c);
    staff(c);
    structuredOrders(c);
  },
});

export type { LatLng };
