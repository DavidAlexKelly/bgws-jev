# Side picture: one record of reports per side, not one per unit

Status: **design, for review**. This is option B in `WORKER_AND_SCHEDULER.md`, step 4. There is no code yet.

## 1. Why

Measured at 600 units (`ENGINE_BENCHMARK.md`, "Where the time goes, by tick section"):

- Every unit holds its own copy of everything it has been told: about 190 tracks each, about 115,000 in all. This picture is about 13 MB of the game state.
- Each second about 60% of units get a fresh copy of their picture, because one broadcast report reaches every friend in range. Every step, every track is also checked for staleness.
- The radio nets plus forgetting stale reports take roughly half the time. Option A (exact bookkeeping) was tried and saved nothing, because the copying is the cost.
- The same picture is why every message from the background thread carries about 0.6 MB per second of play, even as patches.

The cost grows with units × enemies, and the multi-domain plan adds more of both (drones, EW). It has to grow with enemies, and with messages actually sent.

## 2. What the picture does today

These are the rules the new model must keep, or deliberately change.

- **Writing (comms `receive`).** A contact report reaches each recipient that `recipients()` picks:
  - **Perfect comms:** every living friend except the sender, `reportDelayS` (15 s) after it was sent.
  - **Radio:** within 5 km of the sender, or by way of an HQ and then within the HQ's range. Each side's net carries one message at a time, 4 s each.
- For each recipient, the report is **skipped** if:
  1. the enemy is dead;
  2. the recipient's own sighting is as recent or more recent (`own.time >= seenAt`);
  3. the recipient already has a report at least as recent and at least as good.
- Otherwise it becomes the recipient's track for that enemy, at the **better of the old and new level**.
- **Forgetting.** Every step, a track goes if the enemy is dead or it was seen more than `contactMemoryS` (120 s) ago.
- **Reading.** Fifteen read sites in four files (`engine.ts`, `knowledge.ts`, `comms.ts`, `RealtimePlay.tsx`), all on one unit's picture:
  - `knownTo` / `levelIn` and `knownEnemies` / `nearestKnownEnemy` (engine);
  - `beliefOf`, the belief list and the shared-picture builder (`knowledge.ts`);
  - the re-report check in `observe` ("a friend has just reported this");
  - comms `receive` itself;
  - the stale-report scan;
  - the selected-unit overlay in `RealtimePlay`.

Every reader asks one of two things: "this unit's track for this enemy", or "every enemy this unit has a track for". None needs the picture as a stored object.

## 3. The proposal

Each **side** keeps one record of what has been on its net about each enemy. A unit's picture is not stored. It is **the newest version of each report that the unit heard**.

```ts
interface SideKnowledge {
  /** Per enemy: versions of the report, newest first. */
  reports: Record<string /* enemyId */, ReportVersion[]>;
}

interface ReportVersion {
  track: Track;               // as today, minus receivedAt (that is `heardAt`)
  heardAt: number;            // when it was delivered
  /** Who heard this version: a bitset over the side's units, by element order. */
  heardBy: Uint8Array;        // 600 units → 75 bytes
}

// RtState gains `knowledge: Record<Side, SideKnowledge>`; RtUnit loses `picture`.
```

- **Delivery** (comms) adds one version per delivered report, with `heardBy` set to exactly the recipients `recipients()` returns today. That is O(recipients) bit sets. Nothing is copied per recipient.
- **Reading** goes through two functions that replace every `unit.picture` read:
  - `trackFor(state, unitId, enemyId)`: the newest version whose `heardBy` has the unit. Usually that is the first one.
  - `trackedBy(state, unitId)`: the enemies with such a version, in element order.
- **Forgetting** happens once per side per step, over versions rather than units × tracks:
  - a version goes when it is older than the memory, or the enemy is dead;
  - a version also goes when every unit that heard it has heard a newer one.

  Under perfect comms the newest version is heard by the whole side, so each enemy is down to one or two versions.
- **Per-unit lag** is what `heardBy` expresses. A unit out of radio range, or behind a busy net, keeps an older version until a newer one reaches it, and loses it when it expires. A version is never "pushed" to a unit, so there is no lag figure to tune. The lag comes from the comms rules as they stand.

### Cost at 600 units

| | Today | Proposed |
|---|---|---|
| Stored tracks | ~115,000 (units × tracks) | ~300 enemies × 1–3 versions per side |
| Picture memory | ~13 MB | ~0.1–0.3 MB (tracks plus 75-byte bitsets) |
| One broadcast report | copies each recipient's picture (on first write each step) | one version plus recipient bits |
| Forgetting, per step | every unit × every track | every version, once per side |
| `trackFor` | one lookup | one lookup plus a bit test (more only for lagging units) |
| Worker patch | changed pictures, about 0.6 MB per simulated second | changed versions, expected to be a few KB |

These are estimates. Slice 2 measures them.

## 4. What changes in play

This is a model change. The differences, by rule:

| Rule today | Proposed | Effect |
|---|---|---|
| Skip a report if the enemy is dead | same, once per side | none |
| Skip a report older than the recipient's own sighting | not skipped: the version is heard, and readers already prefer the unit's own sighting (`beliefOf` uses the report only with no own sighting; `knownTo` takes the better level) | a unit whose own sighting has expired may still hold a report that is older than it. Today it holds the report it had before that sighting, which is older still |
| Skip a report not newer and not better than what the recipient has | applied once, against the newest version, when the version is made | differs only for a unit that missed the newest version (radio, out of range) |
| Level = better of the recipient's old and new | better of the newest version's and the new report's level | same under perfect comms; under radio a lagging unit may see a lower level than today |
| Forget after 120 s, or when the enemy dies | same, per version | none |

Under **perfect comms** (the golden tests' default), the only real difference is the second row. Under **radio** (the app's default), lagging units differ slightly in the level and age of what they hold.

## 5. Delivery, in slices

1. **Readers through `trackFor` / `trackedBy` (exact).**
   - Replace the fifteen reads. The functions still read `unit.picture`.
   - Golden tests and final-state hashes unchanged.
   - This is a refactor that makes slice 2 a change in one place.
   - **Done** (`engine/picture.ts`):
     - `trackFor(source, unitId, enemyId)` returns this unit's report of this enemy.
     - `trackedBy(source, unitId, visit)` visits every report the unit holds, in the order held. It is a visitor rather than a list, so the hot loops do not allocate.
     - `reportsHeldBy` builds a list, for display.
     - A source is anything with `units`: the state, or `tick`'s own live view.
   - `knownTo` and `sidePictureLevels` now take a picture source rather than a `units` record.
   - A guard test fails if any file other than `picture.ts` and the two writers mentions `.picture`.
   - Final-state hashes were identical across 40 golden games, under perfect and radio comms, and two 300-unit, 400-second runs. Each hash also covers every unit's beliefs and both HQ pictures at t = 120 s. The benchmark is unchanged, within noise.
2. **The side record behind them (model change).**
   - Comms writes versions, the scan forgets versions, and `unit.picture` goes.
   - Measure the section timings, the benchmark, state size and patch size at 100, 300 and 600 units.
   - **Golden tests:** regenerate deliberately, and the PR lists which decisions changed and why.
   - **Outcomes:** check the model change is benign over 200 seeded games per force list and comms mode, old engine vs new: win rates, game length, losses, decisions asked.
   - **Done, and it turned out exact.** The rules in §4 did not need to change:
     - comms still decides, listener by listener, whether a unit takes a report and at what level (its own sighting, what it held before), exactly as before;
     - the unit then holds exactly that track;
     - listeners with identical tracks share one version;
     - a unit leaves its old version when it takes a new one, so it holds at most one per enemy.

     The skip and merge rules are therefore per unit, as before. §4's table described a cheaper side-level variant that was not needed.
   - **Differences**, neither observable in play:
     - a dead unit's picture is forgotten along with everyone else's (before, it was kept for ever, and nothing reads it);
     - `trackedBy` walks a unit's reports in the order its side first heard of each enemy, not the order that unit did.
   - **Verified.** Behaviour fingerprints were identical to master at every 10-second step of all 42 runs:
     - 40 golden games, under perfect and radio comms;
     - two 300-unit, 400-second runs.

     A fingerprint is the whole state, with each living unit's picture read through `picture.ts` and sorted, minus the storage fields. The decision golden test is unchanged; **no regeneration was needed.**
   - **Hot readers** take a `Holder` (the record and the unit's bit), worked out once per unit rather than per lookup.
   - **Measured at 600 units.**

     | | Before | After |
     |---|---|---|
     | Speed, A/B over 3 rounds, alternating (this machine was slower that day) | 302–381 ms per simulated second | 216–231 ms (about 28% faster) |
     | Speed at 300 units | 63–70 ms | 53–67 ms |
     | State | 13.0 MB | 2.4 MB |
     | Radio nets section | 89 ms | 33 ms |
     | Stale-report section | 31 ms | about 0 |
     | Knowledge readers ("running into the enemy", posture, side picture) | | about 20 ms slower: a bit test per version instead of one property lookup |
     | One second's worker patch | 102 KB | 158 KB (larger) |

     The patch is larger because the diff sends a changed enemy's version list whole, bitsets included. Diffing version lists element by element would fix it; that is not done yet.
   - **Q2 (lag): "a unit holds the newest version it took"**, as designed. It falls out of the comms rules, with no new figure.
   - **Q3 (Jev): done.** Jev's request now carries `sideKnows`. That is up to 12 enemies, newest first: the newest of what the side's living units have seen and what its net has carried, how widely it is known ("most of the side", "part of the side", "a few units"), when it was seen and by whom. It is in words, as with the rest of Jev's request.
3. **Worker and scenario follow-ups.**
   - The patch diff treats `heardBy` as a value; check that is enough, or diff versions instead.
   - Saved scenarios carry no state, so they are unaffected.

## 6. Questions for you

1. **The golden tests change in slice 2.** Is an explained regeneration, plus the 200-game outcome comparison, enough? Or do you want something more specific held fixed?
2. **Lagging units (radio).** Is "a unit keeps the newest version it heard" the behaviour you want? The alternative is to give each unit an explicit delay behind the side's record, which is simpler, but invents a number the comms rules don't have.
3. **Jev.** It reads a unit's beliefs, which would come from `trackFor`, so its view is unchanged in kind. Should the side record also be offered to Jev as "what the side knows" (the commander's view)? That would be new, and separate from this work.
