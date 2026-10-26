# BGWS in Rust

The engine of `apps/bgws/docs/plan/RUST_ENGINE.md`: a local, physics-first,
multi-domain simulation with Jev and Claude as decision makers, written in
Rust. It sits beside the TypeScript build and does not replace it.

```sh
cd bgws-rs
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
target/release/bgws pack packs/baltic --name baltic --lat 54.2 --lon 18.6   # a theatre pack
target/release/bgws batch skirmish-symmetric-control-v1 -n 200 --flat
target/release/bgws vignettes --store data/regression   # vignettes against their bands and baselines
target/release/bgws sensitivity shahed-raid --html sens.html   # Morris + Sobol, the research backlog
target/release/bgws vignettes --html workbench.html  # the workbench page: each vignette against band and baseline
target/release/bgws behaviour --html behaviour.html   # the Behaviour Manager: every template's decision graphs (/behaviour on the server also makes and edits variants)
target/release/bgws compare breach-time --set engineers.breach.rate_mph.line_charge=1000 --html cmp.html   # A/B a figure change
target/release/bgws figures --markdown               # every figure, sourced or [default]
target/release/bgws platform generic/mbt             # a resolved catalogue record
target/release/bgws-server m4-drone-line --person blue --record game.json   # then open http://127.0.0.1:8765/ (/editor, /behaviour)
cargo build --release -p bgws-desktop                # the desktop shell (needs libwebkit2gtk-4.1-dev)
target/release/bgws-desktop m4-drone-line --deciders rules   # the same game and pages in a native window
```

Deciders: `--deciders rules|jev|claude|local` (or as the scenario says).
Keys come from the environment only: `OPENROUTER_API_KEY` for Jev,
`ANTHROPIC_API_KEY` for Claude. A local model is any OpenAI-compatible server
(`--local-url`, `--local-model`). With no key or no network the template
fallbacks decide and the log says why.

## The workspace (plan §3.1)

| Crate | What it holds |
|---|---|
| `bgws-core` | ids, µs time, units, WGS84/ECEF/ENU, the figures registry, keyed ChaCha8 streams, the event log and fingerprint, question and answer types |
| `bgws-catalogue` | 105 platforms and 48 munitions built from parts (movers, bodies, signatures, sensors, mounts, radios, emitters, stores), class defaults, derivation, validation |
| `bgws-env` | terrain grids and pyramids, synthetic theatres, land cover and micro-terrain, soil and trafficability, roads, the change layer, tiles and theatre packs, sea state and sound speed, weather fields, optics, sun and moon, thermal days |
| `bgws-prop` | optical LOS with canopy, horizons, Deygout diffraction, ITM-style loss, HF ground wave and NVIS, satellite and microwave links, acoustic TL, link budgets |
| `bgws-gpu` | LOS, viewsheds, candidate pairs, path-loss and J/S fields, flow fields, Monte Carlo hits: CPU twins always, WGSL kernels behind `--features gpu` |
| `bgws-know` | Kalman tracks, GNN association, Bayesian classification with decoys, covariance-intersection COPs, DF fixes, `SideView` with truth kept behind an audit API |
| `bgws-behaviour` | the TypeScript template walk ported exactly (22 golden scenes from the TS walker), hard gates, validation, patches, safeguards |
| `bgws-scenario` | scope, the v3 loader, scenario v4, the v3 → v4 converter, game records; 14 presets in `data/scenarios/` |
| `bgws-models` | land physics and people: ACQUIRE/TTP, thermal, radar, ESM, sonar, signatures, direct and indirect fire, engineers, suppression, morale, fatigue, casualties |
| `bgws-domains` | munitions in flight (kinematic profiles, CEP), air (3-DoF, sorties), air defence (kill chains, WTA, magazines), naval (hulls, salvo, damage, mines), space (SGP4, passes, imagery, GNSS) |
| `bgws-ops` | comms nets, drones and counter-drone, EW, PNT, logistics (min-cost flow), maintenance, medical, C2 and orders, cyber, aggregated combat |
| `bgws-sim` | the ECS world, the multi-rate clock and event queue, the land loop, decisions, views, and one module per joint domain (`domains/`) |
| `bgws-decide` | the decider and planner traits; rules, Jev (OpenRouter), Claude (Messages API, tool schemas), local (OpenAI-compatible), recorded answers, a router, a person's seat |
| `bgws-analysis` | batches, vignettes with bands, KS and χ² tests, the regression store, Morris and Sobol sensitivity, the research backlog, meta-model fitting |
| `bgws-run` | the host loop, the `bgws` CLI, the vignette experiments |
| `apps/server` | `bgws-server` (a library and a binary): the game on its own thread, the viewer, editor and Behaviour Manager over HTTP and a WebSocket |
| `apps/desktop` | `bgws-desktop`: the Tauri 2 shell, the same host in-process and its pages in a native window |
| `web/viewer` | the viewer page: MapLibre + deck.gl when online, a plain canvas offline |
| `web/types`, `web/client` | TypeScript types generated from Rust (`ts-rs`) and a typed client for the server |

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
| 10 People and tools | A person's HQ seat and plans (server); the viewer's timeline scrubber and "what if" branch, layers for sensor footprints, viewsheds, jamming, the nets and kill chains in progress, a spectrum view, the per-unit *Why?* inspector, a detection inspector (why a unit sees another or not, with every factor), a link-budget inspector (each message hop by hop: queueing, airtime, attempts, SINR, fallbacks) and the after-action review (also `bgws aar`, `bgws why`, `bgws branch`); theatre packs (`bgws pack`); a scenario editor (`/editor`: theatre, time, weather, forces from the catalogue placed on a map, checked, saved or loaded into the game); the Behaviour Manager (`/behaviour`: graph views, and variants of any template made, checked and saved to `data/templates/custom/`, which the editor assigns to forces); the analysis workbench (`--html`, `bgws compare`). The full React screens are not yet ported. |
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
  `--backlog detect-to-strike-networked`, so it is reported but does not fail.
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
- **The React screens** play on the Rust engine through `RustRunner`
  (`apps/bgws/realtime/engine/rustRunner.ts`), the screens' `RunnerHandle` over
  `bgws-server`'s WebSocket: tick "Play in the Rust engine" under Advanced:
  engine. The screen's scenario is loaded into the server, the screen runs the
  clock, and frames and events are written onto its own state and log, so its
  map, feed, overlays and HQ console work unchanged. Differences: the server's
  deciders decide (not the screen's Jev or commander), ground is generated
  there, Rust-only things (drones in flight, the physics views) are on the
  server's own viewer, and an engage or wait order becomes overwatch. The
  screen takes a side's HQ seat when it gives that side orders.
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

## Game records

A record (`--record`, or *Save* on the server) holds the scenario, seed,
build, machine, the run setup (flat ground, start-of-game planning), every
figure set away from its default, every model answer, and every plan and
seat change a person made. Each command carries its position in the answers,
so a replay applies it at the same simulated time and the same point in the
game. A record saved mid-game says where it stops (`endedAt`).
`bgws replay` checks the replay against the record's fingerprint.

## Tests and CI

```sh
cargo test --workspace --exclude bgws-desktop     # unit, property, golden, determinism, vignette and integration tests
cargo clippy --workspace --exclude bgws-desktop --all-targets -- -D warnings
cargo test -p bgws-gpu --features gpu   # GPU twin tests (skip without an adapter)
target/release/bgws vignettes -n 200 --store data/regression   # §15.8: fails on out-of-band or a shifted distribution
```

`.github/workflows/rust-engine.yml` runs all of this on every change to `bgws-rs/`.
Fifteen benchmark vignettes run against stored baselines: fourteen are in band,
and networked detect-to-strike is on the calibration backlog. Batches and
vignettes answer with the rules and the decider crate's heuristic planner, the
same planner the host falls back to.
