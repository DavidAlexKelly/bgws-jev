# Plan: live, in-the-moment decisions with Jev

## 1. How the engine decides today

A general-purpose LLM (the `bgwsCommanderTurn` Foundry query, called through
`data/commanderClient.ts`) plays each side **once per turn**:

- `buildOrdersPrompt` → the model picks one option id per element, plus
  standing orders (ROE) and reserves.
- `buildCounteractionPrompt` → a second call after the main round.

Everything *inside* the turn is then resolved deterministically. The choices a
real crew would make at the moment they happen are **pre-committed** and
evaluated by fixed rules:

| Moment | Where | What decides it now |
|---|---|---|
| An enemy crosses my arc: do I shoot? | `runReactiveFire` → `willReact` (`rules/turnLoop.ts:1664`, `:1737`) | A 4-value ROE chosen at turn start (`never / ifFiredUpon / withinShortRange / always`) |
| Which reactors fire, in what order | `.slice(0, maxReactorsPerAction)` in `runReactiveFire` | List order |
| I bump into a contact mid-move: halt or press? | `option.onContact` → `walkUntilContact` (`rules/turnLoop.ts:1297`, `rules/contact.ts:94`) | Pre-committed on the move option |
| Which enemy tries to spot a concealed mover | `attemptSightingInterrupt` (`rules/turnLoop.ts:1594`) | Nearest observer |
| Per-activation choice (ARC sequence) | `activate` → `Commander.decide` (`rules/turnLoop.ts:1002`) | Heuristic bot, or nothing |
| Counteraction fire / reserve move | `planCounteraction` | A second whole-force LLM call |
| Assault: stay in melee or retreat | `meleeOptionsFor` | Commander option at activation |

These are the places to add live decisions.

## 2. What Jev is good at

Jev (`typesafe/jev-1.13`, or `~typesafe/jev-latest`) is a **decision model, not
a text generator**. On OpenRouter (`POST https://openrouter.ai/api/alpha/decisions`):

- **Input:** a `state` (string, object or array, up to 32k tokens) and a map of
  named `questions`.
- **Question types:**
  - `choice`: picks one of up to 255 described options. Returns the pick, a
    probability for every option, and a confidence.
  - `score`: places the state on a 2–10 level scale.
  - `noul`: yes/no. Returns P(yes).
- **Output:** always well-formed. It cannot hallucinate an option.
- **Latency:** about 70–500 ms. Questions in one request are answered in
  parallel.
- **Cost:** about $0.042 per million input tokens; output is free.

That maps closely onto this engine's design. The engine already generates the
legal options (`optionsFor`), the commander only chooses by id, and
`validateOrders` drops anything else. Jev enforces that contract by
construction: it can only return an option you listed.

What Jev is **not** good at: writing a plan, arithmetic, or reasoning over many
steps. So it should **not replace** the turn-level LLM. It should work
**underneath** it.

## 3. Target architecture: two tiers of command

```
            ┌──────────────────────────────────────────────┐
 per turn   │ Tier 2 — Operational commander (existing LLM)│  Claude / GPT via bgwsCommanderTurn
            │  plan, main effort, orders, ROE as *intent*  │  (unchanged prompt, slow, rich)
            └───────────────────────┬──────────────────────┘
                                    │ intent + orders + ROE
            ┌───────────────────────▼──────────────────────┐
 per event  │ Tier 1 — Tactical decider (Jev)              │  jev-1.13 via bgwsJevDecide
            │  shoot now? halt or press? which target?     │  (fast, cheap, typed)
            │  which element acts next? commit reserve?    │
            └───────────────────────┬──────────────────────┘
                                    │ option id / yes-no (+ probabilities)
            ┌───────────────────────▼──────────────────────┐
            │ Rules engine (deterministic, unchanged math) │  resolvers, dice, fog of war
            └──────────────────────────────────────────────┘
```

Principles, taken from the existing design:

1. **The engine offers the options and computes the numbers; Jev only judges.**
   Pre-compute P(hit), P(kill), range band, armour facing, cover and ammunition
   using the existing resolvers, and give Jev those numbers. Never ask it to do
   the maths.
2. **Build the state from the side's view, never from the true state.** Build
   it from `SideView` (`projectForSide`) so fog of war stays enforced in code
   (`lib/fogOfWar.ts`).
3. **Every call has a deterministic fallback.** On a timeout, a transport
   error, or a confidence below threshold, use today's rule (`willReact`,
   `onContact`, the heuristic). A failure should look like today's behaviour,
   not a crash.
4. **The model call is injected** (the same pattern as `ModelCall`). All prompt
   and state building stays pure and unit-tested.
5. **Every decision is logged with its probabilities**, so replays stay
   deterministic and nothing is re-queried.

## 4. Work breakdown

### Phase 0 — Spike (about 1 day)
- Get an OpenRouter key and hand-craft 10–20 decision states from real logged
  games, for example a reactive-fire moment. Call the Decisions API with `curl`
  or a script and check that answers are sensible and the latency is
  acceptable.
- Confirm the exact request and response schema. The endpoint is **alpha**, so
  pin it behind one adapter.

### Phase 1 — Transport inside Foundry
You can't call OpenRouter from the browser app: the API key would be exposed
and CSP and egress rules would block it. Mirror the `bgwsCommanderTurn` pattern
instead:

1. Create a Foundry **REST API Source** (Data Connection → External systems)
   for `openrouter.ai`, with the API key stored as a source secret and an
   egress policy approved for that host.
2. Write a TypeScript v2 (or Python) **Function** that imports the source:
   `bgwsJevDecide(request: string): string` (a JSON string in and out, so the
   schema can change without republishing). Add a hard timeout of about 1.5 s
   and return a typed error rather than throwing.
3. Publish it as an ontology **query** and regenerate the OSDK.
4. Add `data/jevClient.ts`, which exports `foundryJevCall: JevCall`. Reuse
   `describeApiError` and the permission-hint logic from `commanderClient.ts`.

```ts
// rules/jev.ts — pure types, no I/O
export type JevQuestion =
  | { type: "choice"; instructions: string; criteria: Record<string, string> }
  | { type: "noul";   instructions: string }
  | { type: "score";  instructions: string; levels: string[] };

export interface JevRequest  { state: unknown; questions: Record<string, JevQuestion> }
export interface JevAnswer   { type: string; choice?: string; noul?: number; score?: number;
                               probabilities?: Record<string, number>; confidence?: number }
export type JevCall = (req: JevRequest) => Promise<Record<string, JevAnswer>>;
```

### Phase 2 — Make the decision points pluggable (the main refactor)
Add a `TacticalDecider` to `PhaseConfig`. Its default implementation is exactly
today's behaviour, so every existing test passes unchanged.

```ts
export interface TacticalDecider {
  /** One call per action: which of these eligible reactors fire, best first. */
  reactions(ctx: ReactionContext): Promise<{ reactorIds: string[]; decision?: JevTrace }>;
  /** Called by the walk at the moment of contact, not before the move. */
  onContact(ctx: ContactContext): Promise<{ press: boolean; decision?: JevTrace }>;
  /** Which observer gets the sighting interrupt. */
  sightingObserver?(ctx: SightingContext): Promise<string | null>;
}
export const ruleDecider: TacticalDecider = /* wraps willReact / option.onContact */;
```

The needed changes:
- `runReactiveFire`: split it into *eligibility* (the existing filters: range,
  LOS, ammo, markers, weapon) and *choice* (`willReact` plus the slice). Pass
  the eligible set to `decider.reactions`. Make the function `async`. Its
  callers `activate`, `executePlannedTurn` and `runCounteractionRound` are
  already async. `resolveAssaultAction` has to become async too.
- `walkUntilContact`: return early with `{ contactAt, newlySighted }` when
  contact happens. `resolveAction` then asks `decider.onContact` and, on
  "press", resumes the walk. This turns the pre-committed `onContact` into a
  live decision. Keep `option.onContact` as the commander's stated default,
  which Jev sees as intent.
- **ROE stops being a switch and becomes intent plus a hard gate.** `never`
  stays a hard veto: concealment is the operational commander's call. The other
  three values go into Jev's instructions as the default it should deviate from
  only with reason.
- **Randomness:** keep Jev out of `config.rng`. If you sample from Jev's
  probabilities rather than taking the argmax, use a separate
  `decisionRng`, so the combat dice stream, and so `calibration.test.ts`, are
  unaffected.

### Phase 3 — Tactical state encoder (pure, tested)
Add `rules/jevState.ts`. It builds a compact JSON state (about 1–3k tokens) for
one decision:

```jsonc
{
  "intent": { "plan": "<Tier-2 plan>", "myOrder": "<option summary + why>", "roe": "withinShortRange" },
  "me":     { "id": "B-2", "type": "MBT", "cs": "3/4", "tq": 4, "morale": "steady",
              "markers": ["moved"], "ammo": {"AT": 3}, "terrain": "woods", "inCover": true },
  "event":  { "kind": "enemyCrossingArc", "enemy": "R-1 (full sighting, BMP)", "rangeM": 900,
              "aspect": "flank", "enemyMoving": true },
  "odds":   { "pHit": 0.42, "pKillOrSuppress": 0.31, "snapShotPenalty": -1 },
  "exposure": { "enemiesWithLosToMe": 2, "revealsPosition": true },
  "nearbyFriends": [...], "recentEvents": ["R-3 fired on B-1 last activation", ...]
}
```

Build it from existing pieces: `projectForSide`, `inCover`, `terrainClassForFlag`,
`lineOfSight`, `armourFacing` / `isFlankShot`, `smokeOnLine`, and `hasRounds`.
Compute the odds analytically from the `resolveDirectFire` tables, or by Monte
Carlo on a throwaway RNG. Snapshot-test the output so you can read exactly what
Jev was told, as `buildOrdersPrompt` already allows.

### Phase 4 — Questions and deciders
Add `rules/jevDecider.ts`, which implements `TacticalDecider` on top of a
`JevCall`:

- **Reaction:** one request per enemy action, with one `noul` question per
  eligible reactor ("engage this target now with a snap shot?"). Because the
  questions run in parallel, one round trip covers every reactor. Rank by
  P(yes), fire those above a threshold (for example 0.5), and cap at
  `maxReactorsPerAction`.
- **Contact:** a `choice` of `{halt, press}`, and later `withdraw` if the rules
  get a legal withdraw option.
- **Sighting observer:** a `choice` over the eligible observers.

Also add `jevCommander(): Commander`, a Jev implementation of the per-activation
`Commander.decide` used by the ARC sequence (`activate`):
- Options can exceed 255 (many move destinations). Prune to the top N using the
  heuristic's score, and keep all fire and assault options.
- The option `summary` becomes Jev's `criteria` text. Enrich move summaries
  with "ends in cover / exposed to N / closes to X m of Y" so Jev has something
  to judge.
- Set a **confidence gate**: below a threshold, fall back to the heuristic (or
  escalate to the Tier-2 LLM if you want a hybrid).

Commander selections to offer in the UI (`Play.tsx`):
`heuristic`, `LLM`, `LLM + Jev tactics` (the recommended default), and `Jev only`
(the ARC sequence with `jevCommander` and no Tier-2 LLM).

### Phase 5 — Logging, replay, UI
- `rules/events.ts`: extend `DecisionEvent.chosenBy` with `"jev"`, and add
  optional `probabilities`, `confidence`, `latencyMs` and `fallback`
  (`"timeout" | "lowConfidence" | "error"`). Log a decision event for every
  reaction and contact decision too, not only activations.
- `lib/replay.ts`: replays read decisions from the log and never call Jev.
- Add a cache keyed by `hash(state, questions)` so harness batches stay cheap
  and repeatable.
- UI: show Jev's call on the counter's decision line and timeline
  (`planBadges`, `stepVisuals`), for example "B-2 held fire (P=0.22) — exposed
  to 2". Also add a "why" panel that shows the exact state JSON.

### Phase 6 — Evaluation
Use the existing harness (`rules/harness.ts`, `rules/commanderTrial.ts`).

Compare:
- heuristic
- LLM with rule-based tactics (today)
- LLM with Jev tactics
- Jev only

Run each in both sequences of play and across several seeds and force lists.

Track:
- win rate
- CS lost per CS inflicted
- reactions that hit or were wasted
- ammunition spent
- fallback rate
- latency per turn
- cost per game

Also run `moduleImpact("reactionFire")` with Jev tactics, to check that
reactions now decide engagements rather than acting as ceremony.

## 5. Budgets

- **Cost:** a 2k-token state costs about $0.00008 per request. At roughly 200
  decisions per game that is about **$0.02 per game**, which is negligible
  beside the Tier-2 LLM.
- **Latency:** batch per event, so one round trip covers all reactors. A turn
  with about 30 tactical events adds roughly 3–10 s. Show a "thinking" tick on
  the board and run Jev calls concurrently with animation where possible.
- **Timeouts:** 1.5 s per call, then the rule fallback.

## 6. What "real time" means here, and an optional Phase 7

Phases 1–6 make decisions **at the moment they happen** inside the existing
alternating-activation turn. The rulebook structure is unchanged; the
pre-committed choices become live ones. This is the recommended scope.

If you want true continuous simulation (a wall-clock tick loop where units act
asynchronously), that is a much bigger engine change: `runTurn` would become a
fixed-step scheduler, and movement, sighting and fire would resolve per tick.
Jev's latency makes it viable. You would batch every unit's decision into one
request per side per tick (at 0.5–2 s per tick), and those questions run in
parallel. That is only worth doing after Phases 1–6 show that Jev's tactical
judgement beats the rules.

## 7. Risks

- **The Decisions API is alpha.** Keep one adapter (`jevClient.ts`) and pin the
  model version (`jev-1.13`, not `-latest`) for reproducible experiments.
- **Foundry egress approval** for `openrouter.ai` may need an admin.
- **The 255-option limit on choice:** prune move options (Phase 4).
- **Jev is stateless:** it has no memory between calls. Put intent and recent
  events into every state.
- **Calibration:** tune the Noul and confidence thresholds on the harness, not
  by eye.

## 8. Suggested order of PRs

1. `rules/jev.ts` types, plus a fake `JevCall`, plus `rules/jevState.ts` with
   snapshot tests. No behaviour change.
2. The `TacticalDecider` seam, with async `runReactiveFire` and a resumable
   contact walk, and `ruleDecider` as the default. The full test suite should
   stay green.
3. The Foundry source, function and query, plus `data/jevClient.ts`.
4. `jevDecider` (reactions and contact) with logging and replay support.
5. `jevCommander` for the ARC sequence, plus the UI selection.
6. The harness comparison report.

## 9. What has been built

| Piece | File |
|---|---|
| Decision seam (`TacticalDecider`, `ruleDecider`) | `rules/tactical.ts` |
| Engine hooks: `eligibleReactors` split out of `runReactiveFire`; async `reactiveFireLive`, `resolveAssaultActionLive`, `resolveMoveLive`; `GameConfig.tactical` | `rules/turnLoop.ts` |
| Orders sequence passes commander intent down and uses the live hooks | `rules/orders.ts` |
| Decisions API types, tolerant answer parser, deadline | `rules/jev.ts` |
| State encoder: exact fire odds (all 36 dice outcomes), fog-safe enemy, terrain, exposure, intent | `rules/jevState.ts` |
| Jev tactics: one `noul` per eligible reactor in one request; `choice` halt/press at contact | `rules/jevDecider.ts` |
| Jev as commander: `jevCommander` (activation) and `jevOrdersCommander` (one request per turn) | `rules/jevCommander.ts` |
| OpenRouter client, key from `import.meta.env.VITE_OPENROUTER_API_KEY`, answer cache | `data/jevClient.ts` |
| `DecisionEvent` gains `"jev"`, `probabilities`, `confidence`, `latencyMs`, `fallback`; `TurnRecord.decisions` | `rules/events.ts`, `lib/liveGame.ts` |
| Play screen: one **"Use Jev for decisions"** setup checkbox (both sides, fixed once the game starts); per-turn list of Jev's calls with cost; each call is a step in the turn playback | `Play.tsx` |
| Sighting interrupt: Jev picks which observer looks (`attemptSightingInterruptLive`) | `rules/turnLoop.ts`, `rules/jevDecider.ts` |
| Reserve follow-up (fire / assault / nothing at the end of a reserve move) asked of Jev instead of the heuristic (`chooseOptionLive`) | `rules/turnLoop.ts`, `rules/orders.ts` |
| Trial: challenger may be `heuristic`, plus "Use Jev for decisions" for the challenger's side; reports Jev calls, fallbacks, latency, cost | `rules/commanderTrial.ts`, `Trial.tsx` |
| Tests with a fake `JevCall` | `rules/jev.test.ts` |

`jevOrdersCommander` / `jevCommander` (Jev as the whole commander) remain in
the library for experiments but are not offered on the play screen: there,
the checkbox puts Jev in charge of every in-the-moment decision and the
selected commander keeps planning the turn.

Without a decider configured, the event logs of 72 seeded games (both
sequences of play, two rulesets, every force list) are byte-identical to
before the change.

The key goes directly from the browser to OpenRouter, so it is visible in the
bundle. Use a key with a spend limit, or swap `openRouterJevCall` for a
Foundry-function `JevCall` (Phase 1 above) later. Nothing else changes.


## 10. Second round: making the most of Jev

| # | Improvement | Where |
|---|---|---|
| — | Every Jev decision printed to the browser console: a one-line headline (who, what, probability, confidence, latency), expandable to the probabilities, the options and the exact state sent | `rules/jevConsole.ts` |
| 1 | **Commander plans, Jev acts.** In the orders sequence, the commander's orders fix which units are committed; Jev picks which of them acts next and may adapt the action to the current board. Falls back to the written order. | `rules/orders.ts` (`executePlannedTurn`), `chooseActivation` in `rules/jevDecider.ts` |
| 2 | **Terrain-aware moves:** into cover, overwatch (sees the most known enemies in range), pull back out of sight, onto a flank. Per side, only with Jev on. | `tacticalMoveOptions` in `rules/turnLoop.ts` |
| 3 | **Richer state:** objective distance and bearing, units able to act together, threat odds from each identified enemy, height above the nearest enemy, the last few exchanges (fog-safe: unsighted enemies are "unseen enemy") | `situationOf`, `recentEvents` in `rules/jevState.ts` |
| 4 | **Coordinated reactive fire:** one choice over "nobody", each unit alone and each pair (up to the cap) — a fire plan, not independent votes | `reactionAsk` in `rules/jevDecider.ts` |
| 5 | **Using the probabilities:** unsure answers escalate to the side's LLM (LLM commanders only); optional seeded sampling ("vary decisions"); Jev's danger/opportunity score per unit is put into the LLM's orders prompt | `rules/jevDecider.ts`, `rules/jevAssess.ts`, `rules/llmCommander.ts` |
| 7 | **Speed and memory:** reactions to the planned moves are prefetched in one request per side; answers persist across page loads; the latest decision is marked on each counter (own side only, dashed when the rules decided) | `prefetchReactionsFor`, `data/jevCache.ts`, `decisionMarksFor` in `lib/stepVisuals.ts`, `Play.tsx` |

Item 6 (tuning thresholds) is deliberately left until there are real logs.
The thresholds to tune are `minConfidence` (0.25) in `jevTacticalDecider`,
and the prefetch batch size (8).

With Jev off, event logs are still byte-identical to the original engine over
72 seeded games.
