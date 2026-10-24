// ── world/command/claudePlanner.ts ───────────────────────────────────────
// Claude in an HQ's seat (MULTI_DOMAIN_PLAN.md §4.4.2, W4).
//
// TACTICAL AND LAND PLANNERS write mission orders for the HQ's own
// subordinates, through the same prompt and the same strict parser as the
// player's commander (engine/orders.ts) — told the HQ's picture and the
// guidance from above, and only the HQ's units' orders are kept.
//
// THE OVERALL PLANNER writes JointGuidance: aims, main effort, priorities,
// apportionment of shared means, coordination measures and limits (ROE,
// escalation). It is validated as strictly as orders are: an apportionment
// above what exists is cut back, and the overage becomes a request to the
// joint HQ (§4.4.2 point 1); a ROE or escalation level it does not know is
// refused. Each planner falls back to the heuristic when Claude cannot be
// reached or read, as `commanderOrders` does.

import type { Side } from "../../../lib/state";
import { commanderOrders } from "../../engine/orders";
import type { RtConfig, RtState } from "../../engine/types";
import type { CommanderSeat, PlannerSeat } from "./seat";
import { heuristicPlan } from "./heuristicPlanner";
import type { Guidance, PlanAnswer, PlanRequest } from "../types";

export type ModelCall = (prompt: string) => Promise<string>;

/** The HQ's picture, in words, for the prompt: what reached it, and how old. */
export function pictureWords(seat: CommanderSeat, time: number): string {
  const tracks = Object.values(seat.cop()).sort((a, b) => b.seenAt - a.seenAt).slice(0, 25);
  if (!tracks.length) return "Nothing about the enemy has reached this HQ.";
  return tracks
    .map((t) => `${t.id}: ${t.thought} (${t.domain}), seen ${Math.round((time - t.seenAt) / 60)} min ago by ${t.by}, quality ${t.quality}/15${t.errorM > 200 ? `, may be ${Math.round(t.errorM)} m off` : ""}`)
    .join("\n");
}

/** A tactical or land planner: Claude's mission orders for this HQ's units. */
export function claudeLandPlanner(call: ModelCall, config: RtConfig): PlannerSeat {
  return {
    name: "claude-land",
    async plan(state: RtState, request: PlanRequest, seat: CommanderSeat): Promise<PlanAnswer> {
      const hq = state.world!.hqs[request.hq];
      const guidance = [
        `You are the ${hq.echelon} HQ ${hq.id}. You plan for your own subordinates only: ${hq.subordinates.join(", ")}.`,
        hq.guidance ? `Guidance from ${hq.guidance.by}: ${hq.guidance.aims.join("; ")}. Priorities: ${hq.guidance.priorities.join(", ")}.` : "",
        `Why you are planning now: ${request.why}.`,
        `What has reached your HQ:\n${pictureWords(seat, state.time)}`,
      ]
        .filter(Boolean)
        .join("\n");
      const result = await commanderOrders(call, state, request.side, config, { guidance, log: false });
      const mine = new Set([...hq.subordinates, hq.postId]);
      const orders = Object.fromEntries(Object.entries(result.orders).filter(([id]) => mine.has(id)));
      if (!Object.keys(orders).length) return { ...heuristicPlan(state, hq), warnings: [...result.warnings, "Claude gave no orders for this HQ's units: the heuristic planned"] };
      const fallback = heuristicPlan(state, hq);
      return { by: result.by === "claude" ? "claude" : "heuristic", plan: result.plan, orders, ...(fallback.guidance ? { guidance: fallback.guidance } : {}), warnings: result.warnings };
    },
  };
}

/** What the joint HQ has to apportion: the shared means that exist, by type. */
export function shareableMeans(state: RtState, side: Side): Record<string, number> {
  const entities = Object.values(state.world?.entities ?? {}).filter((e) => e.owner === side && e.status !== "destroyed");
  const count = (pred: (archetype: string) => boolean) => entities.filter((e) => pred(e.archetype)).reduce((s, e) => s + e.fit, 0);
  return {
    sortiesPerDay: count((a) => /fighter|attackHelo/.test(a)) * 2,
    droneHours: count((a) => /male|quadIsr|loiter/.test(a)) * 12,
    longRangeFires: Object.values(state.world?.land ?? {}).filter((l) => /mlrs|sph155/.test(l.archetype)).length * 6,
    satellitePasses: count((a) => /satEo|satSar/.test(a)) * 14,
  };
}

/** The prompt for the overall (joint) planner. */
export function jointGuidancePrompt(state: RtState, request: PlanRequest, seat: CommanderSeat): string {
  const hq = state.world!.hqs[request.hq];
  const means = shareableMeans(state, request.side);
  return [
    `You are the overall (joint) commander for ${request.side}, at ${hq.id}. Write JointGuidance for your component commanders: ${hq.subordinates.join(", ")}.`,
    `Shared means you can apportion: ${Object.entries(means).map(([k, v]) => `${k} ${v}`).join(", ")}.`,
    `What has reached your HQ:\n${pictureWords(seat, state.time)}`,
    `Reply with JSON only: {"aims": [..], "mainEffort": "<component HQ id>", "priorities": [..], "apportionment": {"<means>": <number>, ...}, "roe": {"weaponsFree": true|false, "escalationLimit": "conventional"|"noStrategicStrike"|"noCrossBorder"}}`,
    "Escalation is yours to limit: choose the lowest level that achieves the aims.",
  ].join("\n");
}

const ESCALATION = ["conventional", "noStrategicStrike", "noCrossBorder"] as const;

/** Read and check JointGuidance. Overages become requests; anything unreadable is a warning, not a guess. */
export function parseJointGuidance(text: string, state: RtState, request: PlanRequest): { guidance: Guidance | null; warnings: string[]; requests: string[] } {
  const warnings: string[] = [];
  const requests: string[] = [];
  let raw: Record<string, unknown>;
  try {
    const start = text.indexOf("{");
    raw = JSON.parse(text.slice(start, text.lastIndexOf("}") + 1));
  } catch {
    return { guidance: null, warnings: ["the joint guidance was not JSON"], requests };
  }
  const hq = state.world!.hqs[request.hq];
  const strings = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : []);
  const aims = strings(raw.aims);
  if (!aims.length) warnings.push("no aims given");
  const means = shareableMeans(state, request.side);
  const apportionment: Record<string, number> = {};
  for (const [k, v] of Object.entries((raw.apportionment as Record<string, unknown>) ?? {})) {
    if (typeof v !== "number" || !(k in means)) {
      warnings.push(`apportionment "${k}" ignored`);
      continue;
    }
    apportionment[k] = Math.min(v, means[k]);
    if (v > means[k]) requests.push(`${k}: asked for ${v}, only ${means[k]} exist — the overage goes to the joint HQ as a request`);
  }
  const roeRaw = (raw.roe as Record<string, unknown>) ?? {};
  const escalation = ESCALATION.includes(roeRaw.escalationLimit as (typeof ESCALATION)[number]) ? (roeRaw.escalationLimit as (typeof ESCALATION)[number]) : "conventional";
  if (roeRaw.escalationLimit != null && escalation !== roeRaw.escalationLimit) warnings.push(`unknown escalation limit "${String(roeRaw.escalationLimit)}": conventional`);
  const mainEffort = typeof raw.mainEffort === "string" && hq.subordinates.includes(raw.mainEffort) ? raw.mainEffort : undefined;
  if (raw.mainEffort != null && !mainEffort) warnings.push(`main effort "${String(raw.mainEffort)}" is not a component HQ`);
  return {
    guidance: {
      issuedAt: state.time,
      by: hq.id,
      aims,
      ...(mainEffort ? { mainEffort } : {}),
      priorities: strings(raw.priorities),
      apportionment,
      roe: { weaponsFree: roeRaw.weaponsFree !== false, escalationLimit: escalation },
      measures: hq.guidance?.measures ?? [],
    },
    warnings,
    requests,
  };
}

/** The overall planner: Claude's JointGuidance, sent down to each component HQ. */
export function claudeOverallPlanner(call: ModelCall): PlannerSeat {
  return {
    name: "claude-overall",
    async plan(state, request, seat) {
      const hq = state.world!.hqs[request.hq];
      try {
        const reply = await call(jointGuidancePrompt(state, request, seat));
        const { guidance, warnings, requests } = parseJointGuidance(reply, state, request);
        if (guidance) return { by: "claude", plan: guidance.aims.join("; ") || "joint guidance", orders: {}, guidance, warnings: [...warnings, ...requests] };
        return { ...heuristicPlan(state, hq), warnings };
      } catch (err) {
        return { ...heuristicPlan(state, hq), warnings: [`the overall planner could not be reached (${err instanceof Error ? err.message : String(err)})`] };
      }
    },
  };
}

/** One planner seat per side that routes each request to the right planner by the HQ's planner type. */
export function claudePlanners(call: ModelCall, config: RtConfig): PlannerSeat {
  const land = claudeLandPlanner(call, config);
  const overall = claudeOverallPlanner(call);
  return {
    name: "claude",
    plan: (state, request, seat) => (request.planner === "overall" ? overall : land).plan(state, request, seat),
  };
}
