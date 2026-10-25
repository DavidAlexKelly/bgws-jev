// ── world/command/console.ts ─────────────────────────────────────────────
// The HQ console's view (MULTI_DOMAIN_PLAN.md W15): what a person commanding
// from this HQ can see, and nothing more — the HQ's seat (seat.ts) shaped
// for a screen. No truth: the inbox holds only what has ARRIVED, the picture
// is the HQ's fused COP with each track's age and error, comms are what the
// HQ can tell of its own nets, and orders sent show whether they arrived.
//
// Also the orders composer's helpers: a FRAGO for one unit, and a staff
// draft (the heuristic or Claude's plan) for the person to approve, edit or
// reject. Orders go out through the runner's `command` and its nets.

import type { LatLng } from "../../../lib/board";
import type { RtOrder, RtState, UnitOrders } from "../../engine/types";
import { deliveryDelayS } from "../net";
import { withWorldCtx } from "../standalone";
import type { RtConfig } from "../../engine/types";
import type { NetMessage, PlanAnswer } from "../types";
import { heuristicPlan } from "./heuristicPlanner";

export interface ConsoleView {
  hq: string;
  side: "blue" | "red";
  echelon: string;
  seat: string;
  /** The post is destroyed and the seat moving: nothing new comes in. */
  down: boolean;
  inbox: { id: number; from: string; kind: NetMessage["kind"]; text: string; sentAt: number; arrivedAt: number; bearer: string }[];
  /** CCIR alerts and planning prompts for this HQ, newest first. */
  alerts: { time: number; text: string }[];
  cop: { id: string; thought: string; at: LatLng; ageS: number; errorM: number; classification: string; domain: string }[];
  comms: { unit: string; reachable: boolean; delayS: number | null; lastHeardAgoS: number | null }[];
  sent: { id: number; to: string; text: string; sentAt: number; status: "on its way" | "arrived" | "lost"; arrivedAt?: number }[];
}

export function consoleView(state: RtState, config: RtConfig, hqId: string): ConsoleView | undefined {
  const w = state.world;
  const hq = w?.hqs[hqId];
  if (!w || !hq) return undefined;
  const now = state.time;
  const byId = new Map(w.messages.map((m) => [m.id, m]));
  const inbox = hq.down
    ? []
    : hq.inbox
        .map((id) => byId.get(id))
        .filter((m): m is NetMessage => m != null && !m.lost && m.dueAt <= now)
        .sort((a, b) => b.dueAt - a.dueAt)
        .map((m) => ({ id: m.id, from: m.from, kind: m.kind, text: m.text, sentAt: m.sentAt, arrivedAt: m.dueAt, bearer: m.bearer }));
  const alerts = w.log
    .filter((l) => l.domain === "c2" && l.ids?.includes(hqId) && /CCIR|planning|over to the commander|seat/.test(l.text))
    .slice(-12)
    .reverse()
    .map((l) => ({ time: l.time, text: l.text }));
  const cop = hq.down
    ? []
    : Object.values(hq.cop)
        .map((t) => ({ id: t.id, thought: t.thought, at: t.at, ageS: now - t.seenAt, errorM: t.errorM, classification: t.classification, domain: t.domain }))
        .sort((a, b) => a.ageS - b.ageS);
  // What the HQ can tell of its own nets: can a message get through to each subordinate now, and how long ago it last heard from it.
  const subs = hq.subordinates.map((s) => (w.hqs[s] ? w.hqs[s].postId : s));
  const delays = new Map<string, number | null>();
  withWorldCtx(state, config, (c) => {
    for (const s of subs) delays.set(s, state.game.forceElements[s] || w.entities[s] ? deliveryDelayS(c, hq.postId, s) : null);
  });
  const comms = subs.map((s) => {
    const last = inbox.find((m) => m.from === s);
    return { unit: s, reachable: delays.get(s) != null, delayS: delays.get(s) ?? null, lastHeardAgoS: last ? now - last.arrivedAt : null };
  });
  const sent = w.messages
    .filter((m) => m.from === hq.postId && (m.kind === "frago" || m.kind === "intent"))
    .slice(-20)
    .reverse()
    .map((m) => ({ id: m.id, to: m.to, text: m.text, sentAt: m.sentAt, status: (m.lost ? "lost" : m.dueAt <= now ? "arrived" : "on its way") as "lost" | "arrived" | "on its way", ...(m.dueAt <= now && !m.lost ? { arrivedAt: m.dueAt } : {}) }));
  return { hq: hqId, side: hq.side, echelon: hq.echelon, seat: hq.seat, down: !!hq.down, inbox, alerts, cop, comms, sent };
}

/** One unit's FRAGO from the composer: an order, a task in words, its rules of engagement. */
export function frago(state: RtState, order: RtOrder, task: string, roe: UnitOrders["roe"] = "withinShortRange"): UnitOrders {
  return { task, phases: [{ label: task, order }], phase: 0, intent: task, urgency: "whenAble", roe, onContact: "engage", boundaries: [], by: "commander", issuedAt: state.time };
}

/** The person's orders as a plan answer for `command`. */
export function commanderAnswer(plan: string, orders: Record<string, UnitOrders>, guidance?: PlanAnswer["guidance"]): PlanAnswer {
  return { by: "human", plan, orders, ...(guidance ? { guidance } : {}) };
}

/** A staff draft for the person to approve, edit or reject: the heuristic planner's plan from this HQ's picture. */
export function staffDraft(state: RtState, hqId: string): PlanAnswer | undefined {
  const hq = state.world?.hqs[hqId];
  return hq ? heuristicPlan(state, hq) : undefined;
}

/** Who the composer can send to: the HQ's own units (FRAGOs), and the HQs under it (guidance; they plan their own). */
export function commandable(state: RtState, hqId: string): { units: string[]; hqs: string[] } {
  const w = state.world;
  const hq = w?.hqs[hqId];
  if (!w || !hq) return { units: [], hqs: [] };
  return { units: hq.subordinates.filter((s) => !w.hqs[s] && (state.game.forceElements[s]?.combatStrength ?? 0) > 0), hqs: hq.subordinates.filter((s) => w.hqs[s]) };
}
