// ── runnerHandle.ts ──────────────────────────────────────────────────────
// Step 2 of docs/plan/WORKER_AND_SCHEDULER.md: one surface for the screen,
// two places the game can run.
//
//   InThreadRunner — today's RealtimeRunner, on the main thread. What the
//                    tests use, and the default.
//   WorkerRunner   — the same runner in a Web Worker (workerHost.ts), so a
//                    big fight never blocks a frame. The screen reads a
//                    replica of the worker's state, kept by patches
//                    (statePatch.ts); deciders (Jev, the rules if asked)
//                    stay here and answer the worker's questions from it.
//
// The same seeded game through either ends in the same state: the runner
// waits for answers in simulated time, so message timing changes nothing.

import type { RasterGridData } from "../../../../shared/routing/terrainRaster";
import type { Side } from "../../lib/state";
import type { RtConfigSpec } from "./configSpec";
import { ruleDecider, type RtDecider } from "./deciders";
import { entryContext, type EntryContext } from "./feed";
import { DEFAULT_LOG_LIMIT, RealtimeRunner, recentShots, type RtFlag, type RtLogEntry } from "./runner";
import { applyPatch } from "./statePatch";
import type { RtConfig, RtState, UnitOrders } from "./types";
import type { FromWorker, MessageEndpoint, RunnerSnapshot, ToWorker } from "./workerHost";
import { ruleWorldDecider, type WorldDecider } from "../world/deciders";
import { seatAt, type PlannerSeat } from "../world/command/seat";
import { heuristicPlan } from "../world/command/heuristicPlanner";
import type { PlanAnswer } from "../world/types";

export interface RunnerHandle {
  /** Where the game runs, for the status line. */
  readonly where: "main thread" | "worker";
  /** The game as it last stood. From a worker: as of its last reply. */
  readonly state: RtState;
  /** The live config on this thread: for the commander and the deciders. */
  readonly config: RtConfig;
  readonly log: readonly RtLogEntry[];
  readonly flags: readonly RtFlag[];
  /** How often the clock has waited for a decision. */
  readonly waits: number;
  /** Is a decision outstanding? */
  readonly thinking: boolean;
  /** Units with new orders still waiting on their D0 decision. */
  readonly awaitingOrders: readonly string[];
  recentFor(side: Side): { time: number; text: string }[];
  advance(seconds: number): Promise<void>;
  /** Resolves once the orders are with the runner (at once, on the main thread). */
  issueOrders(orders: Record<string, UnitOrders>, plan?: { side: Side; text: string }): Promise<void>;
  /** A commander's orders from an HQ, sent over its nets (W15). */
  command(hqId: string, answer: PlanAnswer): Promise<void>;
  /** Take an HQ's seat or hand it back (W15). */
  setSeat(hqId: string, seat: "human" | "claude" | "heuristic"): Promise<void>;
  /** Stop: a worker is terminated. */
  dispose(): void;
}

export interface HandleOptions {
  /** Per side; absent means the rules decide. */
  deciders?: Partial<Record<Side, RtDecider>>;
  /** Per side, for world questions (realtime/world); absent means the rules decide. */
  worldDeciders?: Partial<Record<Side, WorldDecider>>;
  /** Per side, the planner for Claude-commanded HQ seats; absent means the heuristic. */
  planners?: Partial<Record<Side, PlannerSeat>>;
  logLimit?: number;
  /** Every feed entry as it is made, with its context then (feed.ts). Must not throw. */
  onEntry?: (entry: RtLogEntry, context: EntryContext) => void;
}

/** The runner on this thread. */
export class InThreadRunner implements RunnerHandle {
  readonly where = "main thread" as const;
  private readonly runner: RealtimeRunner;

  constructor(initial: RtState, config: RtConfig, options: HandleOptions = {}) {
    const { onEntry } = options;
    this.runner = new RealtimeRunner(initial, config, {
      deciders: options.deciders,
      worldDeciders: options.worldDeciders,
      planners: options.planners,
      logLimit: options.logLimit,
      onEntry: onEntry ? (entry, state) => onEntry(entry, entryContext(entry, state)) : undefined,
    });
  }

  get state() {
    return this.runner.state;
  }
  get config() {
    return this.runner.config;
  }
  get log() {
    return this.runner.log;
  }
  get flags() {
    return this.runner.flags;
  }
  get waits() {
    return this.runner.waits;
  }
  get thinking() {
    return this.runner.thinking;
  }
  get awaitingOrders() {
    return this.runner.awaitingOrders;
  }
  recentFor(side: Side) {
    return this.runner.recentFor(side);
  }
  advance(seconds: number) {
    return this.runner.advance(seconds);
  }
  issueOrders(orders: Record<string, UnitOrders>, plan?: { side: Side; text: string }) {
    this.runner.issueOrders(orders, plan);
    return Promise.resolve();
  }
  command(hqId: string, answer: PlanAnswer) {
    this.runner.command(hqId, answer);
    return Promise.resolve();
  }
  setSeat(hqId: string, seat: "human" | "claude" | "heuristic") {
    this.runner.setSeat(hqId, seat);
    return Promise.resolve();
  }
  dispose() {}
}

export interface WorkerRunnerInit {
  /** What the worker builds its config from. */
  spec: RtConfigSpec;
  /** The terrain raster's grid when the ground uses one; its buffer is transferred, so pass a copy. */
  raster: RasterGridData | null;
  initial: RtState;
  /** The same config built on this thread, for the commander and deciders here. */
  config: RtConfig;
}

/** The runner in a worker, over any message endpoint. */
export class WorkerRunner implements RunnerHandle {
  readonly where = "worker" as const;
  readonly config: RtConfig;
  state: RtState;
  readonly log: RtLogEntry[] = [];
  readonly flags: RtFlag[] = [];
  waits = 0;
  awaitingOrders: readonly string[] = [];
  private remoteThinking = false;
  private asking = 0;
  private nextId = 0;
  private failed: Error | null = null;
  private readonly calls = new Map<number, { resolve: () => void; reject: (error: Error) => void }>();
  private readonly listener = (event: MessageEvent) => this.receive(event.data as FromWorker);

  constructor(
    private readonly endpoint: MessageEndpoint,
    init: WorkerRunnerInit,
    private readonly options: HandleOptions = {},
    private readonly stop: () => void = () => {},
  ) {
    this.config = init.config;
    this.state = init.initial;
    endpoint.addEventListener("message", this.listener);
    const message: ToWorker = {
      type: "init",
      spec: init.spec,
      raster: init.raster,
      state: init.initial,
      proxied: (Object.keys(options.deciders ?? {}) as Side[]).filter((side) => options.deciders?.[side] != null),
      proxiedWorld: (Object.keys(options.worldDeciders ?? {}) as Side[]).filter((side) => options.worldDeciders?.[side] != null),
      proxiedPlanners: (Object.keys(options.planners ?? {}) as Side[]).filter((side) => options.planners?.[side] != null),
      logLimit: options.logLimit,
    };
    endpoint.postMessage(message, init.raster ? [init.raster.data.buffer as ArrayBuffer] : []);
  }

  get thinking() {
    return this.remoteThinking || this.asking > 0;
  }

  recentFor(side: Side) {
    return recentShots(this.log, this.state, side);
  }

  advance(seconds: number): Promise<void> {
    return this.call((id) => ({ type: "advance", id, seconds }));
  }

  issueOrders(orders: Record<string, UnitOrders>, plan?: { side: Side; text: string }): Promise<void> {
    return this.call((id) => ({ type: "issueOrders", id, orders, ...(plan ? { plan } : {}) }));
  }

  command(hqId: string, answer: PlanAnswer): Promise<void> {
    return this.call((id) => ({ type: "command", id, hq: hqId, answer }));
  }

  setSeat(hqId: string, seat: "human" | "claude" | "heuristic"): Promise<void> {
    return this.call((id) => ({ type: "setSeat", id, hq: hqId, seat }));
  }

  /** The worker died or could not load: every call from now on fails with this. */
  fail(error: Error): void {
    this.failed = error;
    for (const call of this.calls.values()) call.reject(error);
    this.calls.clear();
  }

  dispose(): void {
    this.endpoint.removeEventListener("message", this.listener);
    this.fail(new Error("engine stopped"));
    this.stop();
  }

  private call(make: (id: number) => ToWorker): Promise<void> {
    if (this.failed) return Promise.reject(this.failed);
    const id = this.nextId++;
    return new Promise<void>((resolve, reject) => {
      this.calls.set(id, { resolve, reject });
      this.endpoint.postMessage(make(id));
    });
  }

  private receive(message: FromWorker): void {
    switch (message.type) {
      case "done": {
        this.apply(message.snapshot);
        this.calls.get(message.id)?.resolve();
        this.calls.delete(message.id);
        break;
      }
      case "error": {
        this.calls.get(message.id)?.reject(new Error(message.message));
        this.calls.delete(message.id);
        break;
      }
      case "decideWorld": {
        const { id, side, questions } = message;
        this.state = applyPatch(this.state, message.patch);
        const state = this.state;
        const decider = this.options.worldDeciders?.[side] ?? ruleWorldDecider;
        this.asking += 1;
        void decider
          .decide(state, side, questions, this.config)
          .catch(() => ruleWorldDecider.decide(state, side, questions, this.config))
          .then((answers) => {
            this.asking -= 1;
            const reply: ToWorker = { type: "worldAnswers", id, answers };
            this.endpoint.postMessage(reply);
          });
        break;
      }
      case "plan": {
        const { id, side, request } = message;
        this.state = applyPatch(this.state, message.patch);
        const state = this.state;
        const planner = this.options.planners?.[side];
        const seat = seatAt(state, request.hq);
        const hq = state.world?.hqs[request.hq];
        this.asking += 1;
        const fallback = (): PlanAnswer => (hq ? heuristicPlan(state, hq) : { by: "heuristic", plan: "no HQ", orders: {} });
        void (planner && seat ? planner.plan(state, request, seat).catch(fallback) : Promise.resolve(fallback())).then((answer) => {
          this.asking -= 1;
          const reply: ToWorker = { type: "planAnswer", id, answer };
          this.endpoint.postMessage(reply);
        });
        break;
      }
      case "decide": {
        const { id, side, requests } = message;
        // The replica, brought to the moment the question was asked.
        this.state = applyPatch(this.state, message.patch);
        const state = this.state;
        const decider = this.options.deciders?.[side] ?? ruleDecider;
        this.asking += 1;
        // As on the main thread: a decider must not reject; if one does, the rules answer.
        void decider
          .decide(state, side, requests, this.config)
          .catch(() => ruleDecider.decide(state, side, requests, this.config))
          .then((decisions) => {
            this.asking -= 1;
            const reply: ToWorker = { type: "decision", id, decisions };
            this.endpoint.postMessage(reply);
          });
        break;
      }
    }
  }

  private apply(snapshot: RunnerSnapshot): void {
    this.state = applyPatch(this.state, snapshot.patch);
    this.waits = snapshot.waits;
    this.remoteThinking = snapshot.thinking;
    this.awaitingOrders = snapshot.awaitingOrders;
    for (const { entry, context } of snapshot.entries) {
      this.log.push(entry);
      if (entry.type === "flag") this.flags.push(entry.flag);
      try {
        this.options.onEntry?.(entry, context);
      } catch {
        // A listener's failure is never the game's.
      }
    }
    const limit = this.options.logLimit ?? DEFAULT_LOG_LIMIT;
    if (this.log.length > limit) this.log.splice(0, this.log.length - limit);
  }
}

/** A runner in a new Web Worker (runner.worker.ts). */
export function spawnWorkerRunner(init: WorkerRunnerInit, options: HandleOptions = {}): WorkerRunner {
  const worker = new Worker(new URL("./runner.worker.ts", import.meta.url), { type: "module" });
  const handle = new WorkerRunner(worker, init, options, () => worker.terminate());
  worker.addEventListener("error", (event) => {
    handle.fail(new Error(`engine worker failed: ${event.message || "could not load"}`));
  });
  return handle;
}
