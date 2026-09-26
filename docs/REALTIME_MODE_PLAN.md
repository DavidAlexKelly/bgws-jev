# Plan: a real-time mode, with Jev in command

## 1. What it is

1. Place units, as now.
2. Generate **initial orders**. An LLM, the heuristic or Jev writes a plan and gives every unit a standing order ("advance to the ridge and hold it", "overwatch the road").
3. Press **Play**. Every unit moves, looks and shoots **at the same time**, on a shared clock that you can pause, speed up or slow down.
4. Whenever **something happens to a unit** (it sights an enemy, is shot at, loses a friend, reaches its destination), Jev is given the current picture and **sets that unit a new order**. Jev makes every decision once the game is running.

This is a different game from the turn mode. The rulebook is built on turns and alternating activations, so real-time is a **variant**. It gets its own engine and **reuses** the turn mode's rules, maths and Jev plumbing. It does **not** modify the turn engine. Turn mode stays exactly as it is.

## 2. The core idea: an autopilot for every unit, and Jev changes its orders

Jev is fast, but not fast enough to steer 14 units 20 times a second. It shouldn't have to. Two layers:

| Layer | Runs | Does |
|---|---|---|
| **Autopilot** (deterministic, in the engine) | every tick | carries out the unit's current order: follow the route at terrain speed; fire at the ordered target when it is in range, in sight and the weapon is ready; on overwatch, shoot movers according to the rules of engagement; stop at the destination |
| **Jev** (decisions) | only when an **event** happens to a unit | picks the unit's **next order** from options the rules generate, using the current situation |

This is how a real crew works. It keeps doing what it was told until something happens, then its commander gives it a new order. It also keeps Jev calls proportional to how much is happening. A quiet approach march costs almost nothing, and a firefight costs a lot of small, cheap calls.

### Orders (what the autopilot can carry out)

```ts
type RealtimeOrder =
  | { kind: "move"; to: LatLng; route?: LatLng[]; purpose?: string }
  | { kind: "hold"; purpose?: string }                  // stay, fire in self-defence per ROE
  | { kind: "overwatch"; sectorDeg?: [number, number] }  // stay, engage movers per ROE
  | { kind: "engage"; targetId: string }                 // stay, fire at this target when able
  | { kind: "assault"; targetId: string }                // close and assault
  | { kind: "withdraw"; to: LatLng };                    // move away, no stopping to fire
// plus, on every unit: roe ("never" | "ifFiredUpon" | "withinShortRange" | "always")
```

## 3. Time

- **Simulation clock in seconds, fixed tick**: one tick is 1 simulated second. Everything is computed per tick. The screen shows the ticks at a **speed multiplier** (×1, ×5, ×10 by default, ×30) and interpolates positions between ticks so the movement is smooth.
- **Pause** stops the clock, and nothing is decided while paused.
- **Converting per-turn rules to per-tick rules.** One turn is 15 minutes (`DEFAULT_TURN_MINUTES`). A rule that happens with probability *p* per turn happens with probability `1 − (1 − p)^(dt / 900 s)` per tick. This covers sighting, rally, morale recovery and so on.
- **Movement:** speeds come from the terrain speed table the router already uses (km/h per mobility class and terrain), so it's direct.
- **Fire:** in the turn game, one fire result stands for about a turn's worth of engagement. In real time, a unit that is engaging resolves a shot on each **engagement cycle** *C* (default 180 s), using the same fire table and a lethality scale *k*. *C* and *k* are the main things to **calibrate** (see §9).

## 4. Events: when Jev is asked

The engine checks for these each tick and queues a decision for the affected unit:

| Event | Example |
|---|---|
| `sighted` | this side has newly partially or fully sighted an enemy that this unit can see |
| `underFire` | this unit was shot at |
| `hit` / `moraleDrop` | it lost strength or became suppressed, disrupted or broken |
| `friendLost` | a friend within 1 km was destroyed or broke |
| `targetGone` | its engage or assault target died, broke or went out of sight |
| `arrived` | it reached its move destination |
| `exposed` | it moved into an identified enemy's sight and range (as in turn mode) |
| `blocked` | its route is impassable, or it has made no progress for a while |
| `ammoLow` | below a threshold, when ammunition rules are on |

**Four guards keep this from turning into constant nagging:**
- **Coalescing:** events for the same unit within about 3 simulated seconds become one question.
- **Cooldown:** a unit isn't asked again within about 20 simulated seconds, unless the event is severe (hit, broken, assaulted).
- **Batching:** all of one side's pending questions in the same tick go in **one Jev request**, one question per unit. Jev answers them in parallel, so ten units cost one round trip.
- **Stability:** "carry on as ordered" is always an option. If Jev isn't confident, the unit carries on.

## 5. Reaction time makes it deterministic and replayable

Jev takes 70–500 ms to answer, and network time varies. If an answer took effect whenever it arrived, the same game would play out differently each time. So:

- Each decision takes effect at **event time + reaction time**, in simulated time. Reaction time is about 5–15 s, shorter for better troop quality. That's realistic (crews don't react instantly) and fixed.
- The request goes out as soon as the event happens. If the answer isn't back by the time it's due, the simulation **waits** for it. At normal speeds that should almost never happen: 5 s of simulated time at ×10 is 500 ms of real time.
- All randomness comes from the seeded dice. Answers are recorded against the simulated time, unit and event, and the persistent cache already does most of this. A replay re-runs the simulation and reads the recorded answers instead of calling Jev.

This keeps what the turn engine already guarantees: the same seed and the same decisions give the same game.

## 6. What Jev chooses from, and what it's told

The options are generated by the rules, as in turn mode, so Jev can't make anything up. One choice question per unit:

- **carry on** with the current order (always offered; the default)
- **hold here** / **overwatch here**
- **engage X**, for each enemy in sight and range, with its exact odds from `fireOdds`
- **assault X**, when close enough
- **move to** each of the terrain-aware positions (cover, overwatch, withdraw, flank), reusing the geometry from turn mode's `tacticalMoveOptions`
- **resume the plan**: move towards the unit's original objective
- **rules of engagement:** either part of the same choice or a second question in the same request

The state Jev gets reuses turn mode's work: `ownBrief`, `situationOf` (objective, support, threats, relative height), recent events (now time-stamped), fog of war from `projectForSide`, and the commander's plan and the unit's **purpose**. Plus two real-time facts: **what the unit is doing right now** (its order and progress) and **what just happened** (the event).

## 7. Initial orders, and the LLM's role

- **Before Play:** "generate initial orders" asks the commander for a plan and one order per unit. That's the LLM (a real-time version of `bgwsCommanderTurnJev` whose options are real-time orders), the heuristic, or Jev. It is drawn on the map as intentions, as now, and you can regenerate.
- **During play, Jev decides everything.** Optionally, the LLM can **re-plan** in the background at big moments: the objective lost, a third of the force gone, or every 10 simulated minutes. That call is slow (seconds) but never holds up the simulation. When the new plan arrives, it updates the units' purposes and the plan Jev reads, and doesn't directly override Jev's current orders. Off by default, so that at first Jev really does control everything.

## 8. How the code is laid out

```
rules/realtime/
  clock.ts        simulation time, tick, speed, pause
  timescale.ts    per-turn → per-tick conversions, engagement cycle, calibration constants
  state.ts        RealtimeState: units + current order + progress + weapon cooldowns + sighting ages
  autopilot.ts    one tick of carrying out orders (pure, deterministic)
  events.ts       detecting events each tick; coalescing, cooldown, severity
  decider.ts      RealtimeDecider interface; the rule-based default (carry on / engage what shoots you)
  jevRealtime.ts  Jev version: batching per side, options, state, reaction-time scheduling, console log
  engine.ts       runRealtime(): tick loop; resolving fire, sighting and morale with the existing resolvers
  replay.ts       recorded run; replay without calling any model
lib/realtimeGame.ts   the play screen's handle: start, play, pause, speed, step, snapshot
RealtimePlay.tsx      (or a mode inside Play.tsx) map animation, controls, event feed, decision marks
```

**Reused as is:** the resolvers (fire, sighting, morale, rally), line of sight, terrain and movement tables, route planner, fog of war, `fireOdds`, `jevState` helpers, the Jev client and cache, and the console printer.

**The engine core is pure.** It takes a state and some time, and returns a new state plus events. That makes it testable without a browser and runnable headless for trials, as the turn engine is.

## 9. Risks, and how each is handled

| Risk | Handling |
|---|---|
| **Calibration.** The fire and morale tables were tuned for 15-minute turns. | Build the engine headless first. Run heuristic-vs-heuristic real-time games through the existing harness and tune *C* and *k* until casualty rates and game length roughly match turn mode. Only then add Jev. |
| **Performance.** Line-of-sight checks every tick for every pair of units. | Stagger sighting checks (each pair every ~5 s), cache line of sight by rounded position, and move the tick loop into a Web Worker if needed. The raster terrain is the expensive case. |
| **Map rendering.** The play screen rebuilds every counter when the state changes, which is too slow at 30 fps. | Add a fast path that only calls `setLngLat` on existing markers each frame, and rebuilds only when units appear or disappear. |
| **Too many calls.** A big firefight generates a lot of events. | Coalescing, cooldowns and per-side batching (§4). Expect a few requests per real second at ×10 during contact, at about $0.0001 each. |
| **Dithering.** A unit flips between orders. | Cooldown, "carry on" as the default, and a small preference for carrying on built into the instructions. Sampling ("vary decisions") stays off by default here. |
| **The mode is not the rulebook.** | Label it clearly as a variant, and never change the turn engine for it. |

## 10. Order of work

1. **Headless core with the rule-based decider:** clock, time scaling, state, autopilot, events, engine. Tests for movement, fire, sighting and events.
2. **Calibration:** real-time vs turn-mode runs in the harness; tune *C* and *k*.
3. **Jev real-time decider:** options, state, batching, cooldowns, reaction-time scheduling, console output, recorded answers. Tests with a fake Jev, as now.
4. **Initial orders:** heuristic and Jev first, then the LLM query for real-time orders.
5. **UI:** mode switch, animation loop with fast marker updates, play/pause/speed, event feed, decision marks, fire lines.
6. **Replay and Trial support:** heuristic + Jev vs heuristic in real time; optional LLM re-plan.

Steps 1–2 are the foundation, and nothing after them is trustworthy without them.

## 11. What has been built (first version)

**Entry.** `/bgws/play` now opens a splash that chooses the mode (`Play.tsx`,
kept in the URL as `?mode=turn` or `?mode=realtime`). The turn-based screen
moved to `TurnPlay.tsx` **byte for byte**. No other existing file changed, and
the turn engine's event logs are still identical.

**Real-time, all in `realtime/`:**

| File | What |
|---|---|
| `engine/types.ts`, `engine/timing.ts` | orders, state, events; the time scale (`engagementCycleS` 300 s, sighting every 30 s per pair, contact memory 120 s, reaction 5–15 s by troop quality) |
| `engine/engine.ts` | the pure tick: broken units withdraw, movement at terrain speed, staggered sighting and fading contact, **simultaneous** fire, morale recovery, victory |
| `engine/options.ts`, `engine/geometry.ts` | a unit's legal next orders: carry on, hold, overwatch, engage X (with odds), move to cover / overwatch / withdraw / flank, advance on the objective |
| `engine/runner.ts` | the clock: coalesces events, cooldowns, severity, one batch per side, answers applied at event time + reaction time, and the clock **waits** for a late answer (so games replay exactly) |
| `engine/deciders.ts` | the rule decider (default and fallback) |
| `engine/jevDecider.ts` | Jev in command: one request per side per moment, "carry on" when unsure, rules when unreachable, console output |
| `engine/initialOrders.ts` | opening orders: heuristic, or Jev (order + rules of engagement per unit, one request) |
| `RealtimePlay.tsx` | setup (placement, force lists, Jev in command, directives, ground and seed), generate initial orders, play/pause, ×1–×60, umpire/blue/red views, moving counters, order and fire lines, decision tags, live feed |

**Findings from the first headless runs:**
- The first engine resolved fire in list order, so blue shot first. On open
  ground blue won 15 of 20 identical games. Fire is now simultaneous: 56–44
  over 100 games, which is within chance.
- On the default generated ground (`baltic-v1`), red wins 20 of 20 identical
  games. That is the ground, not the engine: on a different seed the same
  forces are even. It is worth knowing when judging a result.
- Many games end on the time limit with little contact. Calibration (step 2
  of §10) is the next piece of work.

**Not yet:** assaults, ammunition, concealment, smoke and indirect fire in
real time; LLM-written opening orders and the background re-plan; a Trial
page for real time; calibration against the turn game.
