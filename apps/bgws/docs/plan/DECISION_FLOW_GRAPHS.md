# Decision flow graphs: editable behaviour templates for every unit

Part of the [multi-domain plan](MULTI_DOMAIN_PLAN.md). This is the detailed specification for workstream **W1**.

## 1. What it is

Every unit carries a **decision flow graph (DFG)**. A DFG is a small graph that answers one question: *when something happens to this unit, what is it allowed to consider, and who decides?*

- **By default**, every unit gets a graph built from what it can actually do. A tank that spots an enemy may *fire*, *hold and observe*, *wait for a better shot* or *pull back*. A Javelin team may *fire*, *stay hidden* or *displace*. A ship that detects an inbound missile may *engage*, *decoy* or *turn to unmask more launchers*.
- **A player can author templates**: copies of a graph with things removed, restricted or re-weighted. "Tank: stand fast" is the default tank graph with *pull back* removed from every contact decision.
- **Templates are swappable**: on one unit, on every unit of a type, or across a side. You can swap before play or while paused.
- **Jev still makes the choice.** The graph only decides *which options exist* and *what is decided by rule rather than by Jev*. Jev picks among the options the graph leaves open, exactly as it does today.

The graph replaces three hard-coded places in today's engine:

| Today | File | Becomes |
|---|---|---|
| `decisionPointOf`: the event → decision point priority table | `realtime/engine/decisions.ts:72-97` | the graph's **trigger table** |
| `optionsAt`: one `switch(point)` that builds options | `realtime/engine/decisions.ts:287-730` | the graph's **choose nodes** and the **action library** |
| `ruleFallback`: the default when Jev cannot answer | `realtime/engine/decisions.ts:783-858` | each choose node's **fallback edge** |

The runner (`runner.ts`) stays as it is: coalescing, cooldowns, one decision in flight per unit, the reaction delay, the side batch to Jev and the deterministic wait.

## 2. Why a graph, and not a behaviour tree or a state machine

The professional tools take different approaches:

- **VBS4** (VBS Control) uses visual behaviour trees with Lua.
- **AFSIM** uses behaviour trees plus hierarchical tasking.
- **OneSAF** composes behaviours from primitives.
- **MASA SWORD** is driven by doctrine.
- **Command: Modern Operations** exposes doctrine settings: weapon release authority, EMCON and withdraw/redeploy thresholds.

The literature (Colledanchise & Ögren) shows behaviour trees generalise state machines, and that state machines become brittle as they grow.

The engine already has the split those tools converge on. A deterministic **autopilot** runs every tick. A **decider** runs only at events. So we do **not** need a behaviour tree that is ticked every second; the autopilot already plays that role.

What we need is a **decision-time graph**: evaluated once, when an event arrives, and ending in an order. That keeps the graph small and readable, and it maps directly onto Jev's `choice` / `noul` / `score` questions.

The design has three layers:

1. **Drills stay in the autopilot.** Taking cover when fired on, and returning fire at a located shooter, happen at once without asking anyone. Real crews do the same. Templates can **tune** drills through parameters (e.g. "do not return fire under EMCON"), but drills are not graph nodes, because they cannot wait for a decision.
2. **Rules of engagement and weapon release authority are hard gates outside Jev.** A template can tighten them but never loosen them past the orders. This is the "action masking" pattern: the model can only see options that are legal.
3. **Jev decides at choose nodes.** It sees the options left after gates, capabilities and constraints, and picks one.

## 3. The model

### 3.1 Data types

```ts
// realtime/behaviour/types.ts

/** A versioned, shareable behaviour template. */
export interface DecisionGraph {
  id: string;                       // "core.land.tank" | "user.tank-stand-fast"
  name: string;
  version: number;                  // bumped on every save
  parent?: { id: string; version?: number };  // inherit from; patches below apply on top
  appliesTo: ArchetypeSelector;     // which units it may be assigned to
  triggers: TriggerRule[];          // event → entry node, with priority
  nodes: Record<NodeId, GraphNode>;
  params: Record<string, number | string | boolean>; // tunables read by gates and drills
  doctrine?: string;                // plain words given to Jev with every decision from this graph
  patches?: GraphPatch[];           // only on derived templates
  meta: { author: string; createdAt: string; notes?: string; tags?: string[] };
}

export type NodeId = string;

export interface TriggerRule {
  on: TriggerKind | TriggerKind[];  // "sighted", "underFire", "missileWarning", "bingoFuel", ...
  when?: Condition;                 // e.g. located === true (today's D2 vs D3)
  priority: number;                 // replaces the fixed table in decisionPointOf
  entry: NodeId;
  severe?: boolean;                 // bypass coalescing, as today's severe events
}

export type GraphNode =
  | { kind: "gate";   test: Condition; then: NodeId; else: NodeId }        // rule, never Jev
  | { kind: "judge";  question: string; yes: NodeId; no: NodeId;            // Jev `noul`
      threshold?: number; fallback: "yes" | "no" }
  | { kind: "choose"; title: string; question: string;                     // Jev `choice`
      edges: Edge[]; fallback: EdgeId }
  | { kind: "act";    action: ActionRef; then?: NodeId }                   // emits an order
  | { kind: "call";   graph: string; entry?: NodeId }                      // sub-graph
  | { kind: "done" };                                                      // no change: keep the current order

export type EdgeId = string;
export interface Edge {
  id: EdgeId;                       // stable: "pullBack" — patches refer to it
  label: string;                    // shown in the editor
  to: NodeId | ActionRef;           // usually straight to an action
  when?: Condition;                 // the edge only exists when this holds
  requires?: CapabilityPredicate;   // the edge only exists if the unit can do it
  bias?: number;                    // a soft lean, -2..+2, told to Jev in words ("prefer", "avoid")
  note?: string;                    // doctrine for this option, given to Jev
}

/** An action from the library, with parameters. One action may expand to several concrete options. */
export interface ActionRef { verb: ActionVerb; params?: Record<string, unknown> }
```

**Conditions** are a small expression tree over a fixed **fact vocabulary**. They are not free code, so templates are safe to share and easy to validate.

```ts
export type Condition =
  | { all: Condition[] } | { any: Condition[] } | { not: Condition }
  | { fact: FactKey; op: "<" | "<=" | "==" | ">=" | ">" | "!="; value: number | string | boolean }
  | { param: string; op: ...; fact: FactKey };  // compare a fact to a template parameter

// FactKey examples (each with a type and a doc line, listed in the editor):
//   self.strength            0..1 of starting strength
//   self.cohesion            steady | shaken | broken
//   self.suppression         0..100
//   self.ammo.<weapon>       0..1 of basic load            (needs W3 sustainment)
//   self.fuel                0..1                          (W3)
//   self.seen                unobserved | possiblySeen | knownSeen
//   self.emitting            boolean                       (W6 EW)
//   self.linkQuality         0..1 to its controller/HQ     (W4 comms, W5 drones)
//   threat.belief            unaware | suspected | located | identified | lost
//   threat.range_m, threat.class, threat.canHitMe, threat.iCanHitIt (odds bands)
//   orders.task, orders.urgency, orders.onContact, orders.roe
//   friends.within_m(1500).count, friends.needHelp
//   terrain.cover, terrain.hullDownNear
//   time.sincePhaseStart_s, time.night, weather.visibility
```

### 3.2 The action library

An **action verb** is what a unit can be told to do. Each verb declares:

- **requires**: a capability predicate. `engage` needs a weapon that can reach the threat class; `callFire` needs a link to a fires net; `launchDrone` needs a drone in stores.
- **expand(state, unit, params)**: turns the verb into one or more concrete `RtOption`s. For example, `engage` gives one option per legal target, and `pullBack` gives the best covered spot behind. Today's builders (`hiddenSpot`, `betterShot`, `pullBackSpot`, `triggersFor`, `closePosition`, `hullDownSpot`) become the expanders of the land verbs. They move out of the `switch` into `realtime/behaviour/actions/land.ts`.
- **order(option)**: the `RtOrder` that results.
- **words**: how the option is described to Jev and in the editor.

The verbs extracted from today's `optionsAt` are the v1 land library:

| Verb | Today's option ids | Requires |
|---|---|---|
| `keep` | keep, resume | — |
| `engage` | engage, returnFire, fireFirst, fire, help | a weapon that reaches |
| `waitFor` | wait*n*, wait*n*:fire, letPass | a weapon |
| `betterPosition` | better, closer, shift, relocate, regain | can move |
| `hullDown` | hullDown | `targetClass === armoured_vehicle` (generalised to `hasTurret`) |
| `observe` | observe, watch, quiet | — |
| `search` | search | — |
| `pullBack` | pullBack, pullBackCovered, breakContact | can move |
| `fireAndMove` | fireAndMove, fireAndBack, covered | a weapon and can move |
| `assault` | assault | a direct-fire weapon, ≤1.5 km |
| `callFire` | callFire | a fires net (W3, W4) |
| `support` | support, join | can move |
| `comply` | comply, partly, finish | — |
| `nextPhase` | nextPhase | orders with a phase left |
| `hold` / `overwatch` | hold, overwatch | — |

Other workstreams add verbs. The engine refuses a template that uses a verb its build does not have.

| Domain | Verbs |
|---|---|
| Land | `dismount`, `mount`, `layMines`, `breach`, `digIn`, `smoke`, `fireMission`, `shootAndScoot` |
| Drones | `launch`, `recall`, `loiterAt`, `strike`, `handOff`, `switchToFibre` |
| Air | `commit`, `abort`, `rtb`, `evade`, `jettison`, `crank`, `notch`, `refuel` |
| Naval | `engageSalvo`, `decoy`, `turnToUnmask`, `goDeep`, `sprint`, `drift` |
| EW | `jam`, `goSilent`, `emit`, `relocateAfterTransmit` |
| Logistics | `requestResupply`, `withdrawToRefuel`, `evacuate`, `recover` |

### 3.3 How a decision is resolved

```
event(s) held by the runner for this unit
   │
   ▼
1. TRIGGER   highest-priority TriggerRule whose `on` matches and whose `when` holds
   │         (no rule matches → no decision, the autopilot carries on; same as today's info events)
   ▼
2. WALK      from the entry node:
   │           gate   → evaluate the condition, follow then/else           (no Jev)
   │           call   → descend into the sub-graph
   │           judge  → a Jev `noul` question                              (rare; see §3.5)
   │           act    → this is the answer; stop                            (no Jev)
   │           done   → keep the current order; stop
   │           choose → go to 3
   ▼
3. EXPAND    for each edge: drop it if `requires` fails or `when` fails;
   │         expand its action into concrete options;
   │         drop options that break the orders (ROE gate, boundaries, weapon release);
   │         if nothing is left → take the fallback edge;
   │         if exactly one is left → take it without asking Jev
   ▼
4. ASK       one Jev `choice` question; the options and their biases and notes,
   │         the graph's `doctrine`, the unit's picture (unchanged from today)
   ▼
5. APPLY     the chosen option's order, plus any `act` that follows it; recorded in history,
             with the path walked through the graph (for the inspector and the AAR)
```

Rules that keep it fast and predictable:

- **At most one Jev question per decision by default.** A walk that would reach a second choose node is a validation warning. The player can allow it per template with `params.allowChainedChoice`, at the price of one more round trip (70–500 ms).
- **The walk is pure** and replays deterministically from the same state and graph version.
- **Every choose node has a fallback edge.** It is used on Jev error, timeout, confidence below 0.25, or an empty option set. Today's `ruleFallback` becomes these fallback edges.
- **The graph version is stamped on every decision**, so a replay or AAR knows which rules applied.

### 3.4 What Jev is told (changes to `realtime/engine/jevDecider.ts`)

The real-time decider only. Turn mode has its own `rules/jevDecider.ts`, which this does not touch.

The `unitPicture` state is unchanged. The changes are in the instructions and criteria:

- **`instructions`**: the choose node's `title` and `question` replace `DECISION_POINTS[point]`. The template's `doctrine` text is appended, e.g. *"This crew holds ground at all costs. It never gives up a position it was told to hold."*
- **`criteria`**: each option's summary, plus its edge `note` and its `bias` in words:

  | Bias | Wording |
  |---|---|
  | +2 | strongly preferred by doctrine |
  | +1 | preferred |
  | −1 | discouraged |
  | −2 | only if nothing else works |

- **The key is the edge id plus the option index**, so traces show which edge Jev took.

**Bias is soft, removal is hard.** If a player wants a tank that *never* retreats, they remove the edge (§4, "stand fast"). If they want one that *rarely* retreats, they set its bias to −2. The editor explains the difference.

### 3.5 Judge nodes: Jev for yes/no judgements

Some branches are judgements, not rules. Examples: *is this position untenable?*, *is this contact the main effort or a feint?*, *is the convoy route still safe?*

A `judge` node asks Jev a `noul` question and branches on P(yes) ≥ `threshold` (default 0.5). Use it sparingly; most branches should be gates, which are free and predictable.

Like choose nodes, a judge node counts toward the one-question budget. Both kinds of question can go in the same Jev request, because questions in one request are answered in parallel. The walk can therefore pre-ask a judge and a choice together when the judge only selects between two choose nodes.

## 4. Templates, inheritance and swapping

### 4.1 Where templates come from

| Kind | Id prefix | Who makes it | Editable |
|---|---|---|---|
| **Core** | `core.` | Shipped in code (`realtime/behaviour/library/*.ts`). One per archetype: `core.land.tank`, `core.land.ifv`, `core.land.infantry`, `core.land.atgmTeam`, `core.land.mortar`, `core.drone.fpvTeam`, `core.air.fighter`, `core.sea.destroyer`, ... | No. Duplicate to edit. |
| **User** | `user.` | Made in the editor, stored in Foundry (§6) | Yes |
| **Scenario** | `scn.` | Saved inside a scenario file, so a shared scenario plays the same | Yes, within the scenario |

### 4.2 The default graph for a unit ("based on what it can do")

When a unit is placed:

1. **Archetype.** The unit's archetype comes from its data: `CuratedPlatform.subclass`, `moveType` and `targetClass`, and from W2 on, the platform's domain and role. For example, `armoured_vehicle + T + main gun` gives `tank`. The mapping is a table in `realtime/behaviour/archetypes.ts`, with a generic fallback per domain (`core.land.generic`).
2. **Core template.** The archetype selects its core template.
3. **Capability masking at run time.** Edges whose `requires` the unit fails simply never appear. A tank with no missile never sees `fireMissile`. An infantry section with no radio never sees `callFire`. This happens at decision time, not when the template is built, so one template serves every variant of a type, and a unit that *loses* a capability loses the option (out of ammunition, radio destroyed, drone lost).

So "by default each unit gets a decision flow graph based on what it can do" is: **the archetype picks the template, and the unit's live capabilities mask it.**

### 4.3 Deriving a template: patches, not copies

A derived template keeps a `parent` and a list of **patches**. When the core template improves, derived templates pick up the change unless they patched that exact thing. The editor shows a derived template as *"Default tank, with 3 changes"*, and can show the diff.

```ts
export type GraphPatch =
  | { op: "removeEdge";  node: NodeId; edge: EdgeId }
  | { op: "addEdge";     node: NodeId; edge: Edge }
  | { op: "setBias";     node: NodeId; edge: EdgeId; bias: number }
  | { op: "setNote";     node: NodeId; edge: EdgeId; note: string }
  | { op: "setFallback"; node: NodeId; edge: EdgeId }
  | { op: "setTest";     node: NodeId; test: Condition }      // gates
  | { op: "replaceNode"; node: NodeId; with: GraphNode }
  | { op: "addNode";     node: NodeId; value: GraphNode }
  | { op: "setTrigger";  index: number; rule: TriggerRule }
  | { op: "setParam";    key: string; value: number | string | boolean }
  | { op: "setDoctrine"; text: string };
```

**Worked example: "Tank: stand fast".**

```jsonc
{
  "id": "user.tank-stand-fast",
  "name": "Tank: stand fast",
  "parent": { "id": "core.land.tank" },
  "appliesTo": { "archetype": "tank" },
  "doctrine": "This crew does not give up ground. It fights from where it is or moves forward.",
  "patches": [
    { "op": "removeEdge", "node": "contact.choose",   "edge": "pullBack" },
    { "op": "removeEdge", "node": "underFire.choose", "edge": "pullBack" },
    { "op": "removeEdge", "node": "underFire.choose", "edge": "pullBackCovered" },
    { "op": "removeEdge", "node": "cue.choose",       "edge": "pullBack" },
    { "op": "setFallback", "node": "underFire.choose", "edge": "returnFire" },
    { "op": "setParam", "key": "breakLossThreshold", "value": 0.5 }
  ],
  "meta": { "author": "dk", "createdAt": "2026-10-18" }
}
```

The last patch raises the break-test threshold, and shows a deliberate boundary. A template can tune **drill parameters** the autopilot reads, within limits the engine sets. It cannot make a unit unbreakable: break tests still run, and a stand-fast unit can still be broken by losses.

### 4.4 Assigning templates

The engine resolves which template a unit uses at each decision, so a swap takes effect at the unit's next decision. Most specific wins:

1. **Unit override**: set on the unit in setup or the inspector.
2. **Side assignment by archetype**: "all Red tanks use *Tank: stand fast*".
3. **Scenario default** by archetype.
4. **Core template** for the archetype.

The resolution is recorded on each decision. Swapping while paused is allowed. Swapping while running is allowed at ×1 to ×5 and queues to the next decision.

**Validation on assignment.** `appliesTo` must match the unit's archetype or one of its parents (e.g. a `tracked-combat` template fits tank and IFV). A template that references verbs the unit's archetype can never perform gets a warning, not an error, because masking makes it harmless.

## 5. The editor (UI)

A new screen at `/bgws/behaviour`, linked from Home and from the real-time setup panel.

### 5.1 Two views of the same template

1. **Simple view (the default): a table of decisions.** One card per trigger: *"Spots an enemy"*, *"Under fire, can see the shooter"*, *"Bingo fuel"*. Each card lists its options with three controls each:

   | Control | Values |
   |---|---|
   | Toggle | allowed / removed |
   | Lean | strongly avoid … strongly prefer |
   | Note | a doctrine note for this option |

   Each card also has a fallback picker. This covers most uses ("my tanks don't retreat") without the user seeing a graph.
2. **Graph view: a node canvas.** Built on React Flow (`@xyflow/react`, a pinned version). Triggers sit on the left, gates as diamonds, choose nodes as rounded boxes with their edges as ports, and actions as leaves. You can drag in gates from a palette, wire edges, and drill into `call` sub-graphs. Every change is stored as a patch against the parent, and the canvas renders the merged result.

### 5.2 Panels

- **Library**: core, user and scenario templates, filtered by domain and archetype, with "duplicate", "derive", "export JSON" and "import JSON".
- **Inspector**: properties of the selected node or edge. Conditions are built from dropdowns over the fact vocabulary, never typed as code.
- **Validation**, live, with each problem linked to its node:
  - every trigger reaches an `act` or `done`
  - every choose node has a fallback that is always available (no `requires`, no `when`)
  - no cycles without an `act`
  - walk depth ≤ 16
  - at most one choose node per walk, unless allowed
  - verbs exist in this build
  - parameters are within engine limits
- **Diff**: the patches against the parent, in words.
- **Test bench**, the most important panel. Pick a calibration scene from `situations.ts` (later from a recorded game moment). See the path the walk takes, the options left and why the others were dropped, and what Jev chose with the probability of each option. Compare two templates side by side on the same scene.
- **Batch compare** (from W12): run a scenario N times headless with template A and N times with template B. Report outcomes: losses, time to objective, ammunition used, how often each option was taken.

### 5.3 In game

- **Unit inspector** (exists, under the event log): a "Behaviour" row shows the resolved template and why ("side assignment: Red tanks"), with a "swap…" menu.
- **Decision log**: each decision line can be expanded to show the walked path through the graph, the options dropped by gates and masks, and Jev's distribution. This is how a player learns why a unit did something.

## 6. Storage

- **In the repository, as JSON files**: one file per template, in a folder per kind:

  ```
  realtime/behaviour/
    templates/
      core/     default.json  tank.json  ifv.json  infantry.json  atgm-team.json  mortar.json  recce.json  hq.json
      custom/   tank-stand-fast.json      ← { "parent": "core/tank", "remove": ["pull-back"] }
    schema.ts   validates every template when it is loaded
    walk.ts     the engine walks the graph; Jev is asked only at choose nodes
    actions/    land.ts … the verbs a template can name ("fire", "hull-down", "pull-back")
  ```

  Loaded with Vite's `import.meta.glob("./templates/**/*.json", { eager: true })`, so no package is needed. A test loads every file through `schema.ts`, so a malformed template fails CI rather than a game. Because JSON holds no code, conditions are data over a closed set of facts (`{ "fact": "ammo.main", "op": ">", "value": 0 }`) and actions are referenced by name. That is the same rule as §3.3: templates contain no code.
- **The graph is not sent to Jev.** The engine walks it, applies every hard gate (ammunition, range, ROE) against the real state, and sends Jev one `choice` question with only the legal options as `criteria`, as `jevDecider.ts` does today. The template's words (node title, doctrine, each option's bias) go into `instructions` and `criteria`, so the template shapes Jev's judgement without Jev having to execute it.
- **Golden test first.** `realtime/engine/decisions.golden.test.ts` records today's decisions (every named situation and 20 seeded games) in `__golden__/`. The JSON graphs must reproduce those snapshots exactly before the old switch statements are deleted (§7 steps 3–4).
- **In Foundry, later**: a `BgwsBehaviourTemplate` object type. It has id, name, version, parentId, parentVersion, archetype, JSON body and author, with create/edit actions through OSDK. This needs `api:use-ontologies-write`, already requested in `client.ts`. Templates are shared across a team.
- **Locally**: unsaved drafts go to `localStorage` as a convenience only, and JSON import and export are supported.
- **Schema version** on every template. A migration function per bump lives in `realtime/behaviour/migrate.ts`.

## 7. Building it

| Step | What | Done when |
|---|---|---|
| 1 | `realtime/behaviour/types.ts`, `conditions.ts` (evaluator plus fact registry), `walk.ts` (pure walker) | Unit tests for every node kind |
| 2 | Extract today's option builders into land verbs (`actions/land.ts`); `optionsAt` becomes a thin call into them | All existing `realtime.test.ts`, `command.test.ts`, `situations.test.ts` pass unchanged |
| 3 | Write `core.land.default` as a graph (`templates/core/default.json`) that reproduces `decisionPointOf` + `optionsAt` + `ruleFallback` exactly | **Golden test**: for every scene in `situations.ts` and 20 seeded full games, the option lists and fallbacks are identical to the old path |
| 4 | Switch the runner to `walk()` and delete the `switch` statements | Golden test still green; trace output contains the walked path |
| 5 | Split `core.land.default` into archetype templates, one JSON file each in `templates/core/` (tank, ifv, infantry, atgmTeam, mortar, recce, hq) with the archetype table | Each archetype's option list is reviewed against doctrine (research reference R4, Land) |
| 6 | Patches, inheritance and assignment resolution | Tests: patch application, parent upgrade, precedence order |
| 7 | Simple-view editor plus library plus validation, local storage only | A user can make "Tank: stand fast" and assign it in setup in under two minutes |
| 8 | Test bench on `situations.ts` | Side-by-side compare of two templates on one scene |
| 9 | Foundry storage and sharing | Templates round-trip through OSDK |
| 10 | Graph view (React Flow) | Every patch op is reachable from the canvas |
| 11 | In-game inspector row plus decision log path view | — |

Each later workstream adds its verbs, facts, triggers and core templates, and a golden scene in `situations.ts` for each new decision.

### Status (October 2026), against milestone M1

| Step | State |
|---|---|
| 1–4 | **Done.** `types.ts`, `walk.ts`, `actions/land.ts` (53 named verbs) and `templates/core/default.json`. The runner walks the graph, and the golden test is unchanged. |
| 5 | **Partly.** `core/armour` and `core/infantry` exist (doctrine only, parent `core/default`). IFV, ATGM team, mortar, recce and HQ do not yet; they need those unit types in the catalogue (W14, M2). |
| 6 | **Done.** Patches and inheritance (`parent`, `remove`, `bias`, `when`, `doctrine`). Assignment, most specific first (`store.ts` `resolveBehaviour`): a scenario file's own per-unit choice; a **side assignment by kind** ("Red armour uses *Stand fast*", set from the template's page in the Behaviour Manager: armour, infantry, soft-skinned vehicles); the unit type's assignment (Asset Manager); the kind's built-in default. The setup review says where units' behaviours come from. Assignment is by target class until the archetype table of step 5 exists. |
| 7 | **Done:** the Behaviour Manager (`/bgws/assets/behaviours`), which covers the library, editing, validation and assignment. |
| 8 | **Done:** the **Test bench** tab in the Behaviour Manager (`BehaviourBench.tsx`, `behaviour/bench.ts`). Pick a scene from `situations.ts` and one or two templates. Each column shows: the template's lineage and doctrine; the path walked; every option, either offered or why not (switched off by the template, condition not met, or nothing of that kind to offer here); and what the rules would choose, checked against the scene's hand-judged sensible options. `bench.test.ts` proves the bench shows exactly what the game offers and chooses in every scene. Jev is not asked on the bench. |
| 9–11 | Not started. |

## 8. Risks

| Risk | Mitigation |
|---|---|
| Templates that are too permissive let Jev do unrealistic things | Gates and masks are hard; Jev cannot see a removed option. Core templates are reviewed against doctrine. |
| Templates that are too restrictive deadlock (no option left) | A mandatory, always-available fallback edge per choose node, enforced by validation |
| Editors write free code that breaks determinism or security | No code in templates: conditions are a closed expression tree over a typed fact vocabulary |
| Graphs sprawl and become unreadable | The simple view is the default; sub-graphs (`call`); a depth limit; the one-question rule |
| A core template change silently changes a user's template | Patches name stable edge ids; a parent version can be pinned; the editor flags "parent changed since you derived" |
| Latency grows with chained questions | One question per decision by default; judge and choice batched in one request |
