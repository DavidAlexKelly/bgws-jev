// ── workerHost.ts ────────────────────────────────────────────────────────
// Step 2 of docs/plan/WORKER_AND_SCHEDULER.md: the runner's side of the
// thread boundary. `hostRunner(endpoint)` answers the main thread's messages
// with a `RealtimeRunner` it builds from a config spec (configSpec.ts).
//
// It knows nothing about Web Workers as such: the endpoint is anything with
// postMessage / addEventListener — the worker's global scope in the browser
// (runner.worker.ts), a MessagePort in the tests.
//
// DECIDERS STAY ON THE MAIN THREAD. A side with a decider there gets a proxy
// here that posts the questions and waits for the answers. The runner already
// waits for answers in SIMULATED time, so the game does not depend on how
// long the round trip takes: the same seed and the same answers give the
// same game, in a worker or not. A side without one is decided by the rules,
// here, as in the main-thread runner.
//
// THE MAIN THREAD KEEPS A REPLICA (step 3). Every reply carries a patch from
// the state last sent to the state now (statePatch.ts), not the state: at 600
// units the state is ~13 MB and a second of play changes a few per cent of it.
// A question for a decider carries the patch up to the moment it was asked,
// so the decider reads the replica, exactly the state the runner asked from.

import { TerrainRaster, type RasterGridData } from "../../../../shared/routing/terrainRaster";
import type { Side } from "../../lib/state";
import { configFromSpec, type RtConfigSpec } from "./configSpec";
import type { RtDecider, RtDecision, RtDecisionRequest } from "./deciders";
import { entryContext, type EntryContext } from "./feed";
import { RealtimeRunner, type RtLogEntry } from "./runner";
import { diff, type Patch } from "./statePatch";
import type { RtState, UnitOrders } from "./types";

/** Anything messages can be posted to and heard from: a Worker, its global scope, a MessagePort. */
export interface MessageEndpoint {
  postMessage(message: unknown, transfer?: Transferable[]): void;
  addEventListener(type: "message", listener: (event: MessageEvent) => void): void;
  removeEventListener(type: "message", listener: (event: MessageEvent) => void): void;
}

/** What the main thread sees of the runner after each request: enough to draw, list and stream. */
export interface RunnerSnapshot {
  /** From the state last sent (with the previous reply or question) to the state now. */
  patch: Patch | null;
  /** Feed entries made since the last snapshot, each with its context (feed.ts). */
  entries: { entry: RtLogEntry; context: EntryContext }[];
  waits: number;
  thinking: boolean;
  awaitingOrders: string[];
}

export type ToWorker =
  | {
      type: "init";
      spec: RtConfigSpec;
      /** The terrain raster's grid, when the ground uses one (TerrainRaster.exportGrid). */
      raster: RasterGridData | null;
      state: RtState;
      /** Sides whose decider is on the main thread. */
      proxied: Side[];
      logLimit?: number;
    }
  | { type: "advance"; id: number; seconds: number }
  | { type: "issueOrders"; id: number; orders: Record<string, UnitOrders>; plan?: { side: Side; text: string } }
  | { type: "decision"; id: number; decisions: RtDecision[] };

export type FromWorker =
  | { type: "done"; id: number; snapshot: RunnerSnapshot }
  /** `patch` brings the replica to the state the question was asked from. */
  | { type: "decide"; id: number; side: Side; requests: RtDecisionRequest[]; patch: Patch | null }
  | { type: "error"; id: number; message: string };

/** Serve a runner over `endpoint`. Returns a function that stops listening. */
export function hostRunner(endpoint: MessageEndpoint): () => void {
  let runner: RealtimeRunner | null = null;
  let initError: string | null = null;
  let entries: RunnerSnapshot["entries"] = [];
  /** The state the main thread's replica is at. */
  let sent: RtState | null = null;
  const patchTo = (state: RtState): Patch | null => {
    const patch = diff(sent, state);
    sent = state;
    return patch;
  };
  let nextAsk = 0;
  const answers = new Map<number, (decisions: RtDecision[]) => void>();
  const post = (message: FromWorker) => endpoint.postMessage(message);

  const proxy = (side: Side): RtDecider => ({
    name: `main-thread (${side})`,
    decide(state, _side, requests) {
      const id = nextAsk++;
      return new Promise<RtDecision[]>((resolve) => {
        answers.set(id, resolve);
        post({ type: "decide", id, side, requests, patch: patchTo(state) });
      });
    },
  });

  const snapshot = (): RunnerSnapshot => {
    const r = runner!;
    const out = { patch: patchTo(r.state), entries, waits: r.waits, thinking: r.thinking, awaitingOrders: r.awaitingOrders };
    entries = [];
    return out;
  };

  // One request at a time, in the order they came: an advance never overlaps
  // an order hand-over. Decision answers are not queued — an advance waits on them.
  let queue: Promise<void> = Promise.resolve();
  const serve = (id: number, work: (r: RealtimeRunner) => Promise<void> | void) => {
    queue = queue.then(async () => {
      try {
        if (!runner) throw new Error(initError ? `engine failed to start: ${initError}` : "runner not initialised");
        await work(runner);
        post({ type: "done", id, snapshot: snapshot() });
      } catch (thrown) {
        post({ type: "error", id, message: thrown instanceof Error ? thrown.message : String(thrown) });
      }
    });
  };

  const listener = (event: MessageEvent) => {
    const message = event.data as ToWorker;
    switch (message.type) {
      case "init": {
        try {
          const raster = message.raster ? TerrainRaster.fromGrid(message.raster) : null;
          const config = configFromSpec(message.spec, { raster });
          const deciders = Object.fromEntries(message.proxied.map((side) => [side, proxy(side)]));
          entries = [];
          initError = null;
          // The main thread starts from the state it sent.
          sent = message.state;
          runner = new RealtimeRunner(message.state, config, {
            deciders,
            logLimit: message.logLimit,
            onEntry: (entry, state) => entries.push({ entry, context: entryContext(entry, state) }),
          });
        } catch (thrown) {
          // Reported on the first request, which is where the main thread is listening.
          runner = null;
          initError = thrown instanceof Error ? thrown.message : String(thrown);
        }
        break;
      }
      case "advance":
        serve(message.id, (r) => r.advance(message.seconds));
        break;
      case "issueOrders":
        serve(message.id, (r) => r.issueOrders(message.orders, message.plan));
        break;
      case "decision": {
        const resolve = answers.get(message.id);
        answers.delete(message.id);
        resolve?.(message.decisions);
        break;
      }
    }
  };
  endpoint.addEventListener("message", listener);
  return () => endpoint.removeEventListener("message", listener);
}
