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
| `web/app` | The viewer (React, MapLibre, deck.gl, milsymbol): its build in `web/app/dist` is committed and embedded in `bgws-server` |
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
| 10 People and tools | A person's HQ seat and plans (server); the viewer's timeline scrubber and "what if" branch, layers for sensor footprints, viewsheds, jamming, the nets and kill chains in progress, a spectrum view, the per-unit *Why?* inspector, a detection inspector (why a unit sees another or not, with every factor), a link-budget inspector (each message hop by hop: queueing, airtime, attempts, SINR, fallbacks) and the after-action review (also `bgws aar`, `bgws why`, `bgws branch`); theatre packs (`bgws pack`); a scenario editor (`/editor`: theatre, time, weather, forces from the catalogue placed on a map, checked, saved or loaded into the game); the Behaviour Manager (`/behaviour`: graph views, and variants of any template made, checked and saved to `data/templates/custom/`, which the editor assigns to forces); the analysis workbench (`--html`, `bgws compare`). The React screens play on the engine through `RustRunner` (see below). |
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

## The viewer

`bgws-server` (and `bgws-desktop`) serve the viewer at `/`, laid out like the
TS real-time screen:

- **Left pane:** the game, and who decides for each side, with a warning when
  their API key is missing. The clock, with speeds, a timeline to look back,
  and a branch from any moment. Views (umpire, or one side's eyes), map layers
  and basemap (dark, streets, satellite). The forces, with strength and state.
  A map key.
- **Map:** units as MIL-STD-2525C symbols, with rings for suppressed, pinned
  and broken. Shots by outcome, shell impacts, drone strikes and sightings, as
  they happen. Kill chains, enemy tracks, sensor footprints, jammers and the
  nets. The selected unit's route.
- **Right pane:** the decisions waiting for you, when you hold a side
  (`--person`), with the rules' pick and the default marked. A feed you can
  filter, where a decision opens on its options and who chose. The selected
  unit's orders and phases, people, weapons and what it knows, with *Why?*,
  *Messages*, *Viewshed* and *Can it see…?*
- **Elsewhere:** the HQ console (a side's seat, an objective and a task per
  unit), the spectrum strip, the after-action review, the end-of-game card,
  and *New game…* (any shipped scenario).

Only the map tiles come from the internet. `/classic` is the earlier
single-file viewer.

To change the viewer: `cd web/app && npm install`. Then `npm run dev` serves
it at http://localhost:5173 against a running `bgws-server` (port 8765).
`npm run build` writes `web/app/dist`, which the server embeds; commit it. CI
fails if it is stale.

## Real ground: theatre packs from PMTiles

`bgws terrain <dir> --bbox W,S,E,N` (or *New game… → Generate ground from
the map's current view* in the viewer) builds a theatre pack for a box. It
reads only that box's tiles from two planet PMTiles archives, by HTTP range,
and keeps them in a disk cache (`$BGWS_CACHE`, else `~/.cache/bgws/pmtiles` or
`~/Library/Caches/bgws/pmtiles`):

- **Elevation:** Mapterhorn's terrarium planet (`download.mapterhorn.com`,
  Copernicus GLO-30 plus national models), zoom 12, sampled onto the engine's
  grid (30 m by default, `--cell-m`). Copernicus is a surface model that
  includes trees, and the engine models canopy itself, so half the canopy
  height comes off in woods (`--dsm-canopy-fraction`, 0 to keep it). Water
  sits at its surface, not the lagoon floor.
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

**Woods a vehicle cannot enter.** A vehicle standing in ground it cannot
cross (a dense wood, never water) crawls out at `sim.route.crawl_kmh`. A
route may crawl only within `sim.route.escape_m` of its start. A unit sent
into such a wood goes to its edge (the closest approach).

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
- exposure from believed threats' viewsheds;
- mines, craters and blown bridges cutting the road graph;
- columns on roads and formations off them;
- calibration of road march rates and of where units leave the road.

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

## Tests and CI

```sh
cargo test --workspace --exclude bgws-desktop     # unit, property, golden, determinism, vignette and integration tests
cargo clippy --workspace --exclude bgws-desktop --all-targets -- -D warnings
cargo test -p bgws-gpu --features gpu   # GPU twin tests (skip without an adapter)
target/release/bgws vignettes -n 200 --store data/regression   # §15.8: fails on out-of-band or a shifted distribution
```

`.github/workflows/rust-engine.yml` runs all of this on every change to `bgws-rs/`,
and type-checks and rebuilds the viewer (`web/app`) to check its committed build.
Sixteen benchmark vignettes run against stored baselines: fourteen are in band,
and networked detect-to-strike and the attack halt are on the calibration backlog. Batches and
vignettes answer with the rules and the decider crate's heuristic planner, the
same planner the host falls back to.
