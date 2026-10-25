# Engine off the main thread, and a scheduler for scale

Design for the last two W0 steps of [MULTI_DOMAIN_PLAN.md](MULTI_DOMAIN_PLAN.md): the **Web Worker host** and the **event scheduler**. This document is the plan; each numbered step below is one PR, merged only when the decision golden test (`realtime/engine/decisions.golden.test.ts`) passes unchanged.

## Where things stand

- `engine.ts` → `tick(prev, config)`: **pure**, fixed-step. Knows nothing about decisions.
- `runner.ts` → `RealtimeRunner`:
  - owns the clock and decides when units are asked and when answers take effect;
  - **waits in simulated time** for decider answers (Jev, the commander, both network calls). This is what makes a game replayable: the same seed and the same answers give the same game, however slow the network.
- `RealtimePlay.tsx`:
  - builds `RtConfig`;
  - creates the runner with React-side deciders (`jevCall`, `foundryModelCall`);
  - drives it from a `requestAnimationFrame` loop, reading `runner.state` to draw.
- `RtConfig` holds things a worker cannot receive in a message: `terrain` (a sampler with methods), `rng` (stateful), `planner` (the tiled router, which already runs in its own worker), and `isPassable` (a function).

## Goals and non-goals

**Goals**
1. A battalion-plus fight (hundreds of units) runs without the map stuttering: the engine never blocks a frame.
2. Tick cost grows with **what happens**, not with units × units every tick.
3. Replays stay exact: same scenario + seed + answers ⇒ same game, byte for byte.

**Not goals:** changing rules or behaviour, multi-player, or running deciders (LLM calls) in the worker.

## Step 0: measure before changing anything

- **Add `scripts/` or a Vitest bench** that builds forces of 20, 100, 300 and 600 units (scaled force lists) and records:
  - ms per `tick`;
  - ms per simulated minute;
  - time in detection, fire and comms.
- **Why:** without numbers we can't tell which of steps 1–4 is needed first, or confirm that they helped. It's also the acceptance test for "battalion-plus runs smoothly".
- **Expected result** (to confirm): detection and line-of-sight are pairwise (O(n²)) per tick and dominate above ~100 units.

## Step 1: make `RtConfig` describable as data

A worker needs to rebuild the config from plain data.

- **Introduce `RtConfigSpec`:**
  - ruleset id;
  - timing;
  - comms;
  - behaviours and their template copies;
  - the RNG **seed** (not the generator);
  - a terrain **descriptor**: ground choice, seed, raster dataset id and its bytes or a URL;
  - planner settings.
- **Add `configFromSpec(spec)`**, which builds the live objects.
- **`RealtimePlay` builds the spec**, then calls `configFromSpec`. On the main thread this changes nothing yet.
- **Test:** `configFromSpec(spec)` gives the same game as today's hand-built config (golden + a seeded-game hash).
- **To confirm:** how `terrain` is built from the raster today, and whether its bytes can be transferred (`ArrayBuffer`) rather than refetched in the worker.

**Done** (`realtime/engine/configSpec.ts`). What it found and decided:

- **Terrain.** `TerrainRaster`'s whole loaded state is one grid: five numbers and a `Uint8Array`, after its water clean-up. `exportGrid()` and `TerrainRaster.fromGrid()` were added, so a worker gets the raster as one transferable buffer and never fetches it. A test shows a rebuilt raster answers flags, passability and A* routes identically.
- **Raster stays beside the spec.** The spec names the raster (`ground.rasterRid`); the loaded raster is passed alongside it (`configFromSpec(spec, { raster })`) rather than inside it. The spec stays small, and a raster ground with no raster throws rather than quietly playing on generated ground.
- **Ground.**
  - `ground` = source (`generated` / `raster+relief` / `raster`), seed, and origin, the board centre as resolved.
  - The planner needs no settings of its own: A* on the raster when there is one, the bearing planner otherwise.
- **Timing held a function.** `RtTiming.reactionS` (reaction time by troop quality) is code, not data. The spec carries `RtTimingSpec` (timing without it), and `configFromSpec` puts back `DEFAULT_TIMING.reactionS`. `timingSpec()` throws for any other reaction function rather than silently changing it.
- **Rulesets and dice.**
  - Rulesets are named by id (`RULESETS`, only `house-v3` today).
  - The dice are a seed plus an optional `rngCursor`, so a game can be resumed mid-way.
- **What was checked.**
  - `RealtimePlay` now builds a spec and calls `configFromSpec`.
  - Seeded games give identical final-state hashes through the spec and through the old hand-built config: generated, raster and raster+relief ground, and the raster rebuilt from its exported grid.

## Step 2: the worker host, with deciders staying on the main thread

**The worker** (`realtime/engine/worker.ts`) owns `tick` and the runner. Messages:

| Direction | Message | Payload |
|---|---|---|
| main → worker | `init` | `RtConfigSpec`, initial `RtState` |
| main → worker | `advance` | seconds (simulated) |
| main → worker | `issueOrders` | orders, plan |
| worker → main | `frame` | the state the map draws (see step 3), time, flags, log entries |
| worker → main | `decide` | side, batch of `RtDecisionRequest` (+ the state slice the decider needs) |
| main → worker | `decision` | answers for that batch |

- **Deciders stay where they are** (they need React-side clients and network credentials). The worker's runner gets a *proxy decider* that posts `decide` and awaits `decision`. The runner already waits for answers in simulated time, so **determinism doesn't depend on message timing**. That's the existing design, just across a thread boundary.
- **A `RunnerHandle` interface** with the same surface `RealtimePlay` uses today (`advance`, `issueOrders`, `state`, `thinking`, `awaitingOrders`, `recentFor`), with two implementations: in-thread (today's) and worker. One switch chooses; tests use in-thread.
- **Tests:**
  - the same seeded game through both handles ends in identical state (hash);
  - the golden test is unchanged.

**Done.** As built, with the differences from the plan above:

- **Files.**
  - `engine/workerHost.ts`: `hostRunner(endpoint)`, which works over any message endpoint.
  - `engine/runner.worker.ts`: the worker entry, a single line that calls `hostRunner`.
  - `engine/runnerHandle.ts`: `RunnerHandle`, with two implementations, `InThreadRunner` and `WorkerRunner`.
- **The switch.** The Command step has a new "Advanced: engine" switch, off by default.
- **Messages.**
  - `init` carries the spec, the raster grid (as a transferred buffer) and the initial state.
  - The worker answers every `advance` and `issueOrders` with one `done` snapshot: state, the new feed entries, waits, thinking, and awaitingOrders.
  - Frames are not pushed. Step 3 will slim the snapshot.
- **`issueOrders` returns a promise.** In the worker, the orders land when the reply comes back.
- **Feed streaming needs a context, not the state.** `onEntry` now gets `EntryContext` (`feed.ts`: the unit's position and a shot target's side, as they stood when the entry was made). The worker sends that context with each entry, so streamed rows are identical either way.
- **Who decides.**
  - A side with a decider on the main thread is proxied to it.
  - A side without one is decided by the rules inside the worker, as before.
  - Deciders use no dice, so the main thread's own config, built from the same spec, is enough for them.
- **Verified.**
  - Over a real `MessageChannel`, so every message is structured-cloned, these all match the in-thread runner exactly:
    - final state, feed, flags and entry contexts, for 3 games;
    - slow main-thread deciders (same questions, same decisions);
    - orders issued mid-game;
    - raster ground.
  - The **built** worker bundle, run in a Node worker thread, also played a seeded game to the identical final state. That check was a one-off and isn't committed.
## Step 3: send the map only what it draws

Posting all of `RtState` every frame is a large structured clone.

- **`frameView(state)`**: per unit, position, heading, side, status, the last shot and its detections. Plus side summaries and flags. Sent every animation frame at most.
- **The full state only on demand**: inspecting a unit, end-of-game, and save/replay.
- **Measure:** frame message size and post time at 600 units (from step 0).

**Done, but as patches, not a frame view** (`engine/statePatch.ts`). Measuring came first and changed the plan.

- **The state is big.** In the benchmark's scaled games after a minute of play:

  | Units | State (serialised) | Structured clone |
  |---|---|---|
  | 100 | 0.66 MB | 11 ms |
  | 300 | 3.7 MB | 69 ms |
  | 600 | 13 MB | 246 ms |

  Nearly all of it is each unit's **picture** of the enemy, which grows as units × enemies. Reading the message costs the main thread about as much as sending it costs the worker.
- **A frame view wouldn't have been enough.** The map needs little, but the inspector, the selected unit's picture overlay, the commander and Jev all read much more. They would have needed the full state on demand anyway, including on every decision.
- **One simulated second changes only about 5% of the state.** The engine never mutates a state it has handed out, so "changed" just means "a different object".
- **So the worker sends patches** and the main thread keeps an **exact replica**.
  - `diff(prev, next)` walks only changed objects, to depth 5 (state → units → id → picture → enemy).
  - `applyPatch` copies only along the changed paths.
  - **Key order is preserved:** an object whose keys moved, such as a key deleted and re-added, is sent whole. Iteration order matters to the engine and the deciders.
  - A decider's question carries the patch up to the moment it was asked, so deciders read the replica.

**Measured.** Main-thread cost is reading the message plus applying the patch; worker cost is the diff plus serialising. Times are per simulated second:

| Units | Message | Main thread | Worker |
|---|---|---|---|
| 300, full state | ~4.3 MB | 23–32 ms | 17–22 ms |
| 300, patch | 0.2–0.3 MB | **5–8 ms** | 8–19 ms |
| 600, full state | ~14.8 MB | 86–214 ms | 69–107 ms |
| 600, patch | 0.55–0.68 MB | **18–39 ms** | 32–107 ms (the first is JIT warm-up) |

- **Remaining.** At 600 units a reply still costs the main thread one or two frames. Most of it is copying changed pictures. If that matters, the next lever is to leave pictures out of the replica except for the selected unit and the units being asked a question. The patches make that a small change, but it is no longer exact, so it isn't done here.
- **Verified.** `statePatch.test.ts` includes 500 rounds of random copy-on-write edits. `runnerHandle.test.ts` checks that the replica's hash equals the in-thread game's after **every** simulated second of two whole games, with deciders on the main thread. The built worker bundle passed the same lockstep check in a Node worker thread.

## Step 4: the event scheduler

Replace "check everything every tick" with "do what is due":

- **A priority queue of future events keyed by simulated time:** arrivals at waypoints, reload complete, suppression decay end, comms deliveries (already time-stamped in `comms.ts`), decision due times (already in the runner), and detection checks.
- **Detection becomes scheduled**, not scanned:
  - each observer–target pair within plausible range gets a next-check time from its detection rate (`figures.ts`: `detection.BASE_RATE_PER_S` and the `FACTORS`);
  - a coarse spatial grid limits the candidate pairs;
  - pairs are re-scheduled when either side moves more than a threshold or changes posture.
- **Determinism:** ties are broken by a stable key (time, kind, unit id), never by insertion order or `Map` iteration.
- **Do it in slices,** one subsystem per PR (comms → reloads/suppression → movement arrivals → detection), each with the golden test unchanged, or a deliberate, explained snapshot update where a rate was previously quantised to the tick.
- **To confirm in step 0:** whether detection is in fact the dominant cost. If movement or line-of-sight dominates, schedule that first.

**Re-profiled before starting** (`ENGINE_BENCHMARK.md`, "Where the time goes, by tick section"). **Neither detection nor movement dominates.** The dominant cost is each unit's own full enemy picture: copying it in comms, scanning it for stale reports, and looking things up in it. A scheduler as planned above would mainly speed up detection, about one eighth of the time. It would also reorder events, so the golden snapshots would change.

The options, for a decision before any code is written:

- **A. Exact bookkeeping, no rule change.**
  - Keep a lower bound on each picture's oldest report. Comms already knows when it writes one.
  - Scan a picture for stale reports only when that bound says something has expired, or when an enemy has died.
  - Golden snapshots stay unchanged. The expected gain is most of the forgetting section, about 10% of the total. Radio copying is untouched.
- **B. A side-level picture with per-unit lag (a model change).**
  - Each side keeps one shared picture.
  - Each unit holds only how far behind it is, plus what it saw itself.
  - This removes the units × enemies growth in memory, comms, forgetting and lookups, and in each worker message.
  - Game results change, and the golden snapshots would be regenerated deliberately with an explanation.
  - Needs its own short design: how radio delay and range become per-unit lag.
- **C. The scheduler as written above.** Detection first, as planned. This is the smallest gain of the three for the most churn.

The `pruned` change, which makes the stale-report scan allocation-free, is exact and in, whichever option is chosen.

**Option A was built, measured and not kept.** The design:

- Each picture object carried a note: a lower bound on its oldest report, plus how far through the game's death log it was known to be clear.
- Comms kept the note up to date as it wrote.
- A picture was scanned only when the note could not prove that nothing would go.

It was exact: final states were identical for 40 golden games under both comms modes, and for two 300-unit, 400-second runs where reports actually expire. Unit tests covered the note rules.

It saved nothing, for two reasons:

- **Most pictures still need scanning.** At 600 units with fights to the death, someone dies almost every second, and it is usually an enemy everyone has heard about. So about 60% of pictures really do lose a report each step, and removing one key from a ~190-key object still means copying the object.
- **Even when skipped, it didn't help.** The stale-report section stayed at about 22 ms per simulated second. Total time was 164–167 ms per simulated second, against 158–179 for master and 166–178 for `pruned` alone, all within noise.

The added coupling between comms and the engine was not worth that, so it was removed. The real cost is copying units × enemies pictures. That is option B, designed and now built in `SIDE_PICTURE.md`.

**The event scheduler** itself arrived with the world layer (MULTI_DOMAIN_PLAN W0): world systems schedule what is due (`schedule` / `onScheduled` in `world/context.ts` and `world/step.ts`), ties broken by a stable key. The land engine keeps its fixed step, so its golden snapshots are unchanged. The worker host also carries world deciders and HQ planners to the main thread, and a person's commands and seat changes from it (W15).

## Step 5: scope wiring

As in MULTI_DOMAIN_PLAN W0: done (`scope.ts`, `domainMode`).

## Risks

| Risk | Mitigation |
|---|---|
| Worker and in-thread games diverge | Both implement `RunnerHandle`; a seeded-game hash test runs both. |
| Structured-clone cost eats the gain | Step 3 (`frameView`); transfer typed arrays for positions if needed. |
| Terrain can't be rebuilt in the worker | Step 1 confirms first; fallback: the worker asks the main thread for terrain samples in batches. |
| Scheduler changes results subtly | One subsystem per PR; golden test plus seeded-game hashes. Deliberate changes are explained in the PR. |
| Debugging gets harder | The in-thread handle remains (and is what tests use); one flag switches. |

## Order and size

| Step | Size | Can ship alone |
|---|---|---|
| 0 Measure | S | ✅ |
| 1 Config as data | M | ✅ (no visible change) |
| 2 Worker host | L | ✅ behind a flag |
| 3 Frame view | M | ✅ |
| 4 Scheduler (per subsystem) | L, in 4 slices | ✅ each slice |
