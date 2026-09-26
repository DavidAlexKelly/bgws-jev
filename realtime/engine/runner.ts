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
// Broken units are not asked anything: they are withdrawing, and the engine
// is doing that for them.

import type { Side } from "../../lib/state";
import type { RtDecider, RtDecision, RtDecisionRequest } from "./deciders";
import { ruleDecider } from "./deciders";
import { setOrder, tick } from "./engine";
import { optionsFor } from "./options";
import type { RtConfig, RtEvent, RtShot, RtState } from "./types";

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
    };

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
}

const DEFAULT_LOG_LIMIT = 500;

export class RealtimeRunner {
  state: RtState;
  readonly log: RtLogEntry[] = [];
  /** How often the clock has had to wait for a decision, for the status line. */
  waits = 0;

  private readonly pending = new Map<string, { events: RtEvent[]; firstAt: number }>();
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

  /** Run the clock forward by `seconds` of simulated time, or until the game ends. */
  async advance(seconds: number): Promise<void> {
    const until = this.state.time + seconds;
    while (!this.state.over && this.state.time < until) {
      await this.applyDue();
      const result = tick(this.state, this.config);
      this.state = result.state;
      for (const shot of result.shots) {
        const side = this.state.game.forceElements[shot.firerId]?.side ?? "blue";
        this.push({ type: "shot", time: shot.time, side, shot });
      }
      for (const event of result.events) {
        const side = this.state.game.forceElements[event.unitId]?.side ?? "blue";
        this.push({ type: "event", time: event.time, side, event });
        const held = this.pending.get(event.unitId);
        if (held) held.events.push(event);
        else this.pending.set(event.unitId, { events: [event], firstAt: event.time });
      }
      this.dispatch();
    }
  }

  private push(entry: RtLogEntry): void {
    this.log.push(entry);
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
      if (!fe || fe.combatStrength <= 0 || fe.morale === "broken") {
        this.pending.delete(unitId);
        continue;
      }
      if (busy.has(unitId)) continue;
      const severe = held.events.some((event) => event.severe);
      const settled = time - held.firstAt >= timing.coalesceS;
      const rested = time - (this.lastAsked.get(unitId) ?? -Infinity) >= timing.cooldownS;
      if ((severe || settled) && (severe || rested)) ready[fe.side].push(unitId);
    }

    for (const side of ["blue", "red"] as const) {
      if (ready[side].length === 0) continue;
      const requests: RtDecisionRequest[] = ready[side].map((unitId) => ({
        unitId,
        events: this.pending.get(unitId)!.events,
        options: optionsFor(state, unitId, config),
      }));
      for (const unitId of ready[side]) {
        this.pending.delete(unitId);
        this.lastAsked.set(unitId, time);
      }

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
        if (!decision || !option || !fe || fe.combatStrength <= 0 || fe.morale === "broken") continue;
        if (option.id !== "keep") {
          this.state = setOrder(this.state, unitId, option.order, this.config, option.roe ? { roe: option.roe } : {});
        }
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
