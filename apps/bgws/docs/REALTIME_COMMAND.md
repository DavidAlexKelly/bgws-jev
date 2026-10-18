# Real-time command: how it is built

The design is in [REALTIME_COMMAND_DESIGN.html](REALTIME_COMMAND_DESIGN.html). Open it in a browser. This note maps that design onto the code, lists the figures that were declared rather than taken from data, and says what is still to do.

## Who decides what

| Who | Decides | When | Code |
|---|---|---|---|
| Commander (Claude) | Each unit's mission orders: task in phases, intent, urgency, rules of engagement, lines not to cross, actions on contact, and which unit it supports. It plans from its HQ's picture, not the truth. | Only when the player presses **Generate orders**, before the start or while paused. Never on events, never on a timer. | `realtime/engine/orders.ts`; the `bgwsCommanderRealtimeOrders` query in `llmfunctions.ts` |
| Unit leader (Jev) | One option at a decision point, always inside the orders. This includes answering a friend's request (D12) and asking a friend for help. | Only at decision points D0–D12 | `realtime/engine/decisions.ts`, `jevDecider.ts` |
| Game logic | Movement, spotting, the locate roll, cues, triggers, fire, hits, morale, drills, phases, constraints. Also each unit's own picture of the enemy, and the radio net that carries reports and requests. | Every second | `realtime/engine/engine.ts`, `knowledge.ts`, `comms.ts` |

Communication between units and what each unit knows are covered in [REALTIME_COMMS.md](REALTIME_COMMS.md).

## What each unit knows (`knowledge.ts`)

- **About each enemy:** `beliefOf(state, unit, enemy)` returns *unaware*, *suspected* (a bearing only), *located*, *identified* or *lost* (a last known position and its age). It comes from what this unit has seen itself or been told by radio, not from what its side knows ([REALTIME_COMMS.md](REALTIME_COMMS.md)). Jev sees these beliefs through `beliefsOf`, never the truth. Each belief says where the information came from: in sight, seen a while ago, or reported by another unit (with its age and how far off it may be).
- **The locate roll.** It replaces the old automatic muzzle-flash reveal. When fired on by a shooter it does not know, the target always gets a bearing (`suspects`). It then rolls to locate the shooter:
  - The roll is a hazard rate: 0.7 per volley at 1 km, falling with the square of range.
  - Multipliers: ×0.5 if the shooter is hull-down, ×0.4 if it is in cover, ×2 if the target is searching that bearing, ×0.6 if the target is suppressed, ×0.3 if pinned.
  - Each further volley from the same place adds 50% to the rate. A shooter that moves 50 m resets the count.
  - Within 150 m it always succeeds.
- **Heard firing.** Anyone within 2 km of a firer who does not know it gets a bearing.
- **Self-belief.** `selfBeliefOf(unit)` returns:
  - *knownSeen* if the unit was fired on in the last 60 s;
  - *possiblySeen* after a cue in the last 90 s;
  - otherwise *unobserved*.
- **Cues.** A unit trying to stay hidden (still, not fired for 60 s) reads four behaviours in any enemy it can see within 3 km: halted, turned towards it, went to ground (a dash for cover or into hull-down), or started towards it. Each is decision point D5, at most once per enemy every 30 s. The unit sees the behaviour, never the reason.
- **Searching** (D3's "search the bearing") doubles spotting within 45° of the bearing for 30 s. It also doubles the locate roll against fire from that direction.
- **The drill** still takes cover at once. It returns fire only at a shooter the unit has located.

## Activities and orders (`types.ts`, `engine.ts`)

New order kinds:

- `wait` holds fire on a target until a trigger is met: a hit chance, a range, the target showing its side, or the target reaching a place. With `autoFire` it fires at once, already laid on, and no call is needed. Without it, it asks (D4).
- `observe` watches without firing, with spotting ×1.5.
- `search` holds still and searches a bearing.
- `engage`, `move` and `withdraw` can carry a `then`: the order that follows. `engage` can also carry `volleys` and `until`. This is how "fire and move", "hull-down then return fire" and D0's "finish this fight first" are built.

`activityOf(unit)` names the design's activities: executing its order, engaging, waiting for a trigger, manoeuvring, observing, withdrawing, or shaken/broken.

**Standing orders.** Each unit may carry `UnitOrders`: task, phases, current phase, intent, urgency, rules of engagement, `onContact`, boundaries.

- An order that carries out a phase is marked with `phase`. When it ends, the next phase starts with no decision.
- After the last phase the unit raises `outOfOrders`: decision point D10, plus a flag for the player.
- **Boundaries** are enforced in movement, which stops short of the line. They are also enforced in the option lists: an option that would cross a line is never offered.

## Decision points (`decisions.ts`)

`decisionPointOf(events)` picks the most pressing point, in this order:

D0 > D2 > D3 > D4 > D12 > D5 > D9 > D1 > D6 > D8 > D7 > D10 > D3 (search done) > D11

Anything else asks nobody: `exposed`, `friendLost`, `moraleDrop`, `review`, and informational events.

`optionsAt` builds each point's short list in a fixed order. `ruleFallback` gives the rules' choice, following the order's actions on contact.

| DP | Option ids | Rules (onContact = engage) |
|---|---|---|
| D0 now | comply · covered (asks a friend by radio to cover it) · fireAndBack | comply |
| D0 when able | comply · finish (to the target's end or 2 min) · breakContact | finish |
| D1 | keep · engage · wait*n* / wait*n*:fire · better · observe · pullBack | engage if likely to tell; lie in wait if still, unseen, and the shot is poor or would bounce. Also raised when a report of an enemy it knew nothing of arrives. |
| D2 | returnFire · hullDown · pullBack · pullBackCovered · callFire · assault · quiet | hull-down, then return fire, asking a friend to engage too (callFire) when there is one |
| D3 | search · cover · pullBack · keep | cover in the open, else search |
| D4 | fire · closer · letPass | fire |
| D5 | fireFirst · keep · relocate · pullBack | fire first only on a good shot, else keep |
| D6 | keep · fireAndMove (asking a friend to cover the move, if there is one) · shift · quiet · pullBack | keep |
| D7 | resume · watch · regain · shift | shift if there is a target, else resume |
| D8 | better · shift · quiet · pullBack · keep | better |
| D9 | help · support · keep | help. Raised when a friend's "under fire" report arrives by radio. |
| D10 | resume · hold · overwatch | resume, else overwatch |
| D11 | resume · hold · join | resume |
| D12 | comply (engage from here, or move to get a shot) · partly (overwatch from here) · keep (can't) | comply, unless it avoids fights or is in its own fight with nothing to hit; always comply if it supports the asker |

Rules that keep it sane, as built in `runner.ts`:

- One decision in flight per unit; events that arrive meanwhile fold into its next question.
- Events are held 3 s; severe events skip the 20 s cooldown; answers take effect after the crew's reaction time (5–15 s by troop quality).
- The clock waits for a late answer, so games replay exactly.
- A fight D0 allowed to finish stays time-boxed whatever the unit decides meanwhile, so it cannot drift.

## What Jev is shown (`jevDecider.ts`)

- Only the units being asked are described, each with:
  - its orders (task, current step, intent, urgency, rules of engagement, actions on contact, constraints);
  - vehicles, nerve and ground;
  - whether it thinks the enemy knows it is there;
  - its beliefs about each enemy, with both sides' odds in words;
  - friends nearby, and its last few decisions and what came of them.
- Odds and distances are bands ("likely to knock out one of its vehicles within a minute", "long range"). The exact figures go to the console in the trace, and are never sent to Jev.
- Every question starts with the balance statement: *the intent comes first; keep the unit alive and able to carry it out; take a local opportunity only when it doesn't cost the intent.*

## Orders from the commander (`orders.ts`)

- **`ordersPrompt`** gives Claude what the side knows:
  - its units, with strength, cohesion, position against named reference points, activity, and progress through current orders;
  - sighted enemies and lost contacts;
  - recent events and the player's guidance.

  Positions are written against reference points: the objective, own units and known enemies, as `{ref, bearingDeg, distanceM}`. Raw coordinates are never used.
- **`parseOrders`** checks every field. It drops unknown units, unreadable destinations, steps after one that does not end, and lines the unit is already on the wrong side of. Each drop is reported as a warning, and the warnings are shown in the review.
- **`commanderOrders`** falls back to the heuristic's orders if the call fails or the reply is unreadable, so the game can always go on.
- **In the app.** "Generate orders" works before the start, and while paused for one side or both, with a guidance box. The player reviews the orders, then "Issue orders and resume" calls `runner.issueOrders`:
  - a unit out of contact switches at once;
  - a unit in a fight is asked how to comply (D0).
- **Flags while running:** a unit out of orders, 25% and 50% losses, an objective taken. They appear on screen, and the commander is never called on its own.

**To publish:** `bgwsCommanderRealtimeOrders` must be published from `llmfunctions.ts` and the SDK regenerated. Until then `data/commanderClient.ts` falls back to `bgwsCommanderTurn`, which gets the same prompt under the turn game's brief. The prompt carries its own reply format, so this still works; only the brief is the older one.

## Declared figures

All of these are declared, with no data behind them, like `detection.ts`'s. They are the ones to tune:

| Figure | Value | Where |
|---|---|---|
| Locate rate, one volley in the open at 1 km | 0.7 (≈50%) | `knowledge.ts` |
| Locate multipliers | hull-down ×0.5 · cover ×0.4 · searching ×2 · suppressed ×0.6 · pinned ×0.3 · +50% per further volley | `knowledge.ts` |
| Shooter moved, count resets | 50 m | `knowledge.ts` |
| Heard firing | 2 km | `knowledge.ts` |
| Known seen / possibly seen | 60 s / 90 s | `knowledge.ts` |
| Cue interval, cue range, turn threshold | 30 s, 3 km, 40° | `knowledge.ts` |
| Search arc and length | ±45°, 30 s; spotting ×2 | `engine.ts`, `decisions.ts` |
| Observing spotting | ×1.5 | `detection.ts` |
| Friend help radius, interval | 1.5 km, 60 s | `engine.ts` |
| Finish-first time-box | 2 min | `decisions.ts` |
| Rules: "likely to tell" for D1 | 0.5 a minute | `decisions.ts` |

## Calibration

`realtime/engine/situations.ts` holds 19 hand-checked situations covering D0–D11. Each lists the options a sensible leader might take. `situations.test.ts` checks that the rules' choice is sensible in every one.

With `OPENROUTER_API_KEY` set, the same test file sends each situation to Jev. It prints Jev's choice and confidence, and for each threshold from 0 to 0.6, how many answers would be taken and how many of those are sensible:

```
OPENROUTER_API_KEY=… npx vitest run realtime/engine/situations.test.ts
```

Set `minConfidence` in `jevDecider.ts` (0.25 today) from that table. It has not been run yet: no key was available where this was built.

## Measured (rules deciding, generated relief)

- **Balance:** 32 games per scenario on 16 grounds. Symmetric forces: 16 blue, 16 red. Combined arms: 20 blue, 12 red.
- **Game length and decisions:** game lengths and decisions per game are as before this change.
- **Decision points:** most calls are D1 (new contact), D9 (friend needs help) and D6 (after a volley).
- **Fired on:** about 10% of the times a unit is fired on, it has not located the shooter.
- **Cues (D5):** there were none in these runs, because every unit was advancing. Cues come from units lying in wait.

## Not yet

- The "reaches a place" trigger exists in the engine, but is not yet offered as an option.
- Jev's confidence threshold is still to be calibrated (above).
