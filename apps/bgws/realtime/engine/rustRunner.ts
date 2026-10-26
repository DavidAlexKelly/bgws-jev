// ── rustRunner.ts ────────────────────────────────────────────────────────
// The React screens on the Rust engine (bgws-rs, RUST_ENGINE.md §16): a
// RunnerHandle whose game runs in bgws-server, reached over its WebSocket.
//
//   SETUP    The screen's scenario (the same RealtimeScenario it saves) is
//            loaded into the server, which converts it to its own format
//            with the same unit ids. The TypeScript state built from the
//            same setup is kept as the shape the screens read.
//   CLOCK    The screen still runs the clock: advance(n) asks the server for
//            n simulated seconds and resolves when they have passed there.
//   STATE    Each frame from the server writes what changed onto that state:
//            positions, facing, strength, morale, suppression, the time and
//            the outcome. Units only the Rust engine has (drones in flight)
//            are not drawn by these screens; the server's own viewer shows them.
//   FEED     Rust events become the log entries the screens draw: volleys are
//            shots (fire lines, wrecks), applied answers are decisions, and
//            everything else is a world item with its text.
//   ORDERS   Orders written on the screen, an HQ's plan and seat changes go
//            to the server in its orders schema, over the side's nets.
//
// Deciders are the server's (`bgws-server --deciders`): the screen's Jev and
// Claude deciders are not used, and a question the server sends to a person
// is answered with the rules' choice.

import type { LatLng } from "../../lib/board";
import type { Morale, Side } from "../../lib/state";
import type { PlanAnswer } from "../world/types";
import { entryContext } from "./feed";
import type { HandleOptions, RunnerHandle } from "./runnerHandle";
import { DEFAULT_LOG_LIMIT, recentShots, type RtFlag, type RtLogEntry } from "./runner";
import type { RtConfig, RtOrder, RtState, UnitOrders } from "./types";
import type { RealtimeScenario } from "../scenario";

/** One entity in a server frame (FrameEntity in bgws-rs/web/types). */
export interface ServerEntity {
  id: number;
  name: string;
  side: Side | "neutral";
  domain: string;
  lat: number;
  lon: number;
  alt: number;
  heading_deg: number;
  strength: number;
  fit: number;
  total: number;
  status: string;
  activity: string;
  suppression: number;
}

/** One event as the server sends it. */
export interface ServerEvent {
  t: number;
  kind: string;
  text: string;
  side: Side | "neutral" | null;
  entity: string | null;
  target: string | null;
  data: Record<string, unknown> | null;
}

/** The orders schema's unit order (bgws-sim `hq::UnitOrder`). */
export interface ServerUnitOrder {
  unit: string;
  task: string;
  intent: string;
  urgency?: string;
  roe?: string;
  onContact?: string;
  phases: { label: string; do: string; to?: { lat: number; lon: number }; mode?: string }[];
}

const MORALE: Record<string, Morale> = { ok: "good", suppressed: "suppressed1", pinned: "suppressed2", broken: "broken", destroyed: "broken" };
const STATUSES = ["ok", "suppressed", "pinned", "broken", "destroyed"];

/** A screen order as the orders schema's phases: chains (`then`) flattened. */
export function serverPhases(label: string, order: RtOrder | undefined): ServerUnitOrder["phases"] {
  const out: ServerUnitOrder["phases"] = [];
  for (let o = order, i = 0; o && i < 8; i++) {
    const tag = i === 0 ? label : `${label} (${i + 1})`;
    switch (o.kind) {
      case "move":
        out.push({ label: tag, do: "move", to: { lat: o.to.lat, lon: o.to.lng }, mode: o.mode });
        o = o.then;
        continue;
      case "withdraw":
        out.push({ label: tag, do: "withdraw", to: { lat: o.to.lat, lon: o.to.lng } });
        o = o.then;
        continue;
      case "hold":
        out.push({ label: tag, do: "hold" });
        break;
      case "observe":
      case "search":
        out.push({ label: tag, do: "observe" });
        break;
      // The Rust engine fires at what it sees on overwatch; it takes no single-target engage or wait order.
      case "engage":
      case "wait":
      case "overwatch":
        out.push({ label: tag, do: "overwatch" });
        break;
    }
    break;
  }
  return out;
}

/** The screen's orders for some units, as the orders schema. */
export function serverOrders(orders: Record<string, UnitOrders>): ServerUnitOrder[] {
  return Object.entries(orders).map(([unit, o]) => ({
    unit,
    task: o.task,
    intent: o.intent,
    urgency: o.urgency,
    roe: o.roe,
    onContact: o.onContact,
    // From the phase being carried out on.
    phases: o.phases.slice(o.phase ?? 0).flatMap((p) => serverPhases(p.label, p.order)),
  }));
}

function sourceKind(source: unknown): string {
  return source && typeof source === "object" && "kind" in source ? String((source as { kind: unknown }).kind) : "rules";
}

/** A server event as the screens' log entries (none for what they have no place for). */
export function logEntries(e: ServerEvent, state: RtState): RtLogEntry[] {
  if (e.side !== "blue" && e.side !== "red") return [];
  const side = e.side;
  const data = e.data ?? {};
  const fe = (id: string | null) => (id ? state.game.forceElements[id] : undefined);
  if (e.kind.startsWith("fire.") && e.entity && e.target && fe(e.entity) && fe(e.target)) {
    const kills = Number(data.kills ?? 0), hits = Number(data.hits ?? 0);
    return [{
      type: "shot",
      time: e.t,
      side,
      shot: {
        time: e.t,
        firerId: e.entity,
        targetId: e.target,
        result: e.text,
        narrative: e.text,
        rangeM: data.rangeM != null ? Number(data.rangeM) : undefined,
        rounds: data.rounds != null ? Number(data.rounds) : undefined,
        hits,
        knockedOut: kills,
        pHit: data.ph != null ? Number(data.ph) : undefined,
        outcome: kills > 0 ? "knockedOut" : hits > 0 ? "penetrated" : "miss",
        firerAt: fe(e.entity)?.position,
        targetAt: fe(e.target)?.position,
      },
    }];
  }
  if (e.kind === "decide.apply" && e.entity && data.option != null) {
    const kind = sourceKind(data.source);
    const summary = String(data.summary ?? e.text);
    return [{
      type: "decision",
      time: e.t,
      side,
      summary,
      askedAt: e.t,
      decision: {
        unitId: e.entity,
        optionId: String(data.option),
        trace: {
          actorId: e.entity,
          question: String(data.title ?? data.trigger ?? "decision"),
          options: Array.isArray(data.options) ? (data.options as { id: string; summary: string }[]) : [],
          chosenId: String(data.option),
          chosenBy: kind === "jev" ? "jev" : kind === "claude" || kind === "local" ? "llm" : "heuristic",
          rationale: data.rationale != null ? String(data.rationale) : `decided by ${kind} in the Rust engine`,
          probabilities: (data.probabilities as Record<string, number> | null) ?? undefined,
          fallback: kind === "fallback" ? "error" : undefined,
          mark: summary,
        },
      },
    }];
  }
  const ids = [e.entity, e.target].filter((x): x is string => !!x);
  return [{
    type: "world",
    time: e.t,
    side,
    item: { time: e.t, side, domain: e.kind.split(".")[0], kind: e.kind, text: e.text, ids, at: fe(e.entity)?.position },
  }];
}

/** Write a frame's entities onto the screens' state. */
export function applyEntities(state: RtState, entities: readonly Pick<ServerEntity, "name" | "lat" | "lon" | "heading_deg" | "strength" | "status" | "suppression">[]): void {
  for (const e of entities) {
    const fe = state.game.forceElements[e.name];
    if (!fe) continue;
    fe.position = { lat: e.lat, lng: e.lon } as LatLng;
    fe.facing = e.heading_deg;
    fe.combatStrength = e.status === "destroyed" || e.strength <= 0 ? 0 : Math.max(1, Math.round(fe.combatStrengthStart * e.strength));
    fe.morale = MORALE[e.status] ?? fe.morale;
    const unit = state.units[e.name];
    if (unit) {
      unit.suppression = Math.round(Math.min(1, Math.max(0, e.suppression)) * 100);
      unit.cohesion = e.status === "broken" ? "broken" : unit.cohesion === "broken" ? "shaken" : unit.cohesion;
    }
  }
}

type Socket = Pick<WebSocket, "send" | "close" | "readyState"> & {
  binaryType: string;
  onopen: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: ((ev: unknown) => void) | null;
  onerror: ((ev: unknown) => void) | null;
};

export interface RustRunnerOptions extends Pick<HandleOptions, "logLimit" | "onEntry"> {
  /** For tests: how to open the socket. */
  connect?: (url: string) => Socket;
}

/** The game in bgws-server, behind the screens' RunnerHandle. */
export class RustRunner implements RunnerHandle {
  readonly where = "bgws-server" as const;
  readonly config: RtConfig;
  state: RtState;
  log: RtLogEntry[] = [];
  flags: RtFlag[] = [];
  waits = 0;
  thinking = false;
  awaitingOrders: string[] = [];
  /** Resolves when the server has the scenario; rejects with its problems. */
  readonly ready: Promise<void>;
  private ws: Socket;
  private stepWaiters: ((t: number) => void)[] = [];
  private readonly limit: number;
  private readonly onEntry?: HandleOptions["onEntry"];
  private closed = false;
  private failure: Error | null = null;

  constructor(url: string, initial: RtState, config: RtConfig, scenario: RealtimeScenario, options: RustRunnerOptions = {}) {
    this.state = structuredClone(initial);
    this.config = config;
    this.limit = options.logLimit ?? DEFAULT_LOG_LIMIT;
    this.onEntry = options.onEntry;
    this.ws = (options.connect ?? ((u) => new WebSocket(u) as unknown as Socket))(url);
    this.ws.binaryType = "arraybuffer";
    let loaded: () => void = () => {};
    let refused: (e: Error) => void = () => {};
    this.ready = new Promise<void>((resolve, reject) => {
      loaded = resolve;
      refused = reject;
    });
    this.ready.catch(() => {});
    this.ws.onopen = () => {
      this.send({ kind: "viewer", viewer: "umpire" });
      this.send({ kind: "load", scenario });
    };
    this.ws.onerror = () => this.fail(new Error(`cannot reach bgws-server at ${url}`), refused);
    this.ws.onclose = () => this.fail(new Error("bgws-server closed the connection"), refused);
    this.ws.onmessage = (ev) => {
      if (ev.data instanceof ArrayBuffer) return this.positions(ev.data);
      const msg = JSON.parse(String(ev.data));
      switch (msg.kind) {
        case "checked":
          if (msg.loaded) loaded();
          else if (msg.errors?.length) refused(new Error(`bgws-server refused the scenario: ${msg.errors.join("; ")}`));
          break;
        case "frame":
          if (!msg.scrub) this.frame(msg.frame);
          break;
        case "events":
          for (const e of msg.events as ServerEvent[]) this.event(e);
          break;
        case "questions":
          // Questions for a seat these screens hold. An HQ's: keep the orders
          // the screen gave (it plans with issueOrders/command). A unit's: the rules' choice.
          for (const q of msg.questions ?? []) this.send({ kind: "answer", question: Number(q.id), choice: q.options?.length ? (q.rules_choice ?? q.fallback) : "keep" });
          break;
        case "advanced":
          this.state.time = msg.t;
          for (const w of this.stepWaiters.splice(0)) w(msg.t);
          break;
      }
    };
  }

  private fail(e: Error, refused: (e: Error) => void): void {
    if (this.closed) return;
    this.failure = e;
    refused(e);
    for (const w of this.stepWaiters.splice(0)) w(this.state.time);
  }

  private send(m: unknown): void {
    if (this.ws.readyState === 1) this.ws.send(JSON.stringify(m));
  }

  private frame(f: { t: number; entities: ServerEntity[]; waiting: number; over: { winner: Side | null; reason: string } | null }): void {
    for (const e of f.entities) {
      this.ids.set(e.name, e.id);
      this.byId.set(e.id, e.name);
    }
    applyEntities(this.state, f.entities);
    this.state.time = f.t;
    this.thinking = f.waiting > 0;
    if (f.over) this.state.over = { winner: f.over.winner ?? null, reason: f.over.reason };
  }

  private positions(buf: ArrayBuffer): void {
    const v = new DataView(buf);
    if (v.getUint8(0) !== 1) return;
    const n = v.getUint32(9, true);
    const out: Parameters<typeof applyEntities>[1][number][] = [];
    for (let i = 0, o = 13; i < n; i++, o += 29) {
      const name = this.byId.get(v.getUint32(o, true));
      if (!name) continue;
      const fe = this.state.game.forceElements[name];
      const status = STATUSES[v.getUint8(o + 28)] ?? "ok";
      out.push({ name, lat: v.getFloat64(o + 4, true), lon: v.getFloat64(o + 12, true), heading_deg: v.getFloat32(o + 24, true), status, strength: fe ? fe.combatStrength / Math.max(1, fe.combatStrengthStart) : 1, suppression: (this.state.units[name]?.suppression ?? 0) / 100 });
    }
    applyEntities(this.state, out);
    this.state.time = v.getFloat64(1, true);
  }

  /** Server entity ids by name, from full frames (binary updates carry ids only). */
  private ids = new Map<string, number>();
  private byId = new Map<number, string>();

  private event(e: ServerEvent): void {
    for (const entry of logEntries(e, this.state)) {
      this.log.push(entry);
      if (this.log.length > this.limit) this.log.splice(0, this.log.length - this.limit);
      this.onEntry?.(entry, entryContext(entry, this.state));
    }
  }

  recentFor(side: Side) {
    return recentShots(this.log, this.state, side);
  }

  /** n simulated seconds in the server; resolves when they have passed there. */
  async advance(seconds: number): Promise<void> {
    await this.ready;
    if (this.failure) throw this.failure;
    if (this.state.over) return;
    const done = new Promise<number>((resolve) => this.stepWaiters.push(resolve));
    this.send({ kind: "advance", seconds });
    await done;
    if (this.failure) throw this.failure;
  }

  /** Sides whose HQ seat these screens hold. */
  private seated = new Set<Side>();

  /** The screen commands this side from now on: the Rust HQ stops planning over it. */
  private takeSeat(side: Side): void {
    if (this.seated.has(side)) return;
    this.seated.add(side);
    this.send({ kind: "seat", side, seat: "person" });
  }

  async issueOrders(orders: Record<string, UnitOrders>, plan?: { side: Side; text: string }): Promise<void> {
    await this.ready;
    const bySide = new Map<Side, ServerUnitOrder[]>();
    for (const o of serverOrders(orders)) {
      const side = this.state.game.forceElements[o.unit]?.side;
      if (side !== "blue" && side !== "red") continue;
      bySide.set(side, [...(bySide.get(side) ?? []), o]);
    }
    for (const [side, list] of bySide) {
      this.takeSeat(side);
      this.send({ kind: "command", side, plan: { plan: plan?.side === side ? plan.text : "orders", orders: list } });
    }
  }

  async command(hqId: string, answer: PlanAnswer): Promise<void> {
    await this.ready;
    const side = this.sideOfHq(hqId);
    if (!side) throw new Error(`no HQ ${hqId}`);
    this.takeSeat(side);
    this.send({ kind: "command", side, plan: { plan: answer.plan, orders: serverOrders(answer.orders) } });
  }

  async setSeat(hqId: string, seat: "human" | "claude" | "heuristic"): Promise<void> {
    await this.ready;
    const side = this.sideOfHq(hqId);
    if (!side) throw new Error(`no HQ ${hqId}`);
    // The Rust engine's planner seat uses Claude when it has a key, its heuristic otherwise.
    if (seat === "human") this.seated.add(side);
    else this.seated.delete(side);
    this.send({ kind: "seat", side, seat: seat === "human" ? "person" : "planner" });
  }

  private sideOfHq(hqId: string): Side | undefined {
    const hq = this.state.world?.hqs?.[hqId] as { side?: Side } | undefined;
    const side = hq?.side ?? this.state.game.forceElements[hqId]?.side;
    return side === "blue" || side === "red" ? side : undefined;
  }

  dispose(): void {
    this.closed = true;
    this.ws.close();
  }
}
