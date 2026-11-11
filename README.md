# BGWS: Battlegroup Wargame System

A local, physics-first, multi-domain wargame simulation in Rust, with Jev
and Claude as decision makers. The simulation is the project. It runs
headless from the `bgws` CLI, or hosted by `bgws-server`, which speaks a
documented protocol (WebSocket + HTTP, `protocol/PROTOCOL.md`) to any
frontend. The React viewer in `frontend/viewer` is the default frontend. It
can be swapped for another: `bgws-server <scenario> --web <dir>` serves any
folder in its place, and any client that speaks the protocol can drive a
game. The design is in `docs/design/RUST_ENGINE.md`.

## Layout

```
crates/      the simulation: core types, catalogue, environment, propagation,
             knowledge, behaviour, models, domains, the ECS sim, deciders,
             analysis, geodata, and the host and CLI (bgws-run)
apps/
  server/    bgws-server: hosts a game and speaks the protocol
  desktop/   bgws-desktop: the same host in a native (Tauri) window
data/        the platform catalogue, scenarios, behaviour templates and
             the vignettes' regression baselines
protocol/    the interface for frontends: PROTOCOL.md, TypeScript types
             generated from the Rust (types/), a typed client (client/)
frontend/
  viewer/    the default frontend (React, MapLibre, deck.gl); its build
             (dist/) is committed and embedded in bgws-server
  editor/    the scenario editor, served at /editor
  behaviour/ the Behaviour Manager, served at /behaviour
  classic/   the earlier single-file viewer, at /classic
docs/        the engine's design (design/), the research sources behind its
             figures (research/), the rulebook
packs/       theatre packs you build (git-ignored)
```

```sh
cargo build --release
target/release/bgws scenarios                        # the shipped scenarios
target/release/bgws play skirmish-advance-to-contact-v1 --record game.json --events events.jsonl
target/release/bgws replay game.json                 # exact replay, no model calls
target/release/bgws branch game.json --at 30 --deciders jev   # "what if": replay to minute 30, play on
target/release/bgws aar events.jsonl                 # after-action review
target/release/bgws why events.jsonl "Blue 1 Troop"  # what happened to a unit, with the figures
cargo build --release -p bgws-run --features duckdb  # event logs as DuckDB databases (builds DuckDB, ~25 min)
target/release/bgws play m4-drone-line --events game.duckdb
target/release/bgws events game.duckdb --sql "SELECT kind, count(*) FROM events WHERE side = 'blue' GROUP BY kind"
target/release/bgws terrain packs/kaliningrad --bbox 20.35,54.65,20.60,54.76   # real ground for a box (see "Real ground")
target/release/bgws pack packs/baltic --name baltic --lat 54.2 --lon 18.6   # a theatre pack of generated ground
target/release/bgws batch skirmish-symmetric-control-v1 -n 200 --flat
target/release/bgws vignettes --store data/regression   # vignettes against their bands and baselines
target/release/bgws sensitivity shahed-raid --html sens.html   # Morris + Sobol, the research backlog
target/release/bgws vignettes --html workbench.html  # the workbench page: each vignette against band and baseline
target/release/bgws behaviour --html behaviour.html   # the Behaviour Manager: every template's decision graphs (/behaviour on the server also makes and edits variants)
target/release/bgws compare breach-time --set engineers.breach.rate_mph.line_charge=1000 --html cmp.html   # A/B a figure change
target/release/bgws figures --markdown               # every figure, sourced or [default]
target/release/bgws platform generic/mbt             # a resolved catalogue record
target/release/bgws-server m4-drone-line --person blue --record game.json   # then open http://127.0.0.1:8765/ (the viewer; /editor, /behaviour)
cargo build --release -p bgws-desktop                # the desktop shell (needs libwebkit2gtk-4.1-dev)
target/release/bgws-desktop m4-drone-line --deciders rules   # the same game and pages in a native window
```

Deciders: `--deciders rules|jev|claude|local` (or as the scenario says).
Keys come from the environment only: `OPENROUTER_API_KEY` for Jev,
`ANTHROPIC_API_KEY` for Claude. A local model is any OpenAI-compatible server
(`--local-url`, `--local-model`). With no key or no network the template
fallbacks decide and the log says why.

### What a unit decider is told

Each land unit's question carries its state as named fields
(`Question::state`), built from the unit's own beliefs only:

- **unit:** name, type, vehicles fit, strength, posture, suppression,
  morale, when it last fired;
- **weapons:** each mount's munition, maximum (and minimum) range, rounds
  and volleys left, and the farthest it can reach now (`weaponReachM`);
- **enemiesSeen** (nearest six): what it is (only as well as it has been
  recognised), range, compass bearing, moving, how long ago it was seen,
  whether it is within our reach, and for an identified enemy how far its
  weapons reach and whether it can reach us;
- **firedOnByUnseen:** bearings of fire from shooters it has not found;
- **orders:** purpose, what it is doing, phase, urgency, ROE in words, and
  the distance and bearing to where its move leads;
- **ground:** cover, concealment, land cover, on a road, how open to view
  and whether dead ground (on a pack with a terrain analysis), the town
  block it is in, key terrain within 1.5 km;
- **friendsWithin3km** (nearest four): name, range, bearing, strength.

Jev gets it as the request's `state` (under `units.<name>`), as its
Decisions API recommends, and the instructions point there instead of
repeating it in words (`JevConfig::prose` sends both, to compare).
Options are sent under their own ids (`engageNow`, `keep`) rather than
`o0`, `o1`. Claude and local models get the same fields after the words.
A different state is a different cache key, so a remembered answer is only
reused for the same situation.

### The decision lab

How good are the deciders' choices? The lab (`crates/bgws-run/src/lab.rs`,
the page at `/lab`) measures it.

- **Moments:** every land unit's decision in a game (a question with more
  than one option), kept with the game's record and the unit's state as it
  was put. `bgws lab collect <scenario> --seeds 1-8` plays games and keeps
  them; `bgws lab import game.json` takes a saved game; **Add to lab** in
  the viewer keeps the game being played.
- **Play-outs:** `bgws lab playout` replays the game to a moment, forces
  each option in turn, and plays on (20 minutes by default) with the rules
  deciding and fresh dice for each run (12 by default). Each run is scored
  from the deciding unit's side: enemy share lost − own side share lost −
  ½ × its own unit's share lost + ¼ × share of the way to its objective
  made good, ±1 if the game is won or lost in that time. Options within one
  standard error of the best are near-best. A moment is decisive when some
  option is clearly worse; only decisive moments score a decider. Forcing
  the answer that was given, with the original dice, replays the game
  exactly (a test checks it). On a skirmish a moment takes 5–15 s.
- **Your call:** on the page, mark the best option, the acceptable ones, a
  flag and a note, beside the unit's picture (a radar of what it has seen,
  its friends, its objective and its weapons' reach), its facts and the
  play-out scores.
- **Grading:** `bgws lab grade --deciders jev` asks a decider every
  moment's question afresh, as it was put, and keeps its choice,
  probabilities and confidence under its name (the model; `+prose` with
  `--prose`), so settings can be compared.
- **Report:** `bgws lab report` (and the page's Report tab) gives each
  decider's near-best share, best share and mean regret against the
  play-outs, its agreement with your labels, and for Jev its mean
  confidence and calibration error (how far its confidence is from how
  often it is near-best), overall and per decision point. The template
  fallback, the rules' choice and what the game chose are yardsticks.

**Decision cases** test a decider faster and more clearly than games do.
A case is a hand-written situation (the unit's state as the game sends it)
for a template and decision point, with the options expected to be
acceptable, the ones that are wrong, and why. Its question is built as the
game builds one: the template's options after its gates (ROE, weapons,
cohesion), with its doctrine and leanings. A case can have variations
(the same case with a fact changed, such as an enemy at 4.5, 3.2, 2.4 and
1.2 km), each with its own expectations, and a trend: as the variations go
on, the share the decider puts on some options should not fall. Every case
is also asked with its options reversed and its unit renamed, and the
choice should not change.

- `bgws lab test --deciders jev` asks every case (one question per request,
  eight at a time) and reports pass, wrong and neither per decision point,
  invariance, trends, and Jev's confidence when right and when wrong. It
  warns when answers were the fallback (no key, an error). `--prose`,
  `--model`, `--case`, `--point` narrow or vary it; `--check` only builds
  the cases and scores the template fallback. Runs are kept in
  `<lab>/tests/`, and the page's **Cases** tab compares them case by case.
- The starter set is in `data/cases/`: 16 cases across all 13 decision
  points (tanks, infantry, an ATGM ambush, a recce patrol), built on the
  starting states in `bases.json` with `set` changes (dotted paths). After
  each change, weapon reach and each enemy's `inOurReach` and `canReachUs`
  are worked out again from the ranges. The expectations are a reading of
  standard small-unit doctrine: review and correct them. Two more
  (`from-suite-armour-duel.json`) came from the engagement suite's play-outs
  (below). Over all 18, the template fallback (the doctrine default) passes
  14 of 25 questions (56%) and is wrong on 9: a decider worth its cost
  should beat that. Jev 1.13 passed 18 of the first 23 (78%); its 3 misses
  were answers below the 0.25 confidence cut-off, where its own pick
  (search, shift fire, help the friend) was acceptable.
- **Make it a case** on a lab moment (or `bgws lab case <moment>`) keeps it
  as a case in `<lab>/cases/`: your best and acceptable options (else the
  play-outs' near-best) pass, and options the play-outs found clearly worse
  fail.

**The engagement suite** measures what a decider changes in played games.
Each scenario in `data/suite/suite.json` is played over several seeds with
the decider answering one side's units and the rules the other's, paired
with a rules-against-rules game on the same seed. The two start identically
and differ only by the decider's choices, so the difference is what it adds
(with the rules under test, every difference is exactly zero; a test checks
it). Measured for the side it played: result (+1 a win, −1 a loss),
exchange (enemy share lost − own share lost), own and enemy losses, and
survival (the share of the game its vehicles lived), each as a mean
difference with its standard error, per scenario and overall.
`bgws lab suite --deciders jev --seeds 1-4` (or the page's **Suite** tab);
`--keep-worst N` keeps the decider's N worst games in the lab, its side's
decisions as moments to play out. Two scenarios are held out
(`heldOut`), kept back for checking a tuned setting.

First results (Jev 1.13, against the doctrine default): on the cases, 78%
pass against the fallback's 52%, with 32 of 32 consistency checks held, but
in the suite's 40 paired games Jev did worse: result −0.20 ± 0.12, exchange
−0.06 ± 0.05, better in 6 pairs and worse in 18. Its worst game, an armour
duel, shows why: on first contact with an enemy tank at 1.7 km, both in
reach, it chose to move to a better position. Played out, that loses the
troop (−1.1), while carrying on, engaging, waiting or watching all win
(+1.5). In this simulation, moving mid-duel gives the enemy the first
shots. That moment is now a case. Near-best ignores score gaps under 0.05
(`lab::MIN_GAP`), however small the noise.

**Tuning Jev's wording.** Everything Jev is told besides the situation
itself is in `data/jev/wording.json` (`bgws lab wording` prints it; the
defaults are the text the engine always sent). It holds the instruction
templates (with `{placeholders}`), the rule of thumb, the notes, how
doctrine leanings read, Jev's own description of any option
(`optionWords`, by action, in place of its short label) and the confidence
cut-offs (overall and per decision point). `bgws lab tune` improves it:

1. A larger model (Claude Opus 5.5 through OpenRouter, `--proposer`) is
   shown the current wording, the training cases Jev gets wrong (the
   state, the options as worded, Jev's probabilities, the answers expected
   and why), the suite's results and earlier rounds, and proposes a few
   rewrites (`--candidates`, 4).
2. Each rewrite is checked first: every placeholder kept, no text too
   long, option descriptions only for real actions, and nothing naming a
   case (case ids, unit, platform and enemy names from the cases) and no
   number of three or more digits, so it can only help through general
   wording. The proposer may not touch the cut-offs.
3. The ones left are asked the training cases and must keep every
   consistency check and trend the current wording keeps, and beat it by
   more than 0.02 (passes less wrongs, as a share).
4. The best is played on the suite (the scenarios not held out) and taken
   only if it is not worse in games beyond one standard error.
5. After the rounds (`--rounds`, 3), the confidence cut-off is chosen from
   the training answers (Jev's raw picks are kept, so no re-asking).
6. Last, the original and the tuned wording are compared on what tuning
   never saw: about 30% of the cases (fixed by a hash of their ids) and the
   suite's held-out scenarios.

First runs (Jev 1.13, proposer Claude Opus 5.5, $0.70–0.95 each): what
helped was almost all **option descriptions**. Jev had only seen short
labels ("Better position"); the rewrite says what each option costs and
when it fits ("Drive to another firing position before engaging. No fire
while moving. Best before the fight starts, when it cannot fire from here,
or when an enemy out-ranges it in the open."). Most other ideas (reading
checklists, new balance rules) were refused by the consistency checks:
they changed answers when options were reordered. With 13 training and 5
held-out cases:

| | training cases | training scenarios | held-out cases | held-out scenarios |
|---|---|---|---|---|
| the engine's wording | 17 pass, 3 wrong of 20 | −0.33 ± 0.13 | 2 pass, 1 wrong of 5 | +0.02 ± 0.19 |
| tuned | 20 pass, 0 wrong | −0.17 ± 0.12 | 3 pass, 0 wrong | +0.29 ± 0.25 |

(Scenario scores are result Δ + exchange Δ against the doctrine default.)
An earlier run held out only 2 cases, which is too few: the split now
holds out a fixed 30%. The confidence cut-off stayed at 0.25 because the
tuned wording left Jev nothing below it to rescue.

**In use now.** Two runs' wordings were compared with the engine's own over
168 paired games each (all 7 suite scenarios, held-out ones included, both
sides): seeds 1–6, then fresh seeds 7–12 that no tuning used.
`BGWS_JEV_WORDING=<file>` points the game and the lab at a wording file
without installing it, so these comparisons need no edits.

| wording | cases (18) | seeds 1–6: result Δ / exchange Δ | seeds 7–12 (fresh) | all 168 |
|---|---|---|---|---|
| the engine's | 19 pass, 4 wrong | −0.14 ± 0.08 / −0.06 ± 0.03 | −0.02 ± 0.08 / −0.06 ± 0.03 | −0.08 / −0.06 |
| run 1 (`tune-1794253290`) | 25 pass, 0 wrong | +0.14 ± 0.10 / +0.05 ± 0.03 | −0.02 ± 0.09 / −0.02 ± 0.03 | +0.06 / +0.02 |
| run 2 (`tune-1794254133`) | 22 pass, 1 wrong | −0.05 ± 0.10 / −0.04 ± 0.03 | – | – |

Run 1's wording is the one in `data/jev/wording.json`. Part of its gain on
seeds 1–4 was fitting those seeds (they are what it was tuned on), but on
fresh seeds it still trades better than the engine's wording (exchange
−0.02 against −0.06, and 26 pairs better and 28 worse against 20 and 32),
with the same results. It is worse in one place: the mechanised hasty
attack (M2), where it loses more of its own (exchange −0.09 and −0.13).
That is the place to look next, with `--keep-worst`. On the cases its
consistency is a little lower (32/36 against 35/36): near-tied options
sometimes flip when reordered.

A run is kept in `<lab>/tune/<id>/` (`report.md`, `run.json`,
`wording.json`), and the lab's **Tune** tab shows it. Nothing changes the
game until it is accepted: `bgws lab tune --accept <id>` (or the tab's
button) writes `data/jev/wording.json` and prints the diff; commit the file
to keep it. A run costs well under a dollar: Jev's questions are a
fraction of a cent each, and the proposer is a few cents a round.

The lab lives in `$BGWS_LAB`, else `./lab` (gitignored): `games/` and
`moments/`, one JSON file each. A first look at three rules games of the
advance-to-contact skirmish: 6 of 12 played-out moments were decisive, and
the doctrine default was near-best in 5 of them. Where it was not, a tank
troop on first contact that carried on lost the whole troop in every run,
while one that took a better position kept a third of it.

Grading Jev needs `OPENROUTER_API_KEY` where the lab runs, and the network
to reach openrouter.ai.

## The workspace (plan §3.1)

| Crate | What it holds |
|---|---|
| `bgws-core` | ids, µs time, units, WGS84/ECEF/ENU, the figures registry, keyed ChaCha8 streams, the event log and fingerprint, question and answer types |
| `bgws-catalogue` | 105 platforms and 48 munitions built from parts (movers, bodies, signatures, sensors, mounts, radios, emitters, stores), class defaults, derivation, validation |
| `bgws-geodata` | real ground from planet PMTiles: a PMTiles reader over HTTP ranges with a disk cache, vector tiles, terrarium elevation, land cover, the road graph builder, and pack writing |
| `bgws-env` | terrain grids and pyramids, synthetic theatres, land cover and micro-terrain, soil and trafficability, roads, the change layer, tiles and theatre packs, sea state and sound speed, weather fields, optics, sun and moon, thermal days |
| `bgws-prop` | optical LOS with canopy, horizons, Deygout diffraction, ITM-style loss, HF ground wave and NVIS, satellite and microwave links, acoustic TL, link budgets |
| `bgws-gpu` | LOS, viewsheds, candidate pairs, path-loss and J/S fields, flow fields, Monte Carlo hits: CPU twins always, WGSL kernels behind `--features gpu` |
| `bgws-know` | Kalman tracks, GNN association, Bayesian classification with decoys, covariance-intersection COPs, DF fixes, `SideView` with truth kept behind an audit API |
| `bgws-behaviour` | the behaviour template walk (22 golden scenes from the original walker), hard gates, validation, patches, safeguards |
| `bgws-scenario` | scope, the v3 loader, scenario v4, the v3 → v4 converter, game records; 14 presets in `data/scenarios/` |
| `bgws-models` | land physics and people: ACQUIRE/TTP, thermal, radar, ESM, sonar, signatures, direct and indirect fire, engineers, suppression, morale, fatigue, casualties |
| `bgws-domains` | munitions in flight (kinematic profiles, CEP), air (3-DoF, sorties), air defence (kill chains, WTA, magazines), naval (hulls, salvo, damage, mines), space (SGP4, passes, imagery, GNSS) |
| `bgws-ops` | comms nets, drones and counter-drone, EW, PNT, logistics (min-cost flow), maintenance, medical, C2 and orders, cyber, aggregated combat |
| `bgws-sim` | the ECS world, the multi-rate clock and event queue, the land loop, decisions, views, and one module per joint domain (`domains/`) |
| `bgws-decide` | the decider and planner traits; rules, Jev (OpenRouter), Claude (Messages API, tool schemas), local (OpenAI-compatible), recorded answers, a router, a person's seat |
| `bgws-analysis` | batches, vignettes with bands, KS and χ² tests, the regression store, Morris and Sobol sensitivity, the research backlog, meta-model fitting |
| `bgws-run` | the host loop, the `bgws` CLI, the vignette experiments |
| `bgws-interop` | interoperability with other simulations: two-way DIS v7 (Entity State with dead reckoning, Fire, Detonation, simulation management); HLA 4 through the Federate Protocol with the RPR FOM, time management and a mock RTI; the EGM96 geoid, WGS-84 placement and DIS orientation (`docs/design/INTEROP_PLAN.md`) |
| `apps/server` | `bgws-server` (a library and a binary): the game on its own thread, speaking the protocol over HTTP and a WebSocket; bundles the default viewer (feature `bundled-viewer`), or serves any frontend folder (`--web`) |
| `apps/desktop` | `bgws-desktop`: the Tauri 2 shell, the same host in-process and its pages in a native window |
| `protocol/` | `PROTOCOL.md` (every message and route), TypeScript types generated from Rust (`ts-rs`) and a typed client |
| `frontend/viewer` | the default frontend (React, MapLibre, deck.gl, milsymbol): its build in `dist/` is committed and embedded in `bgws-server` |
| `frontend/editor`, `frontend/behaviour`, `frontend/classic` | the scenario editor, the Behaviour Manager and the earlier single-file viewer, served by `bgws-server` |

## Rules the code keeps (plan §2)

- Deciders never read truth: they get a `SideView` or a question's words.
- Every number is a declared figure with unit, range, source and confidence
  (`bgws figures`); tests fail on undeclared or unused figures.
- The core is sans-IO: `Sim::advance` stops at decision points and takes
  answers; the host does the network.
- Deterministic for a seed, build and machine: keyed streams, ordered
  iteration, simultaneous fire within a tick; tests compare fingerprints across
  seeds and across 1 and 4 threads. Game records replay exactly.

## Where it stands against the roadmap (§18)

| Phase | State |
|---|---|
| 0 Spike | Workspace, core, ECS, terrain, CPU/GPU LOS twins, a brigade bench and Tracy spans (`--features profile`). The desktop shell: `bgws-desktop` (Tauri 2), and `bgws-server` for a browser. |
| 1 Land tactical | Done: movement on trafficability with cost-raster routes, EO/IR sensing, tracks, direct fire, people, templates, rules decider, v3 converter, CLI. MBT Ph and attack-halt vignettes in band. |
| 2 Deciders | Done: Jev, Claude, local and recorded deciders, batching, caching, timeouts, fallbacks, cost logging, exact replay. |
| 3 Fires, engineers, sustainment | Done in the sim: calls for fire, adjust, Carleton lethality, counter-battery radar, displacement, minefields and breaching, digging in, stores. Breach time and counter-battery survival in band. |
| 4 C2 and comms | Done: nets with link budgets, PACE, HQ COPs and staff queues, planning time, CCIRs, succession, orders over the nets. |
| 5 Drones, EW, PNT | Done: drone flights and link dependence, the FPV strike chain, counter-drone, jammers, ESM/DF, GNSS and INS. FPV, precision under jamming and pre-network detect-to-strike in band; networked detect-to-strike just outside (backlog). |
| 6 Air and air defence | Done: aircraft on tasks, sorties, munitions in flight, radar, kill chains and WTA, airspace. Shahed raid and sortie rates in band. |
| 7 Logistics, medical, aggregation | Done: logistics, maintenance and medical in the sim (culmination test, evacuation delays in band); level of detail with Lanchester attrition fitted against entity runs; quiet skipping. |
| 8 Naval and undersea | Done: ships and submarines, radar and sonar, layered defence with magazines, damage control, mines, reload and UNREP. Red Sea vignette in band. |
| 9 Space, cyber, kill web | Satellites and imagery, GNSS and satcom helpers, scripted space events, cyber access campaigns. |
| 10 People and tools | A person's HQ seat and plans (server); the viewer's timeline scrubber and "what if" branch, layers for sensor footprints, viewsheds, jamming, the nets and kill chains in progress, a spectrum view, the per-unit *Why?* inspector, a detection inspector (why a unit sees another or not, with every factor), a link-budget inspector (each message hop by hop: queueing, airtime, attempts, SINR, fallbacks) and the after-action review (also `bgws aar`, `bgws why`, `bgws branch`); theatre packs (`bgws pack`); a scenario editor (`/editor`: theatre, time, weather, forces from the catalogue placed on a map, checked, saved or loaded into the game); the Behaviour Manager (`/behaviour`: graph views, and variants of any template made, checked and saved to `data/templates/custom/`, which the editor assigns to forces); the analysis workbench (`--html`, `bgws compare`). |
| 11 Calibration | The vignette suite, sensitivity and the research backlog run; the campaign itself is ongoing. |

### Performance (this 4-core container, release build)

| Case | Measured |
|---|---|
| Brigade, 2,000 land entities, whole tick | about 47 ms per simulated second (×21) |
| M6 sustained offensive, 47 forces, 72 h | 90 s wall (×2,900) |
| M7 littoral, 24 h / M8 joint strike, 24 h | 25 s / 20 s wall |
| Skirmish batch, rules deciders | about 0.1 s per game |

### Calibration backlog

- **Detect-to-strike, networked [P2]:** on the M4 drone line the median from
  a detection to the first FPV arriving on it is 5.35 minutes (95% CI
  5.03–5.97, 200 games), just above the 2–5 minute band. The HQ chain itself
  (report, staff, hand-off, launch) takes about 1.5 minutes; the rest is
  transit to targets 3–6 km away at 90 km/h, mid-range of the reported
  65–150 km/h. On voice nets (the scenario's `network: voice`) the same chain
  takes about 23 minutes, inside P2's ~20. CI runs it with
  `--backlog detect-to-strike-networked,attack-halt`, so it is reported but does not fail.
- **Attack halt [P14]:** in the advance-to-contact skirmish the attack stops at
  a median of a third of its strength lost (4 of 12 vehicles; 95% CI
  0.25–0.42, 200 games), just above the 10–30% band. The stored 0.25 came
  from a Red troop that started inside a dense wood its vehicles could not
  leave, so it never advanced. Vehicles now crawl out of such woods
  (`sim.route.crawl_kmh`), all of Red fights, and Blue loses one more vehicle
  before it halts. The pinned threshold (`people.morale.theta.pinned`) and
  the route weights do not move it: the halt here comes from the fight, not
  from morale. The next step is a playtest of the scene, not a figure tweak.
- **Vehicle march rates:** the research reference sources dismounted pace
  (P26, now a vignette: 2.3 km/h across generated mixed ground, in its
  1.5–3 km/h band) but no road-march or cross-country rate for vehicles. The
  road speeds by class (`env.road.*`) and the planner's march weights
  are [default] until a source is added.
- **Lethality:** meeting engagements in open ground end within a minute or two.
  Modern tanks do kill quickly at 1.5 km, but the time to lay and the rate of
  fire are worth checking against playtests.
- **Morale:** a unit can cascade from steady to routed within seconds under
  heavy fire. The hazard rates in `bgws_models::people` want playtesting.
- **Mostly unsourced figures:** most figures are `[default]`.
  `bgws sensitivity <vignette>` ranks which ones matter.

### Known gaps

- **The desktop shell** (`apps/desktop`, Tauri 2) runs the host and the game
  in its own process and shows the viewer, editor and Behaviour Manager in a
  native window; `bgws-server` is the same host for a browser. The channel is
  the one §16 describes: JSON commands one way; full frames when the units on
  the map change (or once a second) and binary position updates between
  them (about 20× smaller) the other.
- **Snapshots** are JSON for inspection, not restorable state. A game
  restarts from a moment by replaying its record there (`bgws branch`, or
  *Branch here* on the server's timeline), which is exact: a branch played on
  with the same deciders ends with the same fingerprint. The server scrubs
  through keyframes of each view (`--keyframe-s`), not snapshots.
- **Event logs** are JSONL by default. With `--features duckdb` a log named
  `*.duckdb` is a DuckDB table (`events`, the figures in `data` as JSON) that
  `bgws events` queries with SQL; `bgws aar` and `bgws why` read either.
  DuckDB is a feature because it compiles from C++.
- **Missile and air-defence models** work at the engagement level: kinematic
  flights, per-shot kill probabilities and kill chains. They do not model
  guidance laws or seekers.
- **Drones:** drones in flight are records in the drones module, not ECS
  entities, so the air-defence kill chains and WTA do not see them. The
  counter-drone loop engages them instead, and units with a short- or
  medium-range SAM and a radar take on fixed-wing drones out to
  `sim.drones.sam_reach_m`.

## The viewer

`bgws-server` (and `bgws-desktop`) serve the default frontend at `/`:

- **Left pane:** the game, and who decides for each side, with a warning when
  their API key is missing. The clock, with speeds, a timeline to look back,
  and a branch from any moment. Views (umpire, or one side's eyes), map layers
  and basemap (dark, streets, satellite). The forces, with strength and state.
  A map key.
- **Map:** units as MIL-STD-2525C symbols, with rings for suppressed, pinned
  and broken. Shots by outcome, shell impacts, drone strikes and sightings, as
  they happen. Kill chains, enemy tracks, sensor footprints, jammers and the
  nets. The selected unit's route, and what it can see: a radial sweep by
  the game's own sight rules, about 2,500 rays marched every 2.5 m. Along
  each ray the highest ground and roof seen so far hide what lies below
  them. A straight line from the eye keeps one elevation angle, so the
  woodland it passes under is the canopy seen above that angle, summed per
  class over angle bins. The result is drawn on cells of 5–12 m, depending
  on the unit's reach, with smooth edges. It agrees with the game's
  point-to-point sight lines at 99% or more of points, and takes about
  90 ms. Bright cells have a clear line. Faint
  cells see through trees, which may or may not hide a target. Dark cells
  are hidden by the ground or a building. On a pack with a sight surface
  (below), buildings and woods are traced cell by cell, so a unit in a town
  sees down its streets and not through its blocks. Elsewhere, woods and
  towns count as a chance by their land cover. A solid ring shows how far
  its best sensor looks, and a dashed ring how far its weapons reach. The
  view follows the unit as it moves. *Hide what it sees* turns it off, and
  the choice is remembered. A side's view shows only its own units' sight.
  In sight is not spotted: whether a unit picks a target out also depends on
  range, light, weather and the target's cover and movement (*Can it see…?*
  gives every factor).
- **3D terrain:** on a theatre pack, the *3D terrain* button (under the
  basemaps) drapes the map over the pack's own elevation. It is the same
  ground the game is played on, stretched ×1.5 upright because most theatres
  are low. It tilts the view, and buildings stand up as blocks at their mapped
  height (10 m where none is mapped). Units, tracks, shots and rings sit on
  the ground; drones and aircraft keep their height. Right-drag or Ctrl-drag
  to tilt and turn. Generated ground has no real elevation to show, so 3D is
  only offered on a pack, and only one built with an elevation map
  (`dem.pmtiles`; build an older pack again). The editor has the same
  *3D terrain* box.
- **Right pane:** the decisions waiting for you, when you hold a side
  (`--person`), with the rules' pick and the default marked. A feed you can
  filter, where a decision opens on its options and who chose. The selected
  unit's orders and phases, people, weapons and what it knows, with *Why?*,
  *Messages*, *Show what it sees* and *Can it see…?*
- **Elsewhere:** the HQ console (a side's seat, an objective and a task per
  unit), the spectrum strip, the after-action review, the end-of-game card,
  and *New game…* (any shipped scenario).

Only the map tiles come from the internet. `/classic` is the earlier
single-file viewer.

**The world map.** The viewer's *terrain* basemap and the scenario editor
(`/editor`) draw a world overview: coastlines, land cover, water, borders,
main roads and place names. It comes from the Protomaps planet at zoom 9 and
below, and maps over-zoom it beyond that. `bgws-server` proxies it at
`/world.pmtiles`. It fetches each byte range once and keeps it in the cache
(`$BGWS_CACHE`). It pins one daily build (`world.url` in the cache), so a
place looked at once draws offline later. `$BGWS_WORLD` names another
archive. A scenario's pack draws over its box in full detail, and the
editor outlines every pack. In the editor, drag a force to move it, or
select it and click the map.

To change the viewer: `cd frontend/viewer && npm install`. Then `npm run dev`
serves it at http://localhost:5173 against a running `bgws-server` (port
8765). `npm run build` writes `frontend/viewer/dist` (the viewer's `app.js`
and the editor's map, `map.js`), which the server embeds; commit it. CI fails
if it is stale.

To use another frontend, run `bgws-server <scenario> --web <dir>`: `<dir>` is
served at `/` (with `index.html` for paths that are not files), alongside the
server's own routes. A frontend may also be served from anywhere else: every
response allows any origin. `cargo build -p bgws-server --no-default-features`
builds the server with no frontend embedded. `protocol/PROTOCOL.md` lists
everything a frontend can send and receive.

## Real ground: theatre packs from PMTiles

`bgws terrain <dir> --bbox W,S,E,N` (or *New game… → Generate ground from
the map's current view* in the viewer) builds a theatre pack for a box. It
reads only that box's tiles from two planet PMTiles archives, by HTTP range,
and keeps them in a disk cache (`$BGWS_CACHE`, else `~/.cache/bgws/pmtiles` or
`~/Library/Caches/bgws/pmtiles`):

- **Elevation:** Mapterhorn's terrarium planet (`download.mapterhorn.com`,
  Copernicus GLO-30 plus national models), zoom 12, sampled onto the engine's
  grid (30 m by default, `--cell-m`). Copernicus is a surface model: its
  heights include trees and roofs, smeared over 30 m. The engine has every
  wood and building itself (the sight surface), so woods and towns are
  taken back to bare ground by a morphological opening: the lowest height
  nearby, then the highest of those, over `--ground-filter-m` (150 m; 0
  keeps the raw heights). This keeps slopes and drops the bumps roofs and
  canopy make. Water sits at its surface, not the lagoon floor.
- **Buildings in movement:** the share of each 30 m cell under building
  footprints is taken from the sight surface. A vehicle cannot pass a cell
  half or more built over (`sim.route.built_block`). Below that, its speed
  falls with the square of what is left open. Troops on foot get through,
  slower. Along a road, a route follows the road's own line, not the cells'
  centres, so a column in a town keeps to the street. A vehicle's straight
  run across country is kept only if it crosses no footprint. On the
  Kaliningrad pack, tracked routes across the town run through a building
  footprint for 1.9% of their length, down from 4.8%. Most of what is left
  is narrow streets whose 5 m cells touch a wall.
- **Sight surface:** `surface.bin`, on a 5 m grid. Every building footprint
  is drawn at its mapped height, from the archive's deepest zoom (15), where
  Protomaps keeps all of them: 63,000 in a 22 × 33 km box over Kaliningrad.
  About a fifth have a mapped height. The rest get a height by size: 6 m
  under 120 m², 9 m under 600 m², 12 m above. Woods are drawn by class. Sight
  lines step through it at 2.5 m. A line that passes below a roof is
  blocked. A line through trees counts its length toward its chance of
  being blocked (`env.landcover.*.los_block_per_100m`). The last 15 m at
  each end are clear, so a unit by a building or at a wood's edge sees out.
  The game's spotting and the viewer's shading use the same lines. A 16 ×
  12 km pack's surface is about 2 MB. Packs built before it fall back to
  land cover for sight.
- **Land cover:** a Protomaps OpenStreetMap build (`build.protomaps.com`, the
  latest daily one unless `--planet`), zoom 14. Land use and water polygons
  are painted in priority order (water over towns over wetland over woods
  over fields), then cells turn to dense urban by building footprint cover.
  Soil, wetness and forest obstacles follow from the class.
- **Roads:** from the same tiles, by a port of the project's Python
  road-graph builder:
  - tile clipping and T-junctions;
  - snapping within `--snap-m` (3 m), where motorways join only motorways
    and slip roads unless one really ends there;
  - degree-2 contraction.

  Tracks and service roads are kept for a tactical game. The output is in
  `roads/`: `nodes.bin`, `edges.bin`, `edge_geom.bin` and `adjacency.bin`
  (fixed-width little-endian), with `graph.json` describing the layout and
  the build's statistics.
- **For the viewer:** `basemap.pmtiles` and `dem.pmtiles`, extracts of the
  same archives that `bgws-server` serves under `/packs/`. The "terrain"
  basemap draws them, with hillshade and town names, with no internet.
- **To check a build:** `preview.png` shows land cover over hillshade, with
  rivers, roads by class and bridges. `pack.json` records the box, the
  archives used and their attributions.

A 16 × 12 km box west of Kaliningrad builds in about seven seconds once
cached. It has 1,900 km of road, and 96% of its nodes are in one network.

A scenario names a pack as `theatre.pack`: a folder under `packs/`
(`$BGWS_PACKS`), or a path. The host re-samples the pack into the scenario's
frame through latitude and longitude when their origins differ. The viewer's
New game card can move any shipped scenario onto a pack, keeping its layout,
and the editor's *Move onto the pack* does the same. On a pack's roads a
vehicle moves at the road's speed for its class, and route planning prefers
roads where they are quicker. Moving vehicles in a shipped scenario on the
Kaliningrad pack spend 53% of their time on roads, which cover 23% of the
ground. Generated ground has no road layer.

Moving a scenario onto a pack (New game's *Ground*, or the editor's *Move
onto the pack*) centres its forces on the pack and moves everything in it
with them: tasks, lanes, obstacles, measures, areas and victory conditions
(`ScenarioV4::translate`). The pack must hold the forces. The skirmishes and
M2 fit a 16 × 12 km box. M3 and the M4 drone line need about 22 × 33 km
(`--bbox 20.30,54.55,20.65,54.85`). M5–M8 span hundreds of kilometres and
play on generated ground. A scenario that names a pack it is not on fails
with a message saying where each is and how to move it.

CI builds a pack with no network: `crates/bgws-geodata/tests/offline.rs`
writes a small vector archive (two roads meeting at a junction, a wood, a
lake) and a terrarium elevation archive. It then builds the pack through the
same path as `bgws terrain`, and checks the heights, the land cover, the
water and the road graph. The tests against the real planet archives are
ignored by default (`cargo test -p bgws-geodata -- --ignored`), and so are
the engine's tests on a real pack (`BGWS_TEST_PACK=<dir> cargo test
--release -p bgws-run --test terrain_pack -- --ignored`).

### Moving on roads and across country

One planner (`bgws_sim::routing`) searches roads and open ground together.
It works on the terrain grid at its own resolution (30 m), in a corridor round
the move. Very long moves use a coarser grid, and a wider corridor when
nothing joins the ends. A cell's cost is its travel time (the road's speed for
the vehicle, or what the going allows), plus exposure, plus a verge's time to
get on or off a road. What the move is for decides the weights:

| Mode | Weighs |
|---|---|
| March | time; wheeled vehicles keep to roads (a strong preference), tracked ones cut across fields when that is quicker (a soft one) |
| Tactical | time, open ground by the land cover's concealment, more so near the enemy the side believes in; roads near that enemy cost more, the nearer the more |
| Assault | time only: straight at it |
| Stealth | concealment everywhere; roads are watched |

Exposure counts the time spent in the open: troops on foot are exposed for
as long as they take to cross, and a fast vehicle at least at a reference
pace. Threats are the side's confirmed tracks and its units' own sightings,
never the truth. The enemy at the destination is what the move closes with,
so only threats elsewhere are steered round. Recce, screens and observers
keep to cover but do not steer away from the enemy they are looking for.
An urgent move (orders `now`, or fire support getting into range) weighs
exposure only near a believed threat. A threat weighs fully only on ground
it can see: its sight is worked out over the terrain (`sim.route.sight_*`),
and in its dead ground, behind a crest or a ridge, half its weight
applies (`sim.route.masked`: dead ground hides a unit from direct fire, not from drones or indirect fire). So a move round an enemy takes the side it
cannot see. Orders set the mode per phase
(`march`, `assault`, `bound` for stealth, otherwise tactical).

**Rivers and bridges.** A pack's rivers are drawn onto the grid with their
depth. Across country, a vehicle crosses one only where it is shallower than
the vehicle can ford (`env.ford.*`: tracked 1.2 m, wheeled 0.75 m, on foot
0.9 m), at the cost of a ford's time. A road on the ground crosses what it
crosses (culverts, causeways). Bridges and tunnels sit on a second level of
the grid: a unit gets on and off them only along their road, from an edge
that joins them at a node, or onto open ground at their ends. So an
overpass is not a junction with the road beneath it.

`Ground::blow_bridge(edge)` drops a bridge, leaving whatever is under it.
`Ground::cut_road(edge)` takes a road's speed and any deck away. Routes
re-plan round both.

**Obstacles and craters.** A move plans round the minefields, ditches and
dragon's teeth its side knows of. Crossing one without a lane costs about a
breach (`sim.route.obstacle_s`, 1 h). So a unit goes round when that is
quicker, uses a lane when one is open, and goes to the edge to be breached
when there is no other way. Ditches and teeth stop vehicles only; troops on
foot cross them. Obstacles the side does not know of are not in the plan: the
unit meets them on the ground (and the planner leaks nothing to it). Heavy
rounds (155 mm, 122 mm rockets) crater the cell they land in. Two craters
(`sim.route.crater_cut`) cut a road through that cell. Each crater also slows
the going across it (`sim.route.crater_slow`). Both movement and the planner
see the craters.

**Columns.** On a road, a unit within `sim.route.column_gap_m` (100 m) of a
friendly unit moving the same way ahead of it follows at that unit's pace: a
column does not drive through itself. Off the road, units pass each other.

**Long moves.** A move longer than `sim.route.long_m` (20 km) goes by the
pack's road graph: Dijkstra over its nodes and edges, by the same weights.
It gets on at a node within `sim.route.access_m` (3 km) of the start, gets
off near the destination, and plans both end legs on the grid. Cut edges and
blown bridges are left out; when the graph does not join the ends, the move
is planned on the grid. On the Kaliningrad pack, routes by the graph take
about as long to drive as the grid's and plan in about half the time.

**Never stuck.** A vehicle standing in ground it cannot cross crawls out
at `sim.route.crawl_kmh`. That ground is a dense wood, or water a scenario
put it in. A route may crawl only within `sim.route.escape_m` of its start,
and through water only when it starts in water. So a unit on a river bank
still goes to a bridge or a ford. A unit sent into such a wood goes to its
edge (the closest approach). Roads are drawn on the 30 m terrain grid, and
routes cut its corners. A vehicle following a road into a cell beside it
(water by a causeway) keeps the road's speed. Before this, units on real
ground could stop for good in such a cell.

**Unreachable destinations.** When a destination cannot be reached (ground
the vehicle cannot enter, or water with no way over), the route goes as near
as it can (the wood's edge, the near bank) rather than in a straight line.

Every weight is a figure (`sim.route.*`, [default] until calibrated). On the
Kaliningrad pack, about half of moving vehicles' time is on roads, which
cover 23% of the ground. No plan there falls back to a straight line (an
ignored test, given a pack). A 20 km move plans in about 50 ms.

EW, drone-team and HQ displacements now move a unit to one side (across the
line to the enemy), swinging about its first site. Before, each one moved it
1.5 km further back, so over a few hours they drifted out of their area.

Next steps:
- calibration of road march rates and of where units leave the road.

### Terrain analysis

A staff studies the ground before a fight. BGWS does this once per theatre
pack and keeps the result with it (`analysis.bin`;
`crates/bgws-sim/src/appreciation.rs`), so games and the AI read it rather
than work it out again. `bgws terrain` does it as it builds a pack;
`bgws analyse <pack>` redoes it; a pack without one gets it the first time a
game opens on it (about 5 s for the 23 × 33 km Kaliningrad pack).

- **Exposure (dead ground):** from every 60 m cell, a man-high eye looks out
  48 ways to 2 km over the ground, roofs and canopy. The share of that
  ground it sees, by area, is the cell's exposure. Sight runs both ways, so
  it is also how much of the ground round can see the cell: 0 is dead
  ground, 1 open from all round. Woods and roofs count as solid here, a
  little coarser than the game's own sight lines. Over Kaliningrad, 46% of
  the ground is dead (under 0.1: the city, the woods, folds) and 17% open to
  view (0.4 and over: the open farmland).
- **Mobility:** each 30 m cell is go, slow-go (under 40% of the class's
  open-country speed) or no-go for tracked, wheeled and foot movers, by the
  game's own going (soil, slope, woods, buildings, rivers too deep to ford;
  roads are go). The width of the go-ground through each cell is kept too.
- **Chokepoints:** corridors are go-ground at least 400 m wide (about a
  company's frontage) in open areas of a square kilometre or more. Where
  two such areas are joined only through narrower going, the narrowest point
  between them is a chokepoint, noted with its width and the two areas'
  sizes.
- **Crossings:** road bridges over rivers, with the road's class and
  whether the water can be forded.
- **Observation:** local high points at least 5 m above the ground round
  them, scored by how much they see within 4 km.

The game uses it:
- **Routes:** tactical and stealthy moves count a cell's openness to view
  (`sim.route.exposure.*`) less in dead ground: in fully dead ground, 30% of
  it (`sim.appreciation.dead_ground_share`). They go round by folds and
  keep out of open fields. On Kaliningrad, the advance-to-contact test's
  moving vehicles keep to roads 94% of the time, from 76%, because most of
  its roads run through woods and the town.
- **Positions:** a unit looking for a better position prefers ground less
  open to view as well as cover (`sim.appreciation.cover_exposure_weight`).
- **Behaviour facts:** `self.deadGround` (exposure under 0.1) and
  `self.onKeyTerrain` (within 300 m of an observation point).
- **HQ planners:** an HQ is briefed on the best four observation points,
  chokepoints and bridges within 5 km of its units and objective, labelled
  `OP1`, `GAP1`, `BRIDGE1` and so on. Orders may use those labels as
  reference points.
- **Viewer:** the *dead ground* layer shades dead ground blue and ground
  open to view red. *Going* shades tracked no-go dark and slow-go amber.
  *Key terrain* marks observation points (OP), chokepoints (GAP) and bridges
  (BR), with a note on hover. The unit panel shows how open its ground is
  and the key terrain near it.

Generated and flat ground (no pack) has no analysis, so the calibration
vignettes are unchanged. The figures (`sim.appreciation.*`) are judgement
(confidence low) until checked against a staff's own appreciation of real
ground.

### How units carry out choices, and how HQs plan an attack

**Execution** (`crates/bgws-sim/src/decide.rs`, `systems.rs`):

- **Firing positions against the threat.** "Better position", "hull down
  then fire" and "into cover" used to pick the best cover within 400 m,
  wherever the enemy was. Now they look at the threats the unit knows of
  (seen in the last five minutes, or firing from a bearing it has not
  found). Within `sim.exec.reposition_m` (250 m), they pick a spot from
  which the nearest threat sees the turret but not the hull (hull-down to
  it), or failing that one hidden from all of them. Short moves come
  first, and none closes the range for nothing; lines of sight are traced
  over the ground and the buildings and woods.
- **Quick hull-down.** In a firing position chosen like that, a unit
  counts as hull-down after `sim.exec.occupy_s` (30 s), not after the
  usual settle-plus-hull-down time (4 minutes).
- **Real bounding.** Orders with mode `bound` used to mean "move
  stealthily". Now a bounding unit moves a bound (`sim.exec.bound_m`,
  250 m) at tactical pace, halts to cover (`sim.exec.bound_halt_s`, 30 s,
  not moving, so it fires as a halted unit), and goes on. Over 1.2 km it
  halts four times and arrives in 262 s against 142 s moving straight
  through (a test checks it).

Measured against the previous engine: the calibration vignettes are
unchanged, and Jev against the doctrine default moved from +0.07 ± 0.10 to
+0.10 ± 0.08 (result, 84 paired games), within the noise. The lab's
play-outs of "better position" at first contact in the advance to contact
(27 moments) show what the option costs: units that reposition lose fewer
vehicles than those that carry on (about 31% of the unit against 40%), but
they give up the advance (3% of the way to the objective made good
against 31%). That is a trade-off between survival and momentum, not a
blunder, and it is the choice a decider should be weighing.

**The HQ planner** (`crates/bgws-decide/src/hq.rs`, `heuristic_plan`)
used to send every lead unit abreast straight at the objective. With
`decide.heuristic.fire_and_movement` (on), an attack with at least
`decide.heuristic.fm_min_units` (3) lead units does this instead:

- The lead unit that reaches farthest **supports by fire**: from an
  observation point of the terrain analysis in reach of the objective on
  our side (OP1…), else from a stand-off of `sbf_reach` (70%) of its reach,
  with weapons free.
- The others move to a **release line** (`decide.heuristic.release`,
  1.5 km short of the objective), then **bound** onto it.
- A lead unit with under half its vehicles (`decide.heuristic.weak`)
  **supports by fire from where it is**, and the reserve takes its place.

With two lead units it attacks as before: splitting two only lets the
enemy beat them one at a time (tried first, it cost the small scenarios
dearly: −0.83 result in the armour duel).

`bgws lab suite --planner-ab` measures a planner against another, per
side: the new planner plans for the side under test and the old for the
other side and for both in the baseline game (the rules decide for every
unit). Over the suite's seven scenarios, both sides:

| seeds | pairs | better / worse | result Δ | exchange Δ |
|---|---|---|---|---|
| 1–6 | 84 | 14 / 13 | +0.08 ± 0.06 | +0.02 ± 0.02 |
| 7–12 (fresh) | 84 | 15 / 8 | +0.05 ± 0.05 | +0.02 ± 0.02 |

It changes only the scenarios with enough units: the combined-arms
skirmish (+0.58 and +0.33 result), the battalion attack (5 better, 1 worse
on the fresh seeds) and the mechanised hasty attack. The small ones play as
before.

### Generated scenarios

`bgws lab generate` writes a varied set of scenarios from short specs
(`crates/bgws-run/src/scengen.rs`) to `data/scenarios/gen-*.json` and
checks each loads; `--only <name>` writes one, `--spec <file.json>` writes
your own from a JSON list of specs, such as
`[{"name": "gen-my-attack", "kind": "attack", "echelon": "company",
"ratio": 2.0, "gapKm": 4.0, "hour": 6.0, "weather": "rain", "ground":
{"type": "generated"}, "minutes": 90.0, "seed": 7}]`. A spec names the kind of fight
(attack, defence, meeting engagement, ambush, withdrawal, river crossing,
attack into a town), the echelon (company or battalion), the force ratio
(attacking platoons per defending element), the gap between the forces,
the local hour, the weather, the ground (flat, generated, or a theatre
pack, where a river crossing is put across the pack's widest river and an
urban attack against its largest town) and the time limit. The standard
twelve are in the suite (four held out):

| scenario | what | rules vs rules, seeds 1–3 |
|---|---|---|
| `gen-company-attack` | 8 platoons on a dug-in 3-element position, 3:1, a river between | blue wins 3 of 3, in about 78 min |
| `gen-company-attack-even` | the same at 1.5:1, overcast | blue wins 3 of 3 |
| `gen-company-attack-night` (held out) | at night | blue wins 3 of 3 |
| `gen-battalion-attack` | 12 platoons on a reinforced company with tanks behind | blue wins 3 of 3, 79–104 min |
| `gen-company-defence` | blue's dug-in company against 1.5:1 | red wins 2, blue holds 1 |
| `gen-battalion-defence-fog` (held out) | at dawn in fog | red takes it 2, blue holds 1 |
| `gen-meeting-engagement` | 4 platoons a side, one objective between them | red wins 3 of 3 (the ground favours it) |
| `gen-ambush` | blue's column past 4 concealed, dug-in elements | blue wins 3 of 3, in 8–11 min |
| `gen-withdrawal` (held out) | 3 platoons fall back 4.5 km and must deny the position to 6 | red wins 3 of 3, after about an hour |
| `gen-kgd-company-attack` | on Kaliningrad's ground | blue takes it 1, fails 2 |
| `gen-kgd-river-crossing` | across a real river | blue wins 1, fails 2 |
| `gen-kgd-urban-attack` (held out) | a battalion into a town | blue takes it 2, fails 1 |

The pack scenarios are skipped, with a note, where the pack is not
installed.

Three things came with them:

- **An objective victory** (`{"kind": "objective", "side", "at",
  "radiusM", "holdS", "label"}`): the side takes it when it has fit land
  units (HQs aside) within the radius and the enemy has none, and wins when
  it has held it that long. At the time limit a side that has not taken
  its objective loses, unless both sides had one (then strength decides).
- **A mission for each HQ**, from the objectives as briefed at the start:
  `attack` (its own objective to take), `defend` (the enemy's, on its
  ground) or `withdraw` (its own or the enemy's, lying behind it, away from
  the enemy). The heuristic planner holds when defending whatever the
  odds, and when withdrawing sends every unit back to the objective to
  hold it; Jev's HQ prompt is told the mission.
- **Prepared defences** (`"dugIn": true` on a force): the unit starts dug
  in and hull down, as if it had spent the night digging.

First baselines on the twelve (seeds 1–4, both sides, 96 paired games
each) show what the original seven hid:

| under test | better / worse | result Δ | exchange Δ |
|---|---|---|---|
| planner: fire and movement | 12 / 34 | −0.21 ± 0.08 | −0.05 ± 0.02 |
| Jev 1.13 (tuned wording) against the doctrine default | 29 / 54 | −0.25 ± 0.11 | −0.04 ± 0.02 |

Fire and movement cost most on Kaliningrad's real ground (−0.75 result
in the company attack and the river crossing). The cause was not the plan
but the engine: a phase's route was planned when the order arrived, from
where the unit stood, so starting the next phase (the bound after the
release line), or resuming orders after any interruption, sent the unit
back towards its old position first. On flat ground a route is one
straight leg, which hid it; on real ground the attack wandered for 90
minutes. A phase's move is now planned afresh when it starts or resumes.
After the fix the planner A/B over all nineteen scenarios (seeds 1–4) is
33 better, 29 worse, result +0.03 ± 0.04, exchange +0.02 ± 0.01: the
Kaliningrad attacks now gain from it (exchange +0.11 and +0.03), the
fog defence still loses a little (−0.10), and the calibration vignettes
are unchanged. Jev does worst attacking a dug-in position (−1.00 in
both company attacks, 0 better in 8 at even odds) and in the ambush
(survival −0.16); it does no worse in the urban attack, the Kaliningrad
company attack and the fog defence. Both were tuned on the seven older
scenarios, which these now check.

**Tuning Jev on them.** Play-outs of Jev's worst games on the generated
training scenarios gave two patterns, now four cases. A unit halted part
way through its orders with ground still to cover chose overwatch where
resuming was 0.5–1.6 better (`d10-step-done.json`). On the even-odds
attack's first contact, a tank troop moving on step 1 of 3 stopped to
engage an unidentified, stationary contact at 2.4 km where carrying on was
0.7–1.4 better (`from-suite-generated.json`). A tune run (3 rounds, 12
rewrites) took none: the two best passed all 22 training cases (against
19) but each lost one of 30 consistency checks. The better of them, which
only rewrites the option descriptions (resume is right while steps remain;
hold and overwatch stall an unfinished plan; fire only at what is in
reach), was then played on all nineteen scenarios against the wording it
replaces (seeds 1–4, 152 paired games each against the doctrine default):

| wording | better / worse | result Δ | exchange Δ |
|---|---|---|---|
| before | 42 / 71 | −0.09 ± 0.05 | −0.02 ± 0.01 |
| now (`data/jev/wording.json`) | 43 / 71 | −0.01 ± 0.03 | −0.02 ± 0.01 |

Most of the gain is the even-odds attack (−1.00 to −0.25); the held-out
fog defence (−0.12 to +0.12) and withdrawal (−0.25 to 0.00, exchange
+0.08) improve too. It is still level with the doctrine default rather than
ahead, and it still engages on the first-contact cases (held out, so no
rewrite saw them).

A second tune run, with three more first-contact cases from the other
games (two landing in training), did take a rewrite: option descriptions
that say carrying on is usually right for a far, unidentified, static or
silent contact while orders still have ground to cover; that engaging is
right only when a weapon that can hurt the target reaches it; and that
light weapons do not hurt armour. Training cases 22 to 24 of 24; held-out
cases 4 right and 3 wrong to 7 right and 0 wrong of 8; held-out games
−0.07 to −0.00. Over all nineteen scenarios it is level with the wording
before it (−0.01 ± 0.03, 30 better and 61 worse): the even-odds attack
improves (exchange −0.15 to −0.04) and the held-out withdrawal is worse
(exchange +0.08 to −0.10). On the cases it is clearly better; in games,
Jev now matches the doctrine default rather than beating it.

**Not changed: the doctrine at D10.** The default template answers "order
step done" with overwatch whether or not the unit has ground left to
cover. A branch on a new fact, `orders.moveLeft` (its orders still lead
somewhere over 150 m off), was tried: resume when they do. Measured with
`examples/template_ab.rs` (the patch on one side only, paired with the
doctrine everywhere) it changed nothing under the rules (+0.00 ± 0.01
over 152 pairs: the rules carry on at first contact, so their units seldom
stop short) and nothing measurable as Jev's fallback (+0.01 ± 0.04 against
−0.01 ± 0.03). Shipped templates are kept free of conditions (their walks
are checked against the TypeScript golden scenes), so it was left out; the
fact is there for user templates.

**What firing would do.** Play-outs of first contacts showed carrying on
and engaging best in situations that looked the same in Jev's state, so it
now gets, for each enemy it has seen, `ifWeFire`: the weapon the engine
would fire and the chance one volley hits, from the engine's own sums
(error budget and presented area, bias averaged out). Identified: against
that platform, with `penetratesFront`; unidentified: both ways,
`volleyHitPctIfVehicle` and `volleyHitPctIfTeamInCover`. Also
`onOurObjective`/`fromOurObjectiveM` (within 600 m of where its orders
end), `friendsFiringOnIt`, and in its orders `headingFor.etaMin` and
`gameTimeLeftMin`. At 1.7–2.5 km a tank gun or a 30 mm cannon gives 80–95%
a volley against a vehicle and 3–9% against a team in cover.

Re-played with it, first contacts on the generated attacks favour carrying
on in 15 of 23 (by up to 1.45; IFVs as much as tanks), and stopping only
closer in (1.5 km, on the objective). Six of these are cases. With the new
state and the wording unchanged Jev gets five of them right (it carries on
past a far unknown it can barely hurt); a tune run found no rewrite that
held up on unseen cases (training 27 of 29, held out 8 of 9 already).

In games it changes nothing measurable. Over 256 pairs on fresh seeds
(5–12, the sixteen scenarios not on a theatre pack), the version with the
new state against the one without: result +0.000 ± 0.025, exchange
+0.001 ± 0.007 (paired). Jev's answers barely move (new contact: carry on
67% either way), because the decisive first contacts are a few
decisions in hundreds per game. It stays, being what a crew would know.

**Found on the way: guided missiles are resolved as gun rounds.** Land
direct fire puts every round, ATGMs included, through the gun error
budget, with the missile's flight speed as muzzle velocity. At 180 m/s the
ten-second flight to 1.9 km gives an ATGM team about 1% a volley against
a tank. A missile's guidance should take that error away; until it does,
defenders' missiles hardly count, which favours every attacker.

**Fixed.** A guided round fired direct (an ATGM) now reaches its target
with `direct.guided.ph` (0.8, a stated default: the reference gives no
figure), less what the gunner's suppression costs: up to 60% when the
gunner flies it to the end (wire, laser, joystick, fibre), 15% for fire
and forget (`direct.guided.suppression_operator`, `…_self`). Penetration,
zone, top attack, ERA and APS work as before, and a miss still suppresses.
Fights are far shorter: most generated scenarios end in 8–30 minutes
rather than 30–150, and the meeting engagement turns to blue. The
calibration vignettes are unchanged, as none has a missile in it; one
should be added when a reference figure for ATGM hit rates is found.

**Weapon choice and missile flight.** A unit used to fire the first of
its mounts that had rounds, and held fire when the target was beyond that
mount's reach: an IFV whose 30 mm cannon is listed first never fired its
4 km missile. It now fires, at the ordered or nearest target any of its
weapons reaches, the mount most likely to knock it out (the chance a round
lands, by the error budget or a missile's guidance, times the chance a hit
kills, by what the target is and what the round does to it); a missile is
kept for when it is clearly better (a gun at least `sim.fire.missile_keep`,
0.7, as likely is used instead). A guided missile volley is now launched
and lands range ÷ speed later (2–11 s at 0.4–2 km); guided by the gunner
to the end (wire, laser, joystick, fibre) it needs the gunner able to fire
and still seeing the target when it lands, and the gunner stays put until
then; fire and forget needs only the target there.

A new vignette, `atgm-engagement`, plays a dug-in ATGM team against a tank
platoon advancing over open ground and measures the share of missiles
launched that hit: 0.81 (200 runs), inside a 50–90% band. The band is
low-confidence: no combat data was found, only a TOW test figure (73.5% at
3,500 m for a good gunner) and unsourced Javelin claims (over 90% in
tests, over 80% in combat). Every other vignette is unchanged. The
planner A/B is where it was (+0.01 ± 0.05 result, seeds 1–4), and Jev
(plain) on fresh seeds 5–12 is +0.03 ± 0.04, exchange −0.01 ± 0.01,
against the doctrine default (before: +0.02 and −0.02). Most generated
fights end as before; the ambush is quicker (5–7 min).

**The shared picture, and what a unit can tell of a contact.** A unit's
decision state used to hold only what it saw itself. It now also holds
what its HQ's picture says, as a battle management system or the company
net would pass it on. The picture reaches the unit `sim.c2.picture_lag_s`
(60 s) late, and not at all if its HQ is down or its radios are out.
- An enemy it sees but has not identified gets `reportedAs`: what the
  picture says it is, how sure, how old the report is and who made it.
- Contacts friends reported within 4 km that it has not seen itself are
  listed in `reportedByFriends` (up to 4).
- A unit searching where its picture puts an enemy searches as if cued.
- A contact recognised but not identified is described by kind: a team on
  foot, an armoured vehicle or a soft-skinned vehicle (before: "something").
  The judge style uses this kind to answer "is it a vehicle?" without asking.
- An HQ takes a unit's contact report as an established track, not as one
  blip waiting for a second look (updates come minutes apart).

Building this found a bug: with no GNSS jamming the navigation step
returned early and never refreshed any receiver's fix, so every unit's
inertial navigation drifted unchecked (3–20 km of error by ten minutes)
and that error was added to every contact report. HQ tracks were born too
uncertain to keep, and the side picture stayed empty in every game. With
the fix, company HQs hold their contacts. Moving contacts still drop
between a unit's five-minute updates, which is fair: an attacker seen once
five minutes ago could be 2–3 km away.

Measured: in collected games 20 of 95 unit decisions carry friends'
reports. Every vignette stays in band; detect-to-strike moves from 5.35 to
4.90 min networked (now inside its 2–5 min band) and from 22.9 to 25.1 min
by voice (KS p 0.02, inside its 10–30 min band). The FPV calibration test
now plays 16 seeds by default, as 8 gave as few as 7 attacks. Over 48
seeds, cueing leaves the number of FPV attacks unchanged (79 with, 83
without). Jev (plain) on seeds 5–12 is +0.01 ± 0.04, exchange
−0.01 ± 0.01, against the doctrine default; before it was +0.03 ± 0.04,
so the extra facts have not yet shown up in results.

**Fire effects: the right round, blast, suppression.** Three gaps in
direct fire are closed.
- A mount fired only the first of its rounds. It now loads the one with
  the best volley kill odds: a tank's HEAT or an IFV's HE for troops and
  soft targets, sabot for armour. Changing rounds costs one reload.
- An HE or HEAT miss used to do nothing to troops. A burst now kills the
  share of a section (`sim.fire.team_front_m` × `team_depth_m`, 50 × 20 m)
  inside the round's Carleton lethal area, averaged over the fall of shot
  and reduced by cover; the whole element is lost with that chance, so the
  expected losses are right. A burst is felt as a blast (suppression). An HE
  hit kills a soft-skinned vehicle.
- Suppressive fire. Suppression already slowed, blinded and spoiled the aim
  of whoever felt it, but only aimed rounds caused it. Now a halted unit
  with nothing to aim at fires to suppress:
  - where it lost an enemy within the last minute
    (`sim.fire.suppress_memory_s`);
  - or, while a friend within 1.5 km (`sim.fire.support_m`) is moving, on an
    enemy its picture places within 200 m (`sim.fire.support_error_m`):
    support by fire.

  It uses cheap rounds only (`sim.fire.suppress_max_cost_usd`: bullets and
  cannon HE, not main-gun rounds or missiles), keeps a magazine, follows its
  rules of engagement, and never fires within 150 m of a friend
  (`sim.fire.danger_close_m`): fire lifts and shifts.

**Dismounts.** The catalogue gave an IFV platoon three infantry sections,
which never appeared in a game. They now ride with it (a vehicle lost takes
its share). The carrier is asked `dismountPoint` (here, in cover close by,
or stay mounted) when:
- an enemy is seen within 500 m (`sim.dismount.contact_m`);
- it is within 400 m of the end of the last phase of its move
  (`sim.dismount.objective_m`);
- it is in a town.

The doctrine gets out here in cover or with the enemy within 300 m, else in
the best cover within 150 m. After the drill (`sim.dismount.time_s`, 30 s)
the sections are a foot unit with the carrier's orders, rules of
engagement, sightings and HQ, and the carrier supports by fire. Troops
aboard count in their side's strength from the start, so it never jumps
when they get out (tested). Remounting is not modelled. The unit state
shows `infantryAboard`.

**Doctrine drills.** At three decision points the doctrine (the rules
decider, and the default Jev is measured against) carried on where a drill
says otherwise:
- D3, fired on by an unseen shooter: in the open, get into cover from the
  bearing; in cover, search along it.
- D2, shooter located: return fire, but a vehicle in the open goes hull-down
  first, and a shooter beyond all its weapons' reach is left by pulling out
  of its sight.
- D6, after a volley: a unit holding ground under fire from a shooter it
  has seen fires and moves to another position.

Mutual support: a unit advancing by bounds or tactically halts while a
friend within 600 m (`sim.doctrine.mutual_support_m`) is pinned, and covers
it.

Measured: every vignette passes as before; networked detect-to-strike is
5.22 min, still just over its band and on the backlog. Over seeds 1–6 of
the generated scenarios with the rules on both sides, the winners are
unchanged except the even company attack, which blue now wins in all six
(it lost one in 90 minutes before), and fights against dug-in infantry end
sooner (the battalion attack's longest game 135 → 33 min) while the
company defence lasts longer (23–55 min, from 23–33).

**Variance: crews, weapons and leaders.** Every crew of one troop quality
was the same, and nothing ever failed.
- Crews differ. Each unit draws its skill and its leadership about its troop
  quality's (1σ `sim.people.skill_spread`, `leadership_spread`, 0.1 on a 0..1
  scale), from the game's seed, so a replay draws the same crews (tested).
  Skill sets aim and reaction time; leadership sets how morale holds. A
  crew that knocks something out gains experience.
- Weapons fail. A gun or machine-gun volley jams or misfires with chance
  `sim.fire.gun_stoppage_p` (1%), out for `sim.fire.stoppage_s` (30 s). A
  guided missile fails in flight with chance `sim.fire.missile_fail_p` (6%,
  from the roughly 94% reliability claimed for current ATGMs). The odds
  units choose weapons by allow for it.
- Leaders are hit. Losses can take a unit's leader (the chance is the share
  of the unit lost). It then asks no new decisions for
  `sim.people.leader_replace_s` (2 min) until the second in command takes
  over, carries on with what it was doing, and feels the shock in its
  morale. The unit state shows `leaderReplacedInS`.

Two doctrine gaps showed once crews differed:
- A unit that rallied (D11) held to recover, which dropped its orders for
  good; tanks shaken while waiting at a minefield never went through the
  lane. Rallied and no longer under fire, the doctrine now rejoins.
- "Rejoin" did not resume orders as its description says; it withdrew to
  the nearest friends. Fixed.

The counter-battery test needed 32 seeds, not 16, to show its contrast. A
fast enemy crew can now catch a scooting battery in its two minutes out of
action, and a poorly led battery can break at the first hit. Over 32 seeds
a battery that stays survives 69% and loses 14 guns; one that scoots
survives 84% and loses 7. The guard is now "at least twice" rather than
"more than twice".

**Validation: three new vignettes, and what they found.** Each adds a check
on something nothing checked before. Each source is secondary, so each band
is low-confidence.

| Vignette | Claim | Source | Result |
|---|---|---|---|
| `tank-engagement-range` | tank platoons meeting in open country open fire at 2–3 km | Desert Storm: VII Corps average 2,200 m, US armour 2,200–2,800 m (an armour history quoted on a forum) | 2,893 m mean |
| `suppression-recovery` | troops stay suppressed about 2 min after the fire stops | encyclopaedic, no primary citation | 93 s median (band 1–4 min) |
| `spot-report-latency` | a contact reaches the HQ's picture within 5 min of the sighting | the SALUTE spot report task standard (study-guide summary) | 2.05 min median |

The first one failed at 1,388 m: the heuristic HQ planner ordered every
unit, attackers included, to hold fire until short range (half its weapon's
reach). Attack orders now fire at will (armour engages at its greatest
effective range); defence, reserve and support orders still hold fire to
short range. Games against a dug-in defence end sooner (the company attack
in 8–9 min instead of 10–19), and the meeting engagement is less one-sided
(red wins one of six seeds; it won none before).

That change let blue's tank and IFV in the drone line kill the red infantry
at 3 km before the drones reached it, which starved both detect-to-strike
vignettes and the FPV test. All three now measure the drone line without
blue's armour, the strike chain alone. Networked detect-to-strike is then
3.4 min, inside its 2–5 min band, and it leaves the calibration backlog.

Other baselines accepted: ATGM hit share 0.81 → 0.75 (missiles now fail
6% of the time).

Sensitivity: the suppression recovery time is almost all
`people.suppression.decay_tau_s` (Sobol total index 0.97), an unsourced
default (60 s), now held by the new vignette.

Jev (plain) on fresh seeds 5–12, against the doctrine default after items
3–6 (all 19 suite scenarios, 304 paired games, $0.26): result +0.07 ± 0.04,
exchange +0.02 ± 0.01. Before these items it was +0.01 ± 0.04 and
−0.01 ± 0.01. The default it is measured against changed too (the drills
and fire at will in the attack), so the two numbers are not a strict
before-and-after; but Jev now does better than the doctrine across these
scenarios, most clearly in the skirmishes (+0.25 to +0.38 result).

Re-measured on this engine (seeds 1–4, all nineteen, 152 pairs each):

| under test | better / worse | result Δ | exchange Δ |
|---|---|---|---|
| planner: fire and movement | 34 / 26 | +0.02 ± 0.05 | +0.02 ± 0.02 |
| Jev against the doctrine default | 31 / 61 | −0.08 ± 0.05 | −0.05 ± 0.01 |

The planner is where it was. Jev is clearly behind now: with missiles
that hit, its departures from doctrine cost more (the generated battalion
attack is worse in all 8 pairs). Its cases, wording and tuning all predate
this engine.

Play-outs of its six worst games here put its cost per decision above the
doctrine default's at every point measured: new contact 0.17 against 0.07,
fired on 0.22 against 0.16, after a volley 0.90 against 0.30. After a
volley it fires and moves in 92–95% of answers, where an IFV pressing an
advance should keep its fire on (cost 2.1 and 3.1); a hidden ATGM team
fires at first sight; an ATGM team under tank fire pulls back instead of
answering. Seven of these are cases (`from-suite-generated.json`, the
last seven). Two tune runs on this engine (the first cut short when the
proposer's replies overran its token limit, now 32,000) found no rewrite
that held up: the best scored 30 of 34 training cases, no better than the
wording now, and every one broke consistency checks. Sending "after a
volley" to the doctrine default instead (a point cut-off of 1) gave
−0.05 ± 0.04 against −0.08 ± 0.05, within the noise.

On this engine the doctrine default plays the unit-level decisions better
than Jev 1.13 does, and rewording what Jev is told no longer closes the
gap.

### How Jev is asked: request styles

TypeSafe's guidance for System One models (its skill file and docs) says
to send each item as its own question, keep every field of the state
relevant to it, ask plainly (no preamble; options described in the
criteria), not gate on confidence, and above all not to ask the model
what an agent should do next: break the decision into the judgments it
depends on and write the decision in code. The client had done the
opposite on most counts, so it now has three styles, chosen with
`BGWS_JEV_STYLE`:

- `legacy`: as before (one request per side with every unit's state, a
  templated instruction with the balance rule and doctrine, a confidence
  cut-off).
- `plain` (the default): each land unit in its own request, sent in
  parallel; its state cut to what the decision point needs
  (`jev_ask::trim_state`: the enemy it is about and two others, ground,
  orders; weapons only for D8, friends only where help is an option);
  one plain question; each option described by its action's one
  sentence, the doctrine's lean as a short tag; no cut-off. World and HQ
  questions are asked as before. The pinned model is now Jev 1.14.
- `judge`: judgments from Jev, decisions in code (`jev_judge`). At D1, D2,
  D6, D8 and D10 a truth table reads the exact facts from the unit's state
  (range, reach, the chance a volley hits, orders, posture, cover) and
  asks Jev only the yes/no questions a row turns on and the state cannot
  settle; every other point takes the doctrine default without asking.

The tables were fitted to 179 played-out moments on the engine where
missiles hit. What they found is mostly the doctrine: carry on at a new
contact (engage only an identified or judged vehicle within 600 m that can
reach the unit), answer a shooter you can see (pull back when you cannot
hurt it and stand in the open), keep firing after a volley, take a better
position when halted and a volley has under 10%, resume orders that still
lead somewhere. On those moments they cost, per decision against the best
option, D1 0.075 (doctrine 0.078), D2 0.088 (0.089), D6 0.097 (0.097),
D8 0.038 (0.047); Jev choosing the whole action cost 0.17–0.90. The one
judgment that splits a row is whether a close, unidentified contact is a
vehicle. Another tried, whether the enemy has spotted the unit, read the
same (0.27) where engaging won and where it lost, so it is not asked.

Measured on fresh seeds (5–12, all nineteen, 304 pairs each, against the
doctrine default):

| style | better / worse | result Δ | exchange Δ | cost |
|---|---|---|---|---|
| legacy, Jev 1.13 | 64 / 131 | −0.12 ± 0.03 | −0.06 ± 0.01 | $0.41 |
| plain, Jev 1.14 | 96 / 117 | +0.02 ± 0.04 | −0.02 ± 0.01 | $0.26 |
| judge, Jev 1.14 | 34 / 51 | −0.00 ± 0.02 | −0.01 ± 0.01 | $0.012 |

On the cases: legacy 80% (invariance 75/76; its wording was tuned on
them), plain 62% (65/76), judge 71% (76/76). Both new styles take Jev
from clearly behind the doctrine to level with it; neither is ahead.
`plain` still lets Jev choose and differs from the doctrine in two games
of three; `judge` plays nearly as the doctrine does, in a way that can be
read and tested, at a twentieth of the cost. Jev 1.14 is no better than
1.13 on the cases.

Making them found a fault in generated ground: it kept the terrain but
dropped the generator's rivers and roads, so every river was impassable
water with no bridge. Half the 3:1 attack's start line sat behind a stream
and never moved. Generated ground now keeps both (the roads with their
bridges), as theatre packs do.

### Urban blocks

When a pack has a sight surface, the town is cut into blocks as the game
loads (`crates/bgws-sim/src/urban.rs`): built-over 30 m cells joined to
their neighbours and broken wherever a road crosses, so a block is the
ground between streets. Packs need no rebuild. Each block keeps its
footprint, area and density, its buildings' mean and greatest height,
floors (3 m a storey), an estimated material, and how much of it stands.
OSM rarely says what a building is made of, so the material is a rule of
thumb by height: under 7.5 m light (houses, timber, thin brick), under 15 m
masonry, above that reinforced concrete. Over Kaliningrad that gives 4,999
blocks (1,157 light, 3,205 masonry, 637 concrete), worked out in under a
second.

- **Cover and concealment:** a unit in a block has its material's cover
  (`sim.urban.cover_*`: 0.6, 0.8, 0.9) against direct fire and suppression,
  and concealment 0.85, where its land cover gives less.
- **Indirect fire:** troops halted in a block are under roofs. A burst's
  lethality on an element there falls by the material's protection
  (`sim.urban.protect_*`: 0.5, 0.75, 0.9) times the share still standing.
- **Upper floors:** troops settled in a block watch from its upper floors,
  up to the fourth (`sim.urban.observe_floors`). Their eye height is
  `(floors − 1) × 3 m + 1.5 m`, so a section in a five-storey block sees
  over the roofs of the houses round it.
- **Damage and rubble:** each 152/155 mm shell or heavy rocket that lands in
  a block brings down 60 m² of light buildings, half that of masonry and a
  quarter of concrete (`sim.urban.rubble_m2_per_heavy_round`,
  `resist_*`). Sight lines see its buildings lower by the share fallen,
  rubble slows the going through it (up to 60% for vehicles, half that on
  foot), and there is less roof left to shelter under. Each salvo logs
  `urban.damage` with what still stands. On Kaliningrad, 40 rounds on one
  central block leave 76% of it standing and its tallest roof 12.9 m, from
  17 m.

The unit panel shows the block a unit is in. The figures are estimates
(confidence low) until they are checked against trials or history. The
calibration vignettes are on open ground, so blocks do not move them.

### Units occupy space

A unit keeps one position and one decision maker, but its surviving
elements (vehicles, teams) stand in a formation by what it is doing
(`crates/bgws-sim/src/formation.rs`):

- **column** on a road or a march, and when withdrawing: nose to tail along
  the track the unit has driven, so the column bends with the road;
- **wedge** across country: the lead in front, the rest echeloned back;
- **line** when halted: abreast across the nearest threat it knows of.

Spacing is `sim.formation.*` (50 m between vehicles, 8 m between teams; 50 m
and 5 m in a column). A vehicle that would stand in water or a block of
buildings is drawn in toward the unit. Sensing looks from the unit and its
element nearest the target, at the target's three nearest elements, so a
unit half behind a wood is seen by its exposed half. Indirect fire strikes
each element where it stands: a burst on a dispersed company hits fewer
vehicles than one on a bunched column. The viewer draws each element as a
small dot round the unit's symbol (`FrameEntity.elements`, east and north
metre offsets).

Formations moved detect-to-strike-networked into band (4.9 min, from 5.35)
and counter-battery survival from 0.895 to 0.900. A battery that stays put
under counter-battery fire now survives more often (24 of 32 games, from 19)
because its guns stand 50 m apart rather than as one bunched target; the
`fires` test guards the gap to shoot-and-scoot (32 of 32), not a fixed rate. The `c4` integration
test's pooled detect-to-strike median swings with the seeds (362 s on seeds
1–8, 90 s on 9–24; the code before this change failed the old guard on
seeds 9–24 too), so its guard is a wide sanity band and `D2S_SEEDS` picks the seeds.

Licences: OpenStreetMap data is ODbL (attribute it; a road graph and land
cover derived from it are derived databases). Mapterhorn asks for
attribution. `pack.json` carries both.

## Game records

A record (`--record`, or *Save* on the server) holds the scenario, seed,
build, machine, the run setup (flat ground, start-of-game planning), every
figure set away from its default, every model answer, and every plan and
seat change a person made. Each command carries its position in the answers,
so a replay applies it at the same simulated time and the same point in the
game. A record saved mid-game says where it stops (`endedAt`).
`bgws replay` checks the replay against the record's fingerprint.

## Interoperability: DIS and HLA 4

`bgws play <scenario> --dis <host:port>` publishes the game to other
simulations as DIS version 7 (IEEE 1278.1-2012) Entity State PDUs over UDP,
and plays in real time, and (with `--dis-listen`) takes in other
simulations' entities, fires and detonations. These are phases 1 and 2 of
`docs/design/INTEROP_PLAN.md`. Phase 3 joins HLA 4 federations (below).

```sh
bgws play gen-kgd-company-attack --dis 127.0.0.1:3000             # real time, published
bgws play gen-company-attack --realtime 4                      # 4x real time, not published
bgws play gen-kgd-company-attack --dis 192.168.1.255:3000 --dis-exercise 2 --record game.json
bgws replay game.json                                          # exact, no network
bgws play gen-kgd-company-attack --dis 239.1.2.3:3000 --dis-listen 0.0.0.0:3000 --dis-group 239.1.2.3   # two-way, multicast
bgws export-ground gen-company-attack ground.tif               # our ground, for partners
bgws correlate gen-kgd-company-attack partner.tif --json diff.json # before an exercise: do the grounds agree?
```

**DIS codes.** Every catalogue platform and munition has a SISO-REF-010
(v35) entity type, and each record's `$dis` note names the table and row it
came from. Generic platforms carry the category (a Tank, an Armored
Fighting Vehicle, an Air Defense unit), and the scenario sets the country.
Where the choice was not obvious, the codes follow SISO's own placement of
real equivalents: Iskander, M270 and coastal missile launchers are
Self-Propelled Artillery, and S-400, Patriot and Strela-10 are Air Defense.
People on foot are life forms with their equipment class (assault rifles,
anti-tank missiles, mortars); installations are cultural features. A test
checks every code fits its platform's domain and is a SISO value.

A scenario may carry an `interop` block: `exercise`, `site`, `application`,
the `country` per side (else 0, Other) and the `force` per side (else blue
1 Friendly, red 2 Opposing, neutral 3 Neutral).

**Where things are.** The sim's ground is a flat local map: east and north
metres from the theatre origin, height above mean sea level. Positions are
published as geocentric (ECEF) coordinates on WGS-84:
- latitude and longitude from the east/north position;
- height is the sim's height plus the EGM96 geoid undulation, from a 15'
  grid in `data/geoid/`. NGA's check points agree within 6 cm. The
  undulation is 29.4 m at the Baltic theatre's origin, so without it every
  entity would float or sink by that much in other simulations.
- Taking the 3-D position through the origin's tangent plane instead would
  lift entities by the Earth's curvature (8 m at 10 km, 70 m at 30 km), so
  heights are not done that way.

Orientation is DIS's ψ, θ, φ, from heading and the ground's slope under
each vehicle.

**What each state says.** Damage comes from strength (none, slight,
moderate, destroyed). Land vehicles show mobility and firepower kills, smoke
and flames on a wreck, the engine running and a concealed position.
People's posture (walking, kneeling, prone) is shown too. The marking is
the unit's label, cut to 11 ASCII characters.

**Dead reckoning.** A state is sent:
- the first time an entity is seen;
- every 5 s (the DIS v7 heartbeat);
- when a receiver's extrapolation (static, or constant velocity) would
  stray over 1 m or 3°;
- when the entity's look changes.

A company attack sends about 0.2 states per entity per second instead of 1.

**Real time.** Decisions no longer stop the clock:
- questions go to the deciders in the background;
- a unit with a question out carries on with what it was doing;
- each answer applies when it comes back, and never sooner than the crew's
  reaction time after the question.

So a slow decider costs time in the world, as it would for a real crew.
`--realtime <speed>` paces the clock without publishing. The record notes
the game was played in real time, with the time each answer came back, so
`bgws replay` is still exact (tested). The analytic mode, paused while
deciders think, stays the default for experiments.

**Checked.**
- Unit tests:
  - the Entity State PDU's 144 octets against the standard's field offsets;
  - the geoid against NGA;
  - DIS angles (level and facing north at 0°, 0° gives θ = −90°);
  - placement round trips to 1 cm;
  - appearance bits against SISO-REF-010's layouts;
  - dead-reckoning thresholds.
- Integration tests:
  - a real game published over UDP: every PDU parses, every entity is
    refreshed within the heartbeat, ground vehicles stand on our ground
    once the geoid is taken off;
  - a real-time game replays exactly;
  - pacing at 10× follows the wall clock.
- By hand: 4 simulated minutes at 4× took 59.8 s of wall time. An
  independent listener received all 789 PDUs from 15 entities, none
  malformed, each with the expected type.

### Phase 2: two-way

**Other simulations' entities.** An Entity State PDU from another site or
application makes a remote entity here:
- its platform comes from its DIS type: the catalogue record with the same
  kind, domain, category, subcategory and specific, else the closest by
  those, else a stand-in for its kind and domain (logged once);
- its side comes from its force;
- only its owner moves it: between states it is dead-reckoned, and a land
  entity stands on our ground;
- our units see it, report it and fire at it, but it does not see, decide
  or fire here, and counts in no side's strength;
- it is taken off when its state says it is deactivated, or after 12 s
  with nothing from its owner.

**Who judges damage.** Each simulation judges damage to its own entities.
Our fire on a remote entity goes out as a Fire PDU and a Detonation PDU
(hit, burst near, or missed onto the ground) for its owner to judge; we set
its health to whatever its owner then reports. A detonation from another
simulation at or near one of ours is judged here, with our damage models
and our protection, and logged as `interop.detonation`, so the AAR and
`bgws why` explain it like our own fire. A detonation is judged once even
if the packet comes twice.

**What goes out.**
- a gun volley: a Fire and its Detonation;
- a guided missile: a Fire at launch, its Detonation on arrival, under the
  same event;
- artillery and mortars: a Fire per salvo, a Detonation at its impact
  centre for every simulation to judge its own entities near it;
- fire to suppress: a Fire and a Detonation on the ground at the spot.

**Management.** Stop/Freeze stops our clock and Start/Resume restarts it;
both are acknowledged. Inputs apply only at tick boundaries and are
recorded with the tick they applied at, so `bgws replay` of a federated
game is exact with no network.

**Terrain.** Two simulations only agree on who sees whom if they stand on
the same ground (the plan's section 2 says why they usually don't):
- `bgws correlate` compares our ground with a partner's elevation GeoTIFF
  (add `--ellipsoidal` if its heights are above the ellipsoid): the height
  error on a grid, and sight lines judged on both grounds with the same
  method (4/3 Earth refraction, 2 m eyes). Against its own export our
  ground agrees to 3 cm rms with 1 of 2,000 sight lines judged differently;
  reading the same heights as ellipsoidal shows the 29.4 m geoid step.
- During a game, a remote land entity reported more than 2 m off our ground
  is logged (`interop.terrain_mismatch`, once a minute per entity), so the
  AAR shows where the worlds differ.
- A game on generated ground does not join an exercise, because no partner
  has that ground, unless its `interop` block says `"sharedGround": true`
  (after `bgws export-ground` has given it to every partner).

**Checked (phase 2).**
- Unit tests: remote entities join, dead-reckon, time out and are never
  damaged, moved or decided for by our systems; a detonation is judged
  once; PDUs in become the right inputs, with stand-ins; terrain mismatch is
  noted once a minute; correlation finds a uniform 3 m shift and a removed
  ridge's sight lines (every disagreement near the ridge).
- Two simulations over UDP on localhost, one owning blue and the other red,
  each keeping its own objectives: they see and fight each other, blue's
  side judged 6 detonations on its entities and red's 21, red lost 83%, no
  entity was owned twice and no detonation applied twice. Each side's
  record replays exactly with no network.

### Phase 3: HLA 4

`--hla <host:port>` joins an HLA 4 (IEEE 1516-2025) federation through a
Federate Protocol server, such as Pitch pRTI's, with the RPR FOM. It
publishes the same truth and fire as DIS and takes in other federates'
entities and fire the same way.

```sh
bgws mock-rti                                                   # a small RTI on 127.0.0.1:15164, for trying it
bgws play gen-kgd-company-attack --hla 127.0.0.1:15164 --federation exercise1 --fom RPR_FOM_v2.0_1516-2010.xml ...
bgws play gen-kgd-company-attack --hla rti:15164 --time-managed --federates 2 --record game.json
bgws replay game.json                                           # exact, no RTI
```

**How it talks to the RTI.**
- **Wire messages:** the IEEE 1516.1-2025 Federate Protocol protos, as
  published in Pitch's FedProClient, compiled to Rust (`crates/bgws-interop/proto`;
  "Reprinted with permission from IEEE 1516.1(TM)-2025").
- **Session layer:** written from FedProClient's own code. It has:
  - the 24-octet header and numbered messages;
  - heartbeats;
  - call responses matched to their calls;
  - after a dropped connection, a resume that sends again whatever the
    other side missed.
- **Objects:** each entity is a `BaseEntity.PhysicalEntity` subclass chosen
  by its DIS kind and domain (`GroundVehicle`, `Aircraft`, `SurfaceVessel`,
  `Lifeform.Human`, …). It carries the DIS identifier, type, spatial (static
  or DRM F,P,W), force, marking, damage and the appearance flags RPR has.
  Updates follow the DIS dead-reckoning rules.
- **Interactions:** fire goes out as `WeaponFire` and `MunitionDetonation`.
  Entities are named in them by their object instance names, which we
  reserve as `BGWS-site-application-entity`.
- **The RPR FOM files are not shipped.** SISO publishes no licence terms for
  them, so pass the federation's own with `--fom`.

**Time management.** `--time-managed` makes the game time-regulating and
time-constrained, with a 1 s lookahead, and plays in lock step:
- the clock waits for deciders, as in analytic play;
- each second's updates and fire are stamped a second ahead;
- each second ends with a time-advance request.

Everything stamped up to the grant arrives first, in a fixed order. With
`--federates N`, the game starts only when all N have joined and met at the
`ReadyToRun` synchronization point. Without that, the first federate to
join would run ahead while the others were still joining. So a time-managed
federation run twice with the same seeds gives the same games (tested).
Without `--time-managed` the game plays in real time, as on DIS.

**Checked (phase 3).**
- Byte fixtures:
  - the session header;
  - the HLA encoding's alignment (a double after an octet starts at offset 8);
  - RPR's entity type, identifier, marking and event identifier;
  - the spatial variant record (60 octets for F,P,W, 48 for static);
  - round trips of WeaponFire and MunitionDetonation.
- **Session against the mock RTI:**
  - a quiet session heartbeats and stays up;
  - 300 updates through ten dropped connections arrive once each and in
    order;
  - 2,000 calls from 8 threads each get their own answer.
- **Two federates through the mock RTI, time-managed** (the company attack,
  12 minutes):
  - they fight: blue judged 6 detonations on its units and red 17, and red
    lost 83%;
  - nothing is owned twice and no detonation is judged twice;
  - a second run gives the same two games;
  - each record replays exactly with no RTI.
- **Real time:** two federates see each other's units.
- **By hand:** the mock RTI as its own process. A federate joined through the
  CLI, time-managed, played 4 minutes in 1 s, and `bgws replay` matched.

**Not yet checked:** against pRTI or any other real RTI, and against a
third-party RPR federate. The mock RTI was written from the same reading of
the protocol as the client, so only a real RTI can show a misreading.

PDUs are built with the `dis-rs` crate (MIT). Its version 0.14 underflows
on markings longer than 11 bytes (in a release build it loops until memory
runs out), so markings are always cut first.

## Tests and CI

```sh
cargo test --workspace --exclude bgws-desktop     # unit, property, golden, determinism, vignette and integration tests
cargo clippy --workspace --exclude bgws-desktop --all-targets -- -D warnings
cargo test -p bgws-gpu --features gpu   # GPU twin tests (skip without an adapter)
target/release/bgws vignettes -n 200 --store data/regression   # §15.8: fails on out-of-band or a shifted distribution
```

`.github/workflows/rust-engine.yml` runs all of this on every change to the
workspace, checks the server builds with no bundled frontend, and type-checks
and rebuilds the viewer (`frontend/viewer`) to check its committed build.
Twenty benchmark vignettes run against stored baselines, all in band; the
attack halt stays on the calibration backlog (its median sits at the band's
edge, 0.25 in one run of 200 and 0.33 in another). Batches and
vignettes answer with the rules and the decider crate's heuristic planner, the
same planner the host falls back to.
