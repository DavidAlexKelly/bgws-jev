# Real-time engine scaling: step 0 results

Step 0 of [WORKER_AND_SCHEDULER.md](WORKER_AND_SCHEDULER.md): measure how the engine's cost grows with force size before changing anything for scale.

**How:** `realtime/engine/bench.test.ts`, skipped in CI. Run with:

```
BGWS_BENCH=1 npx vitest run src/apps/bgws/realtime/engine/bench.test.ts
# one size only:  BGWS_BENCH_SIZES=300
```

Each size plays two simulated minutes with the rules decider, on flat terrain, with no breakpoint, so every size runs the full time. The forces are `SYMMETRIC_CONTROL_V1` repeated along the front, each copy 1.5 km further along. Measured in the code workspace container: the absolute figures are machine-dependent; the shape is what matters.

## Scaling

| Units | ms per simulated second | Exponent k vs previous (time ∝ nᵏ) |
|---|---|---|
| 18 | 0.9 | — |
| 102 | 6.6 | 1.15 |
| 300 | 60 | 2.04 |
| 600 | 342 | 2.51 |

The cost grows roughly linearly to about 100 units, then **worse than quadratically**. At 600 units, one simulated second costs about ⅓ s of CPU, so ×10 speed would need about 3.4 s of work per wall-clock second.

## Where the time goes (CPU profile, 300 units, self time)

| Where | Share |
|---|---|
| `comms.ts` `receive` | **22%** |
| `engine.ts` `tick` (its own code) | 15% |
| `engine.ts` `knownTo` | **13%** |
| `board.ts` `distanceM` | 5% |
| `engine.ts` `sidePictureLevels` | 3% |
| `lineOfSight` | 2.5% |
| `detection.ts` | **0.7%** |

## Findings

1. **Detection is not the cost.** Scheduling detection, which the design's step 4 led with, would barely help.
2. **`receive` copies a unit's whole picture for every message it hears** (`{ ...unit.picture, [enemyId]: track }`). A broadcast contact report is heard by every friend in range, so one message costs O(friends × enemies), and the number of messages itself grows with n. That's why k exceeds 2.
3. **`knownTo` is cheap per call but called ~n² times per tick**, from `observe` and `sidePictureLevels`.

## What this changes in the plan

Do two targeted fixes **before** the worker or the scheduler, each checked against the decision golden test and re-measured with this benchmark:

- **Comms:** apply each unit's received messages as one batch per tick, copying its picture at most once per tick rather than once per message.
- **Knowledge:** compute who-knows-whom once per side per tick (or keep `knownTo` results for the tick), so the pairwise loops stop recomputing it.

The worker (design steps 1–3) still matters for a smooth map, but it moves the cost off the main thread rather than reducing it.

## Results of the two fixes

Both fixes leave the final state of every game byte-for-byte the same: a SHA-256 of `JSON.stringify(state)` at the end of the 20 golden games and a 300-unit, 60-second game, before and after each change.

**Comms (copy-on-first-write in `receive`).** 600 units: 380 → 212 ms per simulated second; k from 300 to 600 units: 2.75 → 2.02.

**Knowledge lookups.**
- `sidePictureLevels` now looks only at what each friend has in its own sightings and its picture, not at every friend × enemy pair. Keys still go in in the order the full walk would give.
- `knownEnemies` does the same for a unit that knows few enemies. For a unit that knows many, it walks the elements once instead of calling `knownTo` for each one.
- The settled-posture check finds the nearest known enemy in one pass (`nearestKnownEnemy`) rather than sorting the whole list by distance. It also skips that search entirely when the unit is already in cover.
- The "walking into reach" check goes through `knownEnemies`.

`sidePictureLevels` and `knownTo` together went from about 15% of the 600-unit profile to under 3%. Wall-time gain is smaller because pictures are dense at this size: by mid-game most units know of most enemies, so the lists are long whatever finds them.

A/B timings, master against this change, four alternating rounds on the same machine (ms per simulated second):

| Units | Master (best / median) | Change (best / median) |
|---|---|---|
| 300 | 39 / 42 | 34 / 38 |
| 600 | 188 / 214 | 163 / 176 |

That is roughly 10–18% faster. These timings vary by ±15% between runs in the workspace container, which is why each side was measured four times, alternating.

**What is left at 600 units (self time):**

| Where | Share |
|---|---|
| `tick` (its own code) | ~26% |
| `comms.receive` | ~12% |
| `distanceM` | ~6% |
| `knownEnemies` / `nearestKnownEnemy`, with their per-enemy checks | ~13% |
| `knownTo` | ~3% |

The remaining knowledge cost is per unit per tick over each unit's own, dense list: posture, the exposure check, and decisions. It can only shrink further by asking less often, not by looking things up faster. Further gains need either fewer pairs, from a spatial index for range checks, or the worker and scheduler.

## Where the time goes, by tick section (before step 4 of the scheduler plan)

The profiler's self-time split proved misleading at this point. V8 inlines heavily, and garbage collection is charged to whichever code happens to trigger it. One example: a change that halved the cost of forgetting stale reports moved almost the same amount of time into the radio section. So these figures come from wall-clock timers placed around each section of `tick`. That was a temporary instrumented copy of `engine.ts`, not committed. The run was 600 units, 120 simulated seconds after a 10-second warm-up.

| Section | Share of wall time |
|---|---|
| 1. Radio nets (`stepComms`: copying and updating unit pictures) | 20–37% |
| 4. Sighting: forgetting stale reports (every unit, every report, every step) | 24% before, 12% after `pruned` |
| 3. Running into the enemy (close contact, exposure) | 13–15% |
| 4. Sighting: detection (observer × enemy) | 12–13% |
| 7. Suppression and posture | 6–7% |
| 4. Sighting: side picture and last known | 4% |
| Outside `tick` (runner, decisions) | 7–8% |
| Everything else (fire, effects, breaking, rally, idle, triggers) | under 10% together |

Ranges are across runs and the two versions. The radio-net share rises by about as much as the forgetting share falls.

What this shows:

- **Detection is not the dominant cost.** It is about one eighth of the total.
- **One thing dominates: every unit keeps its own full picture of the enemy.** At 600 units that is about 190 tracks per unit and about 13 MB of state. Each second about 60% of units get a new copy of their picture, and every step checks every report for staleness.
  - Radio, forgetting, the knowledge lookups and the size of each worker message all scale with that picture.
  - Between them, the radio and forgetting sections account for roughly half of all time.
- **Allocation matters as much as arithmetic.** Removing allocations from one section mostly moved garbage-collection time into the next.

The step 4 decision this raises is set out in `WORKER_AND_SCHEDULER.md`, step 4.

## After the side picture: a spatial grid for range checks

After `SIDE_PICTURE.md` (slice 2), the largest sections were "running into the enemy" and detection. Both measured every pair of units and threw away most of them on range. `engine/grid.ts` is a grid of cells one search radius wide; it answers "who could be within r of here" from the 3×3 cells around a point.

- **Detection:** only observers within the 3 km sight cap of an enemy are asked about it.
- **Halting on contact:** only enemies within 300 m are measured.
- **Walking into reach:** only enemies within the sight cap are checked. Being identified already implies being known, and `canHit` already refuses beyond the cap.

**The grid only narrows; it never decides:**

- Callers keep every check they had, including the real `distanceM`.
- The projection errs wide: the cosine of the latitude farthest from the equator, metres per degree a little short, and 5% padding.
- Indices come back in ascending order, so dice are drawn in the same order as before.
- `grid.test.ts` checks the grid never misses a point within the radius, at four latitudes and three radii.

**Checked:** final-state SHA-256 was identical to master for 82 runs:

- 80 golden games: 2 terrains (flat, and generated ground with real line of sight), × 2 comms modes, × 2 force lists, × 10 seeds;
- two 600-unit, 300-second runs, one per comms mode.

The decision golden test is unchanged.

**Measured at 600 units, by section** (the same machine for both columns, and slower than in early sessions):

| Section | Before | After |
|---|---|---|
| Total | 197 ms per simulated second | 141 ms |
| Running into the enemy | 47 ms | 2 ms |
| Detection | 39 ms | 21 ms |
| Radio nets | 35 ms | 39 ms (noise) |

In the alternating A/B benchmark (3 rounds), 600 units ran at 159–214 ms on this branch against 220–292 ms on master, and 300 units at 40–52 ms against 54–84 ms.

**What is left:**

- the radio nets (about 35 ms);
- detection (about 20 ms, now line-of-sight checks within the cap);
- posture's nearest-known-enemy (about 20 ms; uncapped, so the grid does not help);
- the runner and decisions outside `tick` (about 20 ms).
