// ── bgws/realtime/engine/orders.ts ─────────────────────────────────────────
// Orders from the player's commander (Claude), written while the clock is
// stopped.
//
// docs/REALTIME_COMMAND_DESIGN.html §7. Claude runs only when the player
// pauses and presses "Generate orders" — before the start, or at any pause —
// never on events, never on a timer. It sees what its side knows and writes
// MISSION ORDERS for each unit:
//
//   task and phases   "advance to the ridge, then overwatch the bridge":
//                     what game logic carries out, phase after phase
//   intent            one line; Jev weighs every decision against it
//   urgency           "now" or "when able" (the default), for D0
//   constraints       rules of engagement and lines not to cross, enforced
//                     by game logic, never overridden by Jev
//   actions on contact  what the rules fall back to when Jev is unsure
//
// Positions are given against named reference points (the objective, own
// units, sighted enemies) as a bearing and a distance, never as raw
// coordinates: a model is poor at those and good at "800 m west of B2".
//
// Everything here is pure: the prompt in, the reply parsed and checked.
// The call itself is data/commanderClient.ts's. If Claude cannot be reached
// or its reply cannot be read, the heuristic's orders stand in, so the game
// can always go on.

import { distanceM, type LatLng } from "../../lib/board";
import { projectForSide } from "../../lib/fogOfWar";
import { inCover } from "../../lib/proceduralTerrain";
import type { ForceElement, Side } from "../../lib/state";
import { activityOf, setOrder } from "./engine";
import { hqPicture } from "./knowledge";
import { compass, offsetBy } from "./geometry";
import { PINNED_AT, SUPPRESSED_AT, clock } from "./timing";
import type {
  Boundary,
  Mission,
  MoveMode,
  OnContact,
  Phase,
  Roe,
  RtConfig,
  RtOrder,
  RtState,
  UnitOrders,
  Urgency,
} from "./types";

export interface OrdersResult {
  side: Side;
  /** New orders, by unit. Units left out keep what they have. */
  orders: Record<string, UnitOrders>;
  /** The side's plan in one line. */
  plan: string;
  /** What was wrong with the reply, and what was done about it. */
  warnings: string[];
  by: UnitOrders["by"];
}

const ROES: Roe[] = ["never", "ifFiredUpon", "withinShortRange", "always"];
const ON_CONTACT: OnContact[] = ["engage", "observe", "avoid", "bypass"];
const MODES: MoveMode[] = ["march", "tactical", "assault", "bound"];
const STATIONARY = ["hold", "overwatch", "observe"] as const;

/** The objective is "reached" within this. */
const ON_OBJECTIVE_M = 300;

// ── The heuristic's orders ─────────────────────────────────────────────────

/** Everyone advances on the objective and then watches over it; a unit already on it watches over it. */
export function heuristicOrders(state: RtState, side: Side): OrdersResult {
  const objective = state.game.objectives?.[side];
  const orders: Record<string, UnitOrders> = {};
  for (const fe of Object.values(state.game.forceElements)) {
    if (fe.side !== side || fe.combatStrength <= 0) continue;
    const far = objective != null && distanceM(fe.position, objective) > ON_OBJECTIVE_M;
    const phases: Phase[] = far
      ? [
          { label: "advance on the objective", order: { kind: "move", to: objective!, mode: "tactical" } },
          { label: "hold the objective on overwatch", order: { kind: "overwatch" } },
        ]
      : [{ label: "hold the objective on overwatch", order: { kind: "overwatch" } }];
    orders[fe.id] = {
      task: far ? "advance on the objective, then hold it" : "hold the objective",
      phases,
      phase: 0,
      intent: "take and hold the objective",
      urgency: "whenAble",
      roe: "withinShortRange",
      onContact: "engage",
      boundaries: [],
      by: "heuristic",
      issuedAt: state.time,
    };
  }
  return { side, orders, plan: "advance on the objective and take it", warnings: [], by: "heuristic" };
}

/** What a unit is FOR, from its orders: where the task ends, and whether that is the objective. */
export function missionFrom(orders: UnitOrders, fe: ForceElement, objective?: LatLng): Mission {
  const moves = orders.phases
    .map((phase) => phase.order)
    .filter((order): order is Extract<RtOrder, { kind: "move" | "withdraw" }> => order.kind === "move" || order.kind === "withdraw");
  const at = moves.length ? moves[moves.length - 1].to : fe.position;
  const onObjective = objective != null && distanceM(at, objective) <= ON_OBJECTIVE_M;
  const last = orders.phases[orders.phases.length - 1]?.order;
  if (onObjective && moves.length) return { task: "take", at, purpose: orders.intent || orders.task };
  if (last?.kind === "overwatch" && !moves.length) return { task: "support", at, purpose: orders.intent || orders.task };
  return { task: "hold", at, purpose: orders.intent || orders.task };
}

/**
 * Give units their orders at once: before the clock starts, when nobody is
 * in contact. While it runs, orders go through the runner (D0).
 */
export function applyOrders(state: RtState, result: OrdersResult, config: RtConfig): RtState {
  let next = state;
  const objective = state.game.objectives?.[result.side];
  for (const [id, orders] of Object.entries(result.orders)) {
    const fe = next.game.forceElements[id];
    if (!fe || fe.combatStrength <= 0 || !orders.phases.length) continue;
    next = setOrder(next, id, { ...orders.phases[0].order, phase: 0 }, config, {
      orders: { ...orders, phase: 0 },
      roe: orders.roe,
      mission: missionFrom(orders, fe, objective),
    });
  }
  return { ...next, plan: { ...next.plan, [result.side]: result.plan } };
}

/** The heuristic's orders, given at once. */
export function heuristicInitialOrders(state: RtState, side: Side, config: RtConfig): RtState {
  return applyOrders(state, heuristicOrders(state, side), config);
}

// ── The prompt ─────────────────────────────────────────────────────────────

/** Named points Claude places things against, as this side knows them. */
export function referencePoints(state: RtState, side: Side): Record<string, LatLng> {
  const refs: Record<string, LatLng> = {};
  const objective = state.game.objectives?.[side];
  if (objective) refs.OBJECTIVE = objective;
  const view = projectForSide(state.game, side);
  for (const fe of view.own) if (fe.combatStrength > 0) refs[fe.id] = fe.position;
  // Enemies where HQ believes they are: where they were seen, not where they are.
  for (const contact of hqPicture(state, side)) refs[contact.enemyId] = contact.at;
  for (const [id, seen] of Object.entries(state.lastKnown?.[side] ?? {})) if (!refs[id]) refs[id] = seen.at;
  return refs;
}

function relative(at: LatLng, refs: Record<string, LatLng>, prefer = "OBJECTIVE"): string {
  const base = refs[prefer] ? prefer : Object.keys(refs)[0];
  if (!base) return "";
  const d = distanceM(refs[base], at);
  return d < 50 ? `at ${base}` : `${Math.round(d / 50) * 50} m ${compass(refs[base], at)} of ${base}`;
}

/**
 * The prompt for one side's orders: what it knows, what has happened, what
 * each unit is doing, the player's guidance, and the reply format.
 */
export function ordersPrompt(
  state: RtState,
  side: Side,
  config: RtConfig,
  options: { guidance?: string; recent?: readonly { time: number; text: string }[] } = {},
): string {
  const refs = referencePoints(state, side);
  const view = projectForSide(state.game, side);
  const units = view.own
    .filter((fe) => fe.combatStrength > 0)
    .map((fe) => {
      const unit = state.units[fe.id];
      const orders = unit?.orders;
      return {
        id: fe.id,
        unit: fe.label,
        vehicles: unit ? `${unit.vehicles.fit} of ${unit.vehicles.total}` : String(fe.combatStrength),
        cohesion: unit?.cohesion ?? "steady",
        ...(unit && unit.suppression >= SUPPRESSED_AT ? { suppressed: unit.suppression >= PINNED_AT ? "pinned" : "suppressed" } : {}),
        where: relative(fe.position, refs),
        ground: `${config.terrain.classify(fe.position)}${inCover(config.terrain, fe.position) ? " (cover)" : ""}`,
        doing: unit ? activityOf(unit) : "holding",
        ...(orders
          ? {
              orders: `${orders.task} — on step ${orders.phase + 1} of ${orders.phases.length} (${orders.phases[orders.phase]?.label ?? "done"})`,
            }
          : {}),
        ...(unit && unit.vehicles.fit < unit.vehicles.total ? { lost: unit.vehicles.total - unit.vehicles.fit } : {}),
      };
    });
  // What has reached HQ, with its age and how far off it may be now: the
  // commander plans on the picture it has, as a real one does.
  const picture = hqPicture(state, side);
  const enemies = picture.map((contact) => ({
    id: contact.enemyId,
    unit: contact.level === "full" ? (contact.label ?? "identified") : "unidentified",
    where: relative(contact.at, refs),
    seen: state.time - contact.seenAt < 5 ? "in sight now" : `${clock(state.time - contact.seenAt)} ago, by ${contact.from}`,
    ...(contact.errorM >= 100 ? { mayHaveMoved: `up to ${Math.round(contact.errorM / 50) * 50} m since` } : {}),
    ...(contact.level === "full" && state.units[contact.enemyId]
      ? { vehicles: `${state.units[contact.enemyId].vehicles.fit} of ${state.units[contact.enemyId].vehicles.total} still fighting` }
      : {}),
  }));
  const inSight = new Set(picture.map((contact) => contact.enemyId));
  const lost = Object.entries(state.lastKnown?.[side] ?? {})
    .filter(([id]) => !inSight.has(id))
    .map(([id, seen]) => ({ id, unit: seen.label ?? "unidentified", lastSeen: `${relative(seen.at, refs)}, ${clock(state.time - seen.time)} ago` }));

  const situation = {
    clock: clock(state.time),
    you: side,
    ...(refs.OBJECTIVE ? { objective: "OBJECTIVE" } : {}),
    ...(state.plan[side] ? { currentPlan: state.plan[side] } : {}),
    yourUnits: units,
    enemiesKnown: enemies,
    ...(lost.length ? { lostContacts: lost } : {}),
    ...(options.recent?.length ? { recentEvents: options.recent.slice(-12).map((r) => `${clock(r.time)} ${r.text}`) } : {}),
    referencePoints: Object.keys(refs),
  };

  return [
    `You command the ${side} side in a real-time armoured battle, at troop and platoon level. The clock is`,
    "stopped. Write mission orders for your units; nothing moves until they are reviewed and the clock resumes.",
    "Each unit's leader carries your orders out and makes the local calls (when to fire, take cover, wait for",
    "a better shot) within them, weighing every one against your intent.",
    "",
    "What your side knows (enemies you have not sighted are not listed; that is not evidence they are not there):",
    JSON.stringify(situation, null, 1),
    "",
    options.guidance?.trim() ? `The player's guidance, which comes first: ${options.guidance.trim()}\n` : "",
    "Reply with JSON only, in this shape:",
    JSON.stringify(
      {
        plan: "the side's plan in one line",
        orders: [
          {
            unit: "<unit id>",
            task: "advance to the ridge west of the objective, then overwatch it",
            intent: "one line: why, so the leader can weigh a fight against it",
            urgency: "whenAble",
            roe: "withinShortRange",
            onContact: "engage",
            supports: "<optional: the id of the unit it supports>",
            phases: [
              { label: "advance to the ridge", do: "move", to: { ref: "OBJECTIVE", bearingDeg: 270, distanceM: 800 }, mode: "tactical" },
              { label: "overwatch the objective", do: "overwatch" },
            ],
            constraints: [{ stay: "south", of: { ref: "OBJECTIVE" }, label: "the objective line" }],
          },
        ],
      },
      null,
      1,
    ),
    "",
    "Rules:",
    "- unit: one of yourUnits' ids. Units you leave out keep their current orders.",
    '- phases, in order. do: "move" | "withdraw" | "hold" | "overwatch" | "observe". A move or withdraw needs',
    '  "to"; hold, overwatch and observe are where a task ends, so put them last. mode (moves only):',
    '  "march" (fastest, does not fire), "tactical" (fires within its ROE; the default), "assault"',
    '  (fires at anything, closes to point-blank: how ground is taken), "bound" (slow, pairs cover each other).',
    '- to: a reference point name, or {"ref": name, "bearingDeg": 0-359, "distanceM": metres} from it.',
    `  Reference points: ${Object.keys(refs).join(", ")}.`,
    '- urgency: "whenAble" (the default: a unit in a fight may finish it first, for at most two minutes) or',
    '  "now" (it breaks off at once; use it only when the timing is the point).',
    '- roe: "never" (weapons hold), "ifFiredUpon", "withinShortRange", "always".',
    '- onContact: "engage", "observe" (report, do not fire), "avoid" (pull back), "bypass" (carry on).',
    '- supports (optional): another of your units this one supports. Units ask each other for cover and fire',
    "  over the radio; that unit's requests come to this one first, and helping it is part of this one's orders.",
    '- constraints: fixed lines on the ground not to cross, {"stay": "north"|"south"|"east"|"west", "of": <point>,',
    '  "label": "a few words, e.g. the river line"}. The line runs east-west (for north/south) or north-south',
    '  (for east/west) through that point, and it does NOT move: place it from OBJECTIVE, e.g. {"ref":',
    '  "OBJECTIVE", "bearingDeg": 90, "distanceM": 1500}, never from a unit. Spacing and formation ("stay behind',
    '  B2") are not constraints: put them in the intent. The game enforces lines; leave the list empty if there are none.',
  ].join("\n");
}

// ── The reply ──────────────────────────────────────────────────────────────

function jsonIn(text: string): unknown {
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start < 0 || end <= start) throw new Error("no JSON object in the reply");
  return JSON.parse(text.slice(start, end + 1));
}

/** A point from a reference name, or a name with a bearing and a distance. */
function pointOf(spec: unknown, refs: Record<string, LatLng>): LatLng | null {
  if (typeof spec === "string") return refs[spec] ?? null;
  if (!spec || typeof spec !== "object") return null;
  const { ref, bearingDeg, distanceM: metres } = spec as { ref?: unknown; bearingDeg?: unknown; distanceM?: unknown };
  const base = typeof ref === "string" ? refs[ref] : undefined;
  if (!base) return null;
  const b = Number(bearingDeg ?? 0);
  const d = Number(metres ?? 0);
  if (!Number.isFinite(b) || !Number.isFinite(d) || d < 0 || d > 20_000) return null;
  return d === 0 ? base : offsetBy(base, ((b % 360) + 360) % 360, d);
}

/**
 * Read Claude's orders, checking every field. What cannot be used is dropped
 * with a warning, and a unit whose orders are unusable keeps what it has.
 */
export function parseOrders(text: string, state: RtState, side: Side): OrdersResult {
  const refs = referencePoints(state, side);
  const warnings: string[] = [];
  const reply = jsonIn(text) as { plan?: unknown; orders?: unknown };
  const list = Array.isArray(reply.orders) ? reply.orders : [];
  if (!Array.isArray(reply.orders)) warnings.push("the reply had no orders list");
  const orders: Record<string, UnitOrders> = {};

  for (const raw of list as Record<string, unknown>[]) {
    const id = typeof raw?.unit === "string" ? raw.unit : "";
    const fe = state.game.forceElements[id];
    if (!fe || fe.side !== side || fe.combatStrength <= 0) {
      warnings.push(`ignored orders for "${id}": not one of your units`);
      continue;
    }
    const phases: Phase[] = [];
    for (const [index, p] of (Array.isArray(raw.phases) ? raw.phases : []).entries()) {
      const phase = p as { label?: unknown; do?: unknown; to?: unknown; mode?: unknown };
      const label = typeof phase.label === "string" && phase.label.trim() ? phase.label.trim() : `step ${index + 1}`;
      const kind = phase.do;
      if (kind === "move" || kind === "withdraw") {
        const to = pointOf(phase.to, refs);
        if (!to) {
          warnings.push(`${id}: dropped "${label}": its destination could not be read`);
          continue;
        }
        const mode = MODES.includes(phase.mode as MoveMode) ? (phase.mode as MoveMode) : "tactical";
        phases.push({ label, order: kind === "move" ? { kind, to, mode } : { kind, to } });
      } else if (STATIONARY.includes(kind as (typeof STATIONARY)[number])) {
        phases.push({ label, order: { kind: kind as (typeof STATIONARY)[number] } });
        if (index < (raw.phases as unknown[]).length - 1) warnings.push(`${id}: phases after "${label}" dropped: it does not end`);
        break;
      } else {
        warnings.push(`${id}: dropped "${label}": unknown step "${String(kind)}"`);
      }
    }
    if (!phases.length) {
      warnings.push(`${id}: no usable phases, so it keeps its current orders`);
      continue;
    }
    const boundaries: Boundary[] = [];
    for (const c of Array.isArray(raw.constraints) ? raw.constraints : []) {
      const line = c as { stay?: unknown; of?: unknown; label?: unknown };
      // A line placed on a unit is really "stay behind B2", which moves with B2;
      // frozen where B2 happens to be now, it strands the unit. Drop it.
      const ref = typeof line.of === "string" ? line.of : (line.of as { ref?: unknown } | null)?.ref;
      if (typeof ref === "string" && state.game.forceElements[ref]) {
        warnings.push(`${id}: a line placed on ${ref} was dropped: lines are fixed on the ground and do not move with units`);
        continue;
      }
      const at = pointOf(line.of, refs);
      if (!at || !["north", "south", "east", "west"].includes(line.stay as string)) {
        warnings.push(`${id}: a constraint could not be read and was dropped`);
        continue;
      }
      const keep = line.stay as Boundary["keep"];
      const label = typeof line.label === "string" && line.label.trim() ? line.label.trim() : `the line through ${String((line.of as { ref?: string })?.ref ?? line.of)}`;
      // A line the unit is already on the wrong side of would freeze it; say so and drop it.
      const wrong =
        (keep === "north" && fe.position.lat < at.lat) ||
        (keep === "south" && fe.position.lat > at.lat) ||
        (keep === "east" && fe.position.lng < at.lng) ||
        (keep === "west" && fe.position.lng > at.lng);
      if (wrong) {
        warnings.push(`${id}: "stay ${keep} of ${label}" dropped: it is already on the other side`);
        continue;
      }
      boundaries.push({ keep, at, label });
    }
    const urgency: Urgency = raw.urgency === "now" ? "now" : "whenAble";
    const roe = ROES.includes(raw.roe as Roe) ? (raw.roe as Roe) : "withinShortRange";
    const onContact = ON_CONTACT.includes(raw.onContact as OnContact) ? (raw.onContact as OnContact) : "engage";
    const supported = typeof raw.supports === "string" ? state.game.forceElements[raw.supports] : undefined;
    if (typeof raw.supports === "string" && raw.supports && !raw.supports.startsWith("<") && (!supported || supported.side !== side || supported.id === id)) {
      warnings.push(`${id}: "supports ${raw.supports}" dropped: not another of your units`);
    }
    orders[id] = {
      task: typeof raw.task === "string" && raw.task.trim() ? raw.task.trim() : phases.map((p) => p.label).join(", then "),
      phases,
      phase: 0,
      intent: typeof raw.intent === "string" ? raw.intent.trim() : "",
      urgency,
      roe,
      onContact,
      boundaries,
      ...(supported && supported.side === side && supported.id !== id ? { supports: supported.id } : {}),
      by: "claude",
      issuedAt: state.time,
    };
  }
  return {
    side,
    orders,
    plan: typeof reply.plan === "string" && reply.plan.trim() ? reply.plan.trim() : "orders from the commander",
    warnings,
    by: "claude",
  };
}

/**
 * Ask the commander for a side's orders. Never throws: if the call fails or
 * the reply is unreadable, the heuristic's orders stand in, and the warning
 * says why.
 */
export async function commanderOrders(
  call: (prompt: string) => Promise<string>,
  state: RtState,
  side: Side,
  config: RtConfig,
  options: { guidance?: string; recent?: readonly { time: number; text: string }[]; log?: boolean } = {},
): Promise<OrdersResult> {
  const prompt = ordersPrompt(state, side, config, options);
  try {
    const reply = await call(prompt);
    const result = parseOrders(reply, state, side);
    if (options.log !== false && typeof console !== "undefined") {
      console.groupCollapsed?.(`[Commander ${side}] orders at ${clock(state.time)}: ${result.plan}`);
      console.log?.("prompt", prompt);
      console.log?.("reply", reply);
      console.log?.("orders", result.orders);
      if (result.warnings.length) console.warn?.(result.warnings.join("\n"));
      console.groupEnd?.();
    }
    if (Object.keys(result.orders).length === 0) {
      const fallback = heuristicOrders(state, side);
      return { ...fallback, warnings: [...result.warnings, "no usable orders in the reply: the heuristic's orders stand in"] };
    }
    return result;
  } catch (err) {
    const fallback = heuristicOrders(state, side);
    return {
      ...fallback,
      warnings: [`the commander could not be reached or read (${err instanceof Error ? err.message : String(err)}): the heuristic's orders stand in`],
    };
  }
}
