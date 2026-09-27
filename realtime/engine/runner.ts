// ── bgws/realtime/engine/runner.ts ─────────────────────────────────────────
// The clock, and when decisions are asked for and take effect.
//
// `tick` (engine.ts) is pure and knows nothing about deciders. The runner
// owns everything between them:
//
//   WHEN A UNIT IS ASKED. Events are held per unit and asked about together:
//   after `coalesceS` (so a burst of fire is one question, not six), no more
//   often than every `cooldownS` (so a unit does not dither), and at once for
//   anything severe (hit, broken, a friend destroyed). All of one side's
//   questions at the same moment go to the decider as one batch.
//
//   WHEN AN ANSWER TAKES EFFECT. At event time + the unit's reaction time, in
//   SIMULATED seconds — never "whenever the network answered". If the answer
//   is not back when it is due, the clock waits for it. That is what makes a
//   game replayable: the same seed and the same answers give the same game,
//   however fast or slow the connection was.
//
//   WHAT IT IS ASKED. Only the decision points (decisions.ts): an event that
//   matches none of them is logged and asks nobody. One decision in flight
//   per unit; what happens meanwhile is folded into its next question.
//
//   NEW ORDERS. The player's commander writes orders only while the clock is
//   stopped. On resume, `issueOrders` hands them over: a unit out of contact
//   switches at once, with no call; one in a fight is asked how to comply
//   (D0), so a new order never turns its back mid-fight.
//
//   FLAGS. While the clock runs, the game raises flags for the player — a
//   unit out of orders, heavy losses, an objective taken — and never calls
//   the commander on its own. Whether to pause and re-plan is the player's call.
//
// Shaken and broken units are not asked anything, as in Combat Mission: the
// autopilot runs them (holding, falling back, rallying) until they rally,
// when they are asked again.

import { distanceM } from "../../lib/board";
import type { Side } from "../../lib/state";
import type { RtDecider, RtDecision, RtDecisionRequest } from "./deciders";
import { ruleDecider } from "./deciders";
import type { Delivery } from "./comms";
import { decisionPointOf, inContact, optionsAt, ruleFallback } from "./decisions";
import { knownEnemies, remember, sendMessage, setOrder, tick } from "./engine";
import type { RtConfig, RtEvent, RtShot, RtState, UnitOrders } from "./types";

/** Something the player should know about, while the clock runs. */
export interface RtFlag {
  time: number;
  side: Side;
  unitId?: string;
  kind: "outOfOrders" | "losses" | "objective";
  text: string;
}

/** Something for the feed, in sim time order. */
export type RtLogEntry =
  | { type: "event"; time: number; side: Side; event: RtEvent }
  | { type: "shot"; time: number; side: Side; shot: RtShot }
  | {
      type: "decision";
      time: number;
      side: Side;
      decision: RtDecision;
      summary: string;
      /** When it was asked for, and when it took effect. */
      askedAt: number;
    }
  | { type: "flag"; time: number; side: Side; flag: RtFlag }
  /** A radio message, as it was heard (comms.ts). */
  | { type: "message"; time: number; side: Side; delivery: Delivery };

interface InFlight {
  side: Side;
  askedAt: number;
  /** Unit id → sim time its answer takes effect. */
  dueAt: Map<string, number>;
  requests: Map<string, RtDecisionRequest>;
  promise: Promise<RtDecision[]>;
  result?: RtDecision[];
}

export interface RunnerOptions {
  /** Per side. Absent means the rules decide for that side. */
  deciders?: Partial<Record<Side, RtDecider>>;
  /** How many feed entries to keep. */
  logLimit?: number;
  /** Called with every feed entry as it is made — to stream the log elsewhere. Must not throw. */
  onEntry?: (entry: RtLogEntry, state: RtState) => void;
}

const DEFAULT_LOG_LIMIT = 500;

export class RealtimeRunner {
  state: RtState;
  readonly log: RtLogEntry[] = [];
  /** How often the clock has had to wait for a decision, for the status line. */
  waits = 0;
  /** Flags raised for the player, oldest first. */
  readonly flags: RtFlag[] = [];

  private readonly pending = new Map<string, { events: RtEvent[]; firstAt: number }>();
  /** New orders waiting on a unit's D0 decision. */
  private readonly pendingOrders = new Map<string, UnitOrders>();
  /** Flags already raised, so each is raised once. */
  private readonly raised = new Set<string>();
  private readonly lastAsked = new Map<string, number>();
  private readonly inflight: InFlight[] = [];

  constructor(
    initial: RtState,
    readonly config: RtConfig,
    private readonly options: RunnerOptions = {},
  ) {
    this.state = initial;
  }

  /** Is a decision outstanding right now? For the "Jev thinking" indicator. */
  get thinking(): boolean {
    return this.inflight.some((batch) => batch.result === undefined);
  }

  /** The last few things this side saw happen, as short lines, for Jev. */
  recentFor(side: Side): { time: number; text: string }[] {
    const game = this.state.game;
    const name = (id: string) => {
      const fe = game.forceElements[id];
      if (!fe || fe.side === side) return id;
      return game.sighting[side][id] && game.sighting[side][id] !== "none" ? id : "unseen enemy";
    };
    return this.log
      .filter((entry): entry is Extract<RtLogEntry, { type: "shot" }> => entry.type === "shot")
      .filter((entry) => {
        const firer = game.forceElements[entry.shot.firerId];
        const target = game.forceElements[entry.shot.targetId];
        return firer?.side === side || target?.side === side;
      })
      .slice(-8)
      .map((entry) => ({
        time: entry.time,
        text: `${name(entry.shot.firerId)} fired at ${name(entry.shot.targetId)}: ${entry.shot.result}`,
      }));
  }

  /**
   * Hand over new standing orders, written while the clock was stopped. Out
   * of contact, a unit takes them at once; in a fight, it is asked how to
   * comply (D0). Units not named keep what they have.
   */
  issueOrders(orders: Record<string, UnitOrders>, plan?: { side: Side; text: string }): void {
    for (const [unitId, next] of Object.entries(orders)) {
      const fe = this.state.game.forceElements[unitId];
      const unit = this.state.units[unitId];
      if (!fe || !unit || fe.combatStrength <= 0 || !next.phases.length) continue;
      const issued = { ...next, phase: 0, issuedAt: this.state.time };
      if (unit.cohesion === "steady" && inContact(this.state, unitId)) {
        this.pendingOrders.set(unitId, issued);
        const event: RtEvent = {
          time: this.state.time,
          unitId,
          kind: "newOrders",
          detail: `new orders (${issued.urgency === "now" ? "now" : "when able"}): ${issued.task}`,
          severe: true,
        };
        this.push({ type: "event", time: event.time, side: fe.side, event });
        const held = this.pending.get(unitId);
        if (held) held.events.push(event);
        else this.pending.set(unitId, { events: [event], firstAt: event.time });
        continue;
      }
      this.install(unitId, issued, unit.cohesion === "steady");
    }
    if (plan) this.state = { ...this.state, plan: { ...this.state.plan, [plan.side]: plan.text } };
    this.dispatch();
  }

  /** New standing orders on a unit; and, if it takes orders, its first phase. */
  private install(unitId: string, orders: UnitOrders, start: boolean): void {
    this.pendingOrders.delete(unitId);
    for (const flag of [...this.raised]) if (flag.startsWith(`out:${unitId}`)) this.raised.delete(flag);
    if (start) {
      this.state = setOrder(this.state, unitId, { ...orders.phases[0].order, phase: 0 }, this.config, {
        orders,
        roe: orders.roe,
      });
    } else {
      const unit = this.state.units[unitId];
      this.state = { ...this.state, units: { ...this.state.units, [unitId]: { ...unit, orders, roe: orders.roe } } };
    }
  }

  /** Units with new orders still waiting on their D0 decision. */
  get awaitingOrders(): string[] {
    return [...this.pendingOrders.keys()];
  }

  /** Run the clock forward by `seconds` of simulated time, or until the game ends. */
  async advance(seconds: number): Promise<void> {
    const until = this.state.time + seconds;
    while (!this.state.over && this.state.time < until) {
      await this.applyDue();
      const result = tick(this.state, this.config);
      this.state = result.state;
      for (const delivery of result.messages) {
        this.push({ type: "message", time: delivery.time, side: delivery.message.side, delivery });
      }
      for (const shot of result.shots) {
        const side = this.state.game.forceElements[shot.firerId]?.side ?? "blue";
        this.push({ type: "shot", time: shot.time, side, shot });
      }
      for (const event of result.events) {
        const side = this.state.game.forceElements[event.unitId]?.side ?? "blue";
        this.push({ type: "event", time: event.time, side, event });
        const stuck = this.state.units[event.unitId]?.orders?.blocked;
        if (event.kind === "blocked" && stuck && stuck.time === event.time) {
          this.flag(
            { time: event.time, side, unitId: event.unitId, kind: "outOfOrders", text: `${event.unitId} cannot carry out its orders (${stuck.why}) and needs new ones` },
            `stuck:${event.unitId}:${stuck.phase}:${this.state.units[event.unitId].orders?.issuedAt}`,
          );
        }
        if (event.kind === "outOfOrders") {
          this.flag({ time: event.time, side, unitId: event.unitId, kind: "outOfOrders", text: `${event.unitId} has carried out its orders and needs new ones` }, `out:${event.unitId}`);
        }
        if (event.info) continue;
        const held = this.pending.get(event.unitId);
        if (held) held.events.push(event);
        else this.pending.set(event.unitId, { events: [event], firstAt: event.time });
      }
      this.checkFlags();
      this.dispatch();
    }
  }

  private flag(flag: RtFlag, key: string): void {
    if (this.raised.has(key)) return;
    this.raised.add(key);
    this.flags.push(flag);
    this.push({ type: "flag", time: flag.time, side: flag.side, flag });
  }

  /** Heavy losses and objectives taken, once each. */
  private checkFlags(): void {
    const { state } = this;
    for (const side of ["blue", "red"] as const) {
      const start = Math.max(1, state.startStrength?.[side] ?? 0);
      const own = Object.values(state.game.forceElements).filter((fe) => fe.side === side);
      const left = own.reduce((sum, fe) => sum + Math.max(0, fe.combatStrength), 0);
      const lost = 1 - left / start;
      for (const at of [0.25, 0.5]) {
        if (lost >= at) {
          this.flag({ time: state.time, side, kind: "losses", text: `${side} has lost ${Math.round(at * 100)}% of its strength` }, `loss:${side}:${at}`);
        }
      }
      const objective = state.game.objectives?.[side];
      if (!objective) continue;
      const holder = own.find(
        (fe) =>
          fe.combatStrength > 0 &&
          state.units[fe.id]?.cohesion === "steady" &&
          distanceM(fe.position, objective) <= 300 &&
          knownEnemies(state, fe.id).every((enemy) => distanceM(enemy.position, objective) > 500),
      );
      if (holder) {
        this.flag({ time: state.time, side, unitId: holder.id, kind: "objective", text: `${side} has taken its objective (${holder.id} is on it)` }, `obj:${side}`);
      }
    }
  }

  private push(entry: RtLogEntry): void {
    this.log.push(entry);
    try {
      this.options.onEntry?.(entry, this.state);
    } catch {
      // A listener's failure is never the game's.
    }
    const limit = this.options.logLimit ?? DEFAULT_LOG_LIMIT;
    if (this.log.length > limit) this.log.splice(0, this.log.length - limit);
  }

  /** Ask about every unit whose held events are ready, one batch per side. */
  private dispatch(): void {
    const { state, config } = this;
    const time = state.time;
    const timing = config.timing;
    const busy = new Set(this.inflight.flatMap((batch) => [...batch.dueAt.keys()]));
    const ready: Record<Side, string[]> = { blue: [], red: [] };

    for (const [unitId, held] of this.pending) {
      const fe = state.game.forceElements[unitId];
      if (!fe || fe.combatStrength <= 0 || state.units[unitId]?.cohesion !== "steady") {
        this.pending.delete(unitId);
        // Not taking orders: its new orders wait for it to rally (D11).
        const orders = this.pendingOrders.get(unitId);
        if (orders && fe && fe.combatStrength > 0) this.install(unitId, orders, false);
        continue;
      }
      if (busy.has(unitId)) continue;
      // Only the decision points ask anything.
      if (!decisionPointOf(held.events)) {
        this.pending.delete(unitId);
        continue;
      }
      const severe = held.events.some((event) => event.severe);
      const settled = time - held.firstAt >= timing.coalesceS;
      const rested = time - (this.lastAsked.get(unitId) ?? -Infinity) >= timing.cooldownS;
      if ((severe || settled) && (severe || rested)) ready[fe.side].push(unitId);
    }

    for (const side of ["blue", "red"] as const) {
      if (ready[side].length === 0) continue;
      const requests: RtDecisionRequest[] = [];
      for (const unitId of ready[side]) {
        const events = this.pending.get(unitId)!.events;
        const { point, event } = decisionPointOf(events)!;
        const orders = this.pendingOrders.get(unitId);
        const options = optionsAt(state, unitId, point, event, config, { orders });
        this.pending.delete(unitId);
        this.lastAsked.set(unitId, time);
        // A new order with nothing to choose (the fight ended meanwhile): it simply takes over.
        if (point === "D0" && orders && !options.some((option) => option.orders)) {
          this.install(unitId, orders, true);
          continue;
        }
        requests.push({ unitId, point, event, events, options, fallback: ruleFallback(state, unitId, point, options) });
      }
      if (requests.length === 0) continue;

      const decider = this.options.deciders?.[side] ?? ruleDecider;
      const batch: InFlight = {
        side,
        askedAt: time,
        dueAt: new Map(
          requests.map((request) => [
            request.unitId,
            time + timing.reactionS(state.game.forceElements[request.unitId].troopQuality),
          ]),
        ),
        requests: new Map(requests.map((request) => [request.unitId, request])),
        // A decider must not reject; if one does anyway, the rules answer.
        promise: decider.decide(state, side, requests, config).catch(() =>
          ruleDecider.decide(state, side, requests, config),
        ),
      };
      batch.promise.then((result) => {
        batch.result = result;
      });
      this.inflight.push(batch);
    }
  }

  /** Apply every answer that is due now, waiting for it if it is not back yet. */
  private async applyDue(): Promise<void> {
    const time = this.state.time;
    for (const batch of [...this.inflight]) {
      const due = [...batch.dueAt.entries()].filter(([, at]) => at <= time);
      if (due.length === 0) continue;
      if (!batch.result) {
        this.waits += 1;
        batch.result = await batch.promise;
      }
      for (const [unitId] of due) {
        batch.dueAt.delete(unitId);
        const decision = batch.result.find((one) => one.unitId === unitId);
        const request = batch.requests.get(unitId);
        const option = request?.options.find((one) => one.id === decision?.optionId);
        const fe = this.state.game.forceElements[unitId];
        const steady = this.state.units[unitId]?.cohesion === "steady";
        if (!decision || !option || !fe || fe.combatStrength <= 0 || !steady) {
          const orders = this.pendingOrders.get(unitId);
          if (orders && fe && fe.combatStrength > 0) this.install(unitId, orders, false);
          continue;
        }
        if (option.orders) {
          this.pendingOrders.delete(unitId);
          this.state = setOrder(this.state, unitId, option.order, this.config, { orders: option.orders, roe: option.orders.roe });
        } else if (option.id !== "keep") {
          // A fight it was allowed to finish first (D0) stays time-boxed,
          // whoever it ends up firing at, so it cannot drift.
          const current = this.state.units[unitId].order;
          const order =
            current.kind === "engage" && current.until != null && current.then && option.order.kind === "engage" && !option.order.then
              ? { ...option.order, until: current.until, then: current.then }
              : option.order;
          this.state = setOrder(this.state, unitId, order, this.config, option.roe ? { roe: option.roe } : {});
        }
        // A request or reply it sends with this choice goes on the net; a
        // request it has answered is off its list.
        if (option.message) {
          const m = option.message;
          this.state = sendMessage(
            this.state,
            {
              kind: m.kind,
              from: unitId,
              to: m.to,
              text: m.text,
              ...(m.request ? { request: { ...m.request, id: this.state.comms.nextId, text: m.text } } : {}),
              ...(m.reply ? { reply: m.reply } : {}),
            },
            this.config,
          );
        }
        if (option.answers != null) {
          const u = this.state.units[unitId];
          this.state = {
            ...this.state,
            units: { ...this.state.units, [unitId]: { ...u, requests: u.requests.filter((r) => r.id !== option.answers) } },
          };
        }
        // A friend's covering fire, ordered with it.
        for (const also of option.also ?? []) {
          const friend = this.state.game.forceElements[also.unitId];
          if (friend && friend.combatStrength > 0 && this.state.units[also.unitId]?.cohesion === "steady") {
            this.state = setOrder(this.state, also.unitId, also.order, this.config);
          }
        }
        // Remember it, "carry on" included, so the next question about this
        // unit can see what it was told and what came of it.
        const unit = this.state.units[unitId];
        const memory = remember(unit, {
          time,
          chose: option.id === "keep" ? `carry on (${option.summary.replace(/^carry on /, "")})` : option.summary,
          by: decision.trace.chosenBy === "jev" ? "jev" : "rules",
          because: `${request!.point}: ${[...new Set(request!.events.map((event) => event.kind))].join(", ")}`,
          strength: fe.combatStrength,
          dealt: unit.dealt,
        });
        this.state = { ...this.state, units: { ...this.state.units, [unitId]: { ...unit, history: memory } } };
        this.push({
          type: "decision",
          time,
          side: batch.side,
          decision,
          summary: option.summary,
          askedAt: batch.askedAt,
        });
      }
      if (batch.dueAt.size === 0) this.inflight.splice(this.inflight.indexOf(batch), 1);
    }
  }
}
