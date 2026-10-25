# BGWS in Rust: a local, GPU-assisted, physics-first simulation

**Status:** plan, October 2026. **Scope:** a new, separate engine and desktop app, written in Rust. The simulation runs locally and uses the GPU where that helps. Jev and Claude stay decision makers, **called remotely at first** (Jev through OpenRouter, Claude through the Anthropic API); running a model locally is a later option behind the same interface (§10.5). The TypeScript engine is not replaced in place, and exact replay parity with it is not a goal.

**How to read this:**

| Section | What it covers |
|---|---|
| §1 | The goal, and what "most realistic" means here |
| §2 | Design rules |
| §3 | Architecture: the crates, the sans-IO core, the host loop |
| §4 | Technology choices, and what was rejected |
| §5 | The data model: entities, components, coordinates, identifiers |
| §6 | Time: multi-rate stepping, events, compression, level of detail |
| §7 | The environment: terrain, sea, atmosphere, light, the EM spectrum |
| §8 | The physical and human models, domain by domain (the core of the plan) |
| §9 | Knowledge: tracks, fusion, deception |
| §10 | Decisions: templates, gates, Jev, Claude, people |
| §11 | The GPU: kernels, data layout, fallbacks |
| §12 | Determinism and replay |
| §13 | Performance targets |
| §14 | Data: catalogue, terrain, weather, formats |
| §15 | Calibration and validation |
| §16 | UI and tools |
| §17 | Testing |
| §18 | Roadmap: phases and "done when" |
| §19 | What carries over from the TypeScript build |
| §20 | Risks and open questions |

Evidence is cited as in the multi-domain plan: **[ref R4.3, P12]** points to `research/MODERN_WARFARE_REFERENCE.md`. **[default]** marks a value or method chosen without a source, awaiting one.

---

## 1. The goal

A person on one workstation, with one or two GPUs, can:

- play or watch a fight from a platoon skirmish to a joint theatre campaign of several weeks;
- have units led by Jev at their decision points and HQs planned by Claude, or by a person at any HQ;
- run thousands of seeded games overnight and get distributions, sensitivities and AARs;
- trust that what happens comes from modelled causes (physics, signatures, networks, depletion, people) rather than from tuned outcome tables.

### 1.1 What "most realistic" means

The research is clear about what matters, and it isn't platform statistics **[ref Conclusion]**:

1. **Latency through a contested network.** Detect-to-strike is 2–5 minutes in a networked force and about 20 without one **[P2]**. Who knows what, when, through which bearer, decides fights.
2. **Signature.** Every action (moving, firing, transmitting, radiating) makes a unit findable. Staying hidden, staying silent and waiting are often the best moves **[ref R4.1]**.
3. **Depletion.** Interceptors, shells, fuel, batteries, drones, crews and sleep all run down, and cheap-versus-expensive exchange sets how long an operation lasts **[ref R4.4, R6.2, R6.6, R7]**.
4. **People.** Units break well below most game thresholds: an attack halts at about 20% casualties **[P14]**. Suppression, fatigue, skill and leadership change everything downstream.

So the engine is **physics-first where physics decides** (sensing, propagation, ballistics, flyout, link budgets), **process-first where organisation decides** (C2, logistics, medical, kill chains), and **stochastic everywhere**, with every figure declared, sourced or marked **[default]**, and testable for sensitivity.

### 1.2 Non-goals

- Exact replay parity with the TypeScript engine (both engines are deterministic on their own; see §12).
- Classified or real-world-exact performance data. The catalogue stays synthetic and open-source-anchored (§14).
- A first-person or 3D visual simulation. The map is the product. 3D views are a later option (§16).
- Predicting real outcomes. The claim is plausible, explainable, calibrated behaviour that can be checked against benchmarks **[ref R8: SAS-139]**.

---

## 2. Design rules

1. **Truth and belief are separate types.** Nothing that decides (a unit, an HQ, Jev, Claude, a person) can read ground truth. The compiler enforces this: decision code takes a `SideView`, never `&World`.
2. **Time-continuous where possible.** Detection, failures, morale breaks and similar are hazard rates: P = 1 − e^(−λΔt). Results don't depend on the step size **[ref R8]**.
3. **Every number is a declared figure,** with a unit, range, source, confidence and sensitivity, in one registry. No literals in models. A test fails if a figure is unused, or used outside its range.
4. **Models are components, behaviour is data.** Platforms are built from movers, sensors, weapons, radios, emitters and stores (AFSIM-style). Behaviour is editable templates (DECISION_FLOW_GRAPHS.md, same JSON).
5. **Hard gates outside the models.** Rules of engagement, weapon release authority, EMCON and capability are never left to a model's judgement **[ref R8]**.
6. **Multi-resolution by design.** Entity resolution near contact, aggregate resolution away from it. The aggregate models are fitted to entity-level runs, not hand-set (§8.13).
7. **The core is pure.** The simulation never touches the network, the disk or a clock. It returns questions and takes answers (§3.2).
8. **Deterministic for a given seed, build and machine** (§12). Every run can be replayed and bisected.
9. **The GPU narrows, the CPU decides.** GPU kernels produce fields and candidate sets. Dice rolls and state changes happen on the CPU (§11).
10. **Explainable.** Every hit, miss, detection, message and decision can say why, with the figures it used, for the AAR.

---

## 3. Architecture

### 3.1 The workspace

```
bgws-rs/
  crates/
    bgws-core/        ids, time, geodesy (WGS84, ECEF, ENU), units of measure, figures registry,
                      keyed RNG, error types, the event log and fingerprint
    bgws-catalogue/   platform records: movers, bodies, signatures, sensors, weapons, munitions,
                      radios, emitters, stores; validation; archetypes
    bgws-env/         terrain tiles, land cover, soil, roads/rail/rivers, bathymetry, sea state,
                      weather fields, sun and moon ephemeris, atmospheric transmission
    bgws-prop/        propagation: optical LOS, radar horizon and masking, radio path loss
                      (free space, diffraction, ITM-style), HF skywave, acoustic transmission loss
    bgws-sim/         the World (ECS), the scheduler, every domain's systems, the sans-IO API
    bgws-know/        tracks, Kalman filters, association, classification, fusion, the side views
    bgws-behaviour/   templates, patches, inheritance, the graph walk, facts, action library
    bgws-decide/      decider traits; the rules decider; Jev, local-model and Claude clients
    bgws-gpu/         wgpu device, WGSL kernels, CPU twins of each, the kernel test harness
    bgws-scenario/    scenario, scope, game record and template formats (serde + JSON Schema)
    bgws-analysis/    batches, vignettes, distributions, sensitivity (Morris, Sobol), meta-models
    bgws-run/         CLI: play, replay, batch, vignettes, sensitivity, bench, export
  apps/
    desktop/          Tauri shell: the React UI, the Rust host loop in-process
    server/           (later) an authoritative host over WebSocket for multiplayer
  web/                the React app (from the TypeScript repo's screens), types generated from Rust
  data/               catalogue JSON, behaviour templates, vignettes, sample scenarios
  tools/              terrain tiling, catalogue checks, figure-report generation
```

Dependencies only point down: `core` ← `catalogue`, `env`, `prop` ← `know`, `behaviour` ← `sim` ← `decide`, `analysis` ← `run`, `desktop`. `bgws-gpu` is used by `prop`, `know` and `sim` through traits, so everything also builds and runs without a GPU.

### 3.2 The sans-IO core

```rust
pub struct Sim { /* World, scheduler, figures, RNG root, event log */ }

pub enum Step {
    /// The clock reached `until`. Nothing is waiting.
    Advanced,
    /// The clock stopped at `at`: these decisions are due. Answer them, then advance again.
    NeedsAnswers { at: SimTime, questions: Vec<Question> },
    /// The game ended: victory, time limit, or a side broke.
    Over(Outcome),
}

impl Sim {
    pub fn new(scenario: &Scenario, seed: Seed, figures: Figures) -> Result<Sim, LoadError>;
    pub fn advance(&mut self, until: SimTime) -> Step;
    pub fn answer(&mut self, answers: Vec<Answer>) -> Result<(), AnswerError>;
    pub fn command(&mut self, seat: SeatId, orders: CommanderOrders) -> Result<(), CommandError>;
    pub fn set_seat(&mut self, hq: HqId, seat: SeatKind);
    pub fn view(&self, viewer: Viewer) -> FrameView;        // truth for the umpire, else a side's or an HQ's view
    pub fn side_view(&self, side: Side) -> SideView<'_>;    // what deciders may read
    pub fn drain_events(&mut self) -> Vec<LogEvent>;
    pub fn snapshot(&self) -> Snapshot;                      // for replay scrubbing and save games
}
```

A **question** is a decision point that has fired for one agent: a unit (land decisions), an entity (world decisions) or an HQ (a plan request). It carries:
- the agent's options, already masked by capability, gates and the template;
- the template's fallback;
- the agent's reaction time, which sets when the answer takes effect.

The clock does not wait in wall time. It stops in simulated time until the answers arrive, so a slow model never changes the game, only how long it takes to play. This is today's runner rule, made explicit.

### 3.3 The host loop

```
            ┌───────────────────────── host (tokio) ─────────────────────────┐
  UI ◄────► │ wall clock × speed ─► sim.advance(until)                        │
 (Tauri     │                         │                                       │
  events)   │       Advanced ◄────────┤                                       │
            │       NeedsAnswers ─────┼─► batch by side ─► Jev (OpenRouter)   │
            │                         │                  ─► Claude planners   │
            │                         │                  ─► a person's seat   │
            │       sim.answer() ◄────┘◄── answers (or fallback on timeout)   │
            │  drain_events ─► log store (DuckDB/JSONL) ─► UI feed, AAR       │
            └─────────────────────────────────────────────────────────────────┘
```

- The sim runs on a dedicated thread with a `rayon` pool for its parallel systems. The host's async runtime is separate, so model calls never block a tick.
- Model calls for one moment are batched by side and sent concurrently. A per-question timeout falls back to the template's fallback, and the record says so.
- The same core runs under four hosts: the desktop app, the CLI (with no models, or recorded answers), the batch runner (rules deciders, many sims in parallel) and later the multiplayer server.

---

## 4. Technology choices

| Need | Choice | Why | Rejected |
|---|---|---|---|
| Language | Rust (stable, 2021+ edition) | Memory layout, threads without data races, one language from GPU host to CLI | C++ (safety, build); Go (GC, SIMD); Julia (deployment) |
| Entity storage | `bevy_ecs` as a library, without Bevy's renderer or app | Archetype tables, change detection, a scheduler that orders and parallelises systems | Hand-rolled struct-of-arrays (more code); `hecs` (no scheduler); `specs` (dormant) |
| Data parallelism | `rayon` | Work-stealing over entities inside a system | Manual threads |
| Async host | `tokio`, `reqwest` | Concurrent model calls, timeouts | — |
| Linear algebra | `glam` (f64 `DVec3`) for kinematics; `nalgebra` for filters | Fast small vectors; matrices for Kalman | One library for both (worse at one) |
| Geodesy | Own WGS84/ECEF/ENU module; `geo` for polygons | Small, exact, testable; no C dependencies | PROJ (C library, heavy) |
| Spatial index | Uniform hash grid per layer (land, air, sea); `rstar` for static features | Grids are best for moving points; R-tree for roads, obstacles, areas | Quadtrees (rebuild cost) |
| Graphs | `petgraph` | Road, rail and sea networks; comms meshes; command trees | — |
| Routing | A* and contraction hierarchies on road graphs; cost-field A* on rasters; GPU flow fields for many movers | Standard and fast | — |
| Orbits | `sgp4` crate | Standard propagator from orbital elements | Hand-rolled two-body (not good enough) |
| Raster I/O | `tiff` (pure Rust) for GeoTIFF; own tile format for runtime | No GDAL at runtime; GDAL only in the offline tiling tool | GDAL in the app (C, deployment) |
| GPU | `wgpu` with WGSL | Vulkan, Metal, DX12 and WebGPU from one code base; the same shaders run in a browser build | CUDA (NVIDIA only, no browser); OpenCL (dated) |
| Serialisation | `serde`, `serde_json` for files; `postcard` or `bincode` for snapshots and the UI channel | JSON for people, compact binary for speed | Protobuf (extra toolchain) |
| Shared types with the UI | `ts-rs` or `specta` | TypeScript types generated from Rust, so the UI can't drift | Hand-written types |
| Statistics | `statrs`, `rand_distr` | Distributions, tests (KS, chi-squared) | — |
| Optimisation | `pathfinding::kuhn_munkres` for assignment; `good_lp` (pure-Rust solver backend) for LPs | Weapon-target assignment, logistics flows | — |
| RNG | `rand_chacha` keyed per stream (§12) | Quality, reproducible across threads | One global stream (order-dependent) |
| Storage | SQLite (`rusqlite`) for catalogue, templates, saves; DuckDB for event logs and AAR queries | Local, file-based, fast analytics | Foundry (not local); Postgres (server) |
| Desktop | Tauri 2 | Small, native, Rust in-process; hosts the React UI | Electron (size, Node in-process) |
| UI | React + MapLibre + deck.gl | The existing screens; deck.gl draws 100k+ entities on the GPU | Bevy/egui UI (rebuild every screen) |
| Model calls (now) | Remote: Jev through OpenRouter's Decisions API, Claude through the Anthropic Messages API, both with `reqwest` from the Rust host | No GPU memory or model serving to manage; Jev's typed answers (choice, probabilities, confidence) as today; keys stay in the host, never in the UI bundle | Local models from day one (more to build and tune before anything plays) |
| Model calls (later) | Optionally local: llama.cpp server or vLLM, OpenAI-compatible, grammar- or schema-constrained output (§10.5) | Offline play, no per-call cost, lower latency | In-process inference first (later still: `mistral.rs`, `candle`) |
| Profiling | `tracing` + `tracy-client`; `criterion`; `cargo flamegraph` | Per-system timing in real games; micro-benchmarks | — |
| Testing | `proptest`, `insta`, `approx`, `cargo-fuzz` | Properties, snapshots, float comparisons, loader fuzzing | — |

---

## 5. The data model

### 5.1 Entities and components

Every platform is an ECS entity built from components. The current TypeScript `Platform` (MULTI_DOMAIN_PLAN §4.1) maps one to one, with more physics in each part:

| Component | Holds | Notes |
|---|---|---|
| `Identity` | id, side, label, SIDC, archetype, echelon, parent | Stable string ids from the scenario, interned to `EntityId` |
| `Kinematics` | ECEF position, ENU velocity, attitude (heading, pitch, roll), altitude/depth | f64 positions; theatre-safe |
| `Mover` | mover kind and its performance record | ground, foot, wheeled, tracked, rotary, fixed-wing, missile, surface, sub, orbit, static |
| `Body` | sub-platforms (vehicles, aircraft, people) with component damage | Each sub-platform tracks mobility, firepower, sensors, comms, crew and catastrophic state |
| `Signatures` | visual size and contrast, IR (with a thermal state), RCS by aspect and band, acoustic by speed, RF from emitters | Change with posture, movement, firing, heat, camouflage |
| `Sensors` | each sensor's kind, band, aperture, field of regard, scan pattern, modes, current task | |
| `Weapons` | mounts, with fire control, magazines, reload, ready time | |
| `Comms` | radios: bearer, band, power, antenna height, nets, crypto, encryption delay | |
| `Emitters` | radars, radios, jammers, beacons: power, band, duty cycle, on/off | EMCON switches them |
| `Stores` | fuel, ammunition by type, batteries, spares, water, food, medical, carried drones and troops | |
| `Crew` | skill, experience, suppression, stress, cohesion, fatigue, sleep debt, casualties | §8.10 |
| `Orders` | structured orders (task, purpose, phases, control measures, ROE, actions on contact) | C2SIM-shaped **[ref R8]** |
| `Activity` | what it is doing this second | as today's `RtOrder`, generalised |
| `Awareness` | its own sightings, what it has been told (by reference into the side's track store), its self-belief | §9 |
| `Lod` | entity, aggregated (into which formation), far, rules-only | §6.4 |
| `Hq` | for command posts: COP, inbox, staff, battle rhythm, seat | |

Systems ask for exactly the components they need, so the scheduler can run the sensing of air entities in parallel with the logistics of convoys.

### 5.2 Coordinates

- **Storage:** WGS84 geodetic for files, ECEF f64 in memory for everything that moves. That's exact from a platoon fight to a theatre.
- **Computation:** each system works in a local ENU tangent frame for its neighbourhood: a grid cell, an engagement, a sensor's footprint. A 1 km error over 1,000 km doesn't happen, and flat-earth maths is used only within a few tens of kilometres.
- **Altitude:** above the WGS84 ellipsoid internally, converted to above-ground or above-sea-level through the geoid model and the terrain.
- **Earth curvature and refraction** are in every line-of-sight and propagation call: the 4/3-Earth model for radar and radio, the optical refraction coefficient for sight **[default]**.

### 5.3 Identifiers and units

- `EntityId`, `SideId`, `HqId`, `TrackId`, `MessageId`: newtypes, never bare integers.
- **Units of measure:** newtypes for metres, seconds, radians, watts, decibels and hertz, with explicit conversions. The figures registry stores the unit with each figure, and loading checks it.

---

## 6. Time

### 6.1 Multi-rate stepping

One master clock in integer microseconds, with each system running at its own rate:

| Rate | Systems |
|---|---|
| 20–50 Hz sub-steps, only while something fast is in flight | missile and munition flyout, terminal guidance, CIWS, closest point of approach for intercepts and hits |
| 10 Hz | aircraft and drone flight, air-to-air, radar scans for fast tracks |
| 1 Hz (base) | ground movement, sensing, tracking, direct fire, suppression, comms delivery, decisions |
| 0.1 Hz | ships, submarines, sonar, HQ staff work, fusion cycles |
| 1/60 Hz | logistics flows, maintenance, medical, fatigue, weather fields |
| Event-driven | satellite passes, cyber windows, scenario events, arrivals, scheduled resupply, battle-rhythm meetings |

Fast sub-steps run only for entities that need them: an empty sky costs nothing.

### 6.2 The event queue

A binary heap keyed `(time, priority class, stable key)`. Systems schedule follow-ups (a reload completes, a message arrives, a pass begins, a convoy reaches a node) instead of polling. Ties never depend on insertion order or hash order (§12).

### 6.3 Time compression and quiet skipping

- **Interactive** speed runs from ×1 to the scope's maximum (×1,000 for campaigns).
- **Quiet skipping:** when there is no contact, nothing in flight and no question waiting, the host may jump straight to the next scheduled event or the next point where a sensor could detect something (from the GPU coverage fields, §11). A ten-day campaign spends most of its time in jumps.
- Compression and skipping only change how fast the clock runs, never what happens.

### 6.4 Level of detail

| Level | When | How it moves and fights |
|---|---|---|
| Entity | Near contact, in a sensor's view, or focused on by a player or planner | Every model in §8 |
| Aggregated | A formation more than a set distance from any enemy track | One formation mover; attrition and consumption by fitted rates (§8.13); members' state carried as distributions |
| Far | Rear areas, deep logistics | Network-flow logistics only; no tactical sensing |
| Rules-only | Any level, by scope | The rules decider instead of Jev |

**Disaggregation** recreates the entities from the formation's distributions: strength, stores and fatigue are sampled consistently, so killed vehicles stay killed. **Aggregation** folds them back. Both happen at system boundaries, never mid-engagement.

---

## 7. The environment

### 7.1 Terrain

- **Elevation:** Copernicus GLO-30 (30 m), tiled into a pyramid at 30, 90, 270 and 810 m. Tactical areas can add 1–10 m local DEMs.
- **Land cover:** ESA WorldCover (10 m) into the engine's classes: open, crops, scrub, forest by density, urban by density, water, wetland, bare.
- **Soil and trafficability:** soil type and wetness, wetness rising with rain and falling with time (§7.3). Trafficability depends on vehicle class: mud season stops wheels long before tracks.
- **Networks:** OpenStreetMap roads (by class, with bridges and their load class), rail, rivers with width and depth for fording, power lines (rotary hazard), urban blocks.
- **Micro-terrain:** cover and concealment per 30 m cell as distributions (hedgerows, ditches, rubble), so a unit in a forest edge has cover a unit in the open doesn't, without modelling every tree.
- **Change:** craters, rubble, burnt forest, blown bridges, minefields and obstacles change the cells they touch, and are seen only by those who observe or are told about them.

### 7.2 The sea

- GEBCO bathymetry; coastlines from OSM; ports with berths, depth and throughput.
- **Sea state** as a field over time (wind-driven), affecting speed, small craft, USVs, sonar noise, radar clutter and helicopter operations.
- **Sound-speed profile** per region and season: a mixed layer, a thermocline and a deep sound channel, from climatology **[default: simplified profiles]**. These drive sonar (§8.6).
- Currents only where they matter (straits, approaches) **[default: off]**.

### 7.3 The atmosphere and weather

- **Weather as fields over time,** not one global state: a coarse grid (about 10–25 km) of visibility, cloud base and cover, precipitation, wind at three levels, temperature, humidity, and fog.
- **Sources:** a scenario's preset; a synthetic generator (fronts moving through, diurnal fog, seasonal climatology); or, for historical dates, ERA5 reanalysis imported offline.
- **Visibility to extinction:** Koschmieder, σ = 3.912 / V. Optical and IR transmission over a path is Beer–Lambert, τ = e^(−σR), with band-specific coefficients for visible, MWIR and LWIR, plus rain and fog attenuation for radar bands (ITU-R P.838 form) **[default coefficients]**.
- **Wind** acts on drones, parachutes, artillery (with meteorological data age as an error source), smoke drift and sea state.

### 7.4 Light

- **Sun and moon positions** computed from the scenario's real date, time and place (NOAA solar position algorithm; a standard lunar ephemeris). This gives actual dawn, dusk, twilight phases and moon illumination.
- **Illuminance** sets eye and image-intensifier performance: full moon is not new moon, and overcast changes both.
- **Thermal state:** a diurnal heat model per land-cover class and per vehicle state (engine on, recently moved, recently fired, cold-soaked). This gives real thermal crossover times instead of a fixed dawn and dusk window **[ref R4.1]**.

### 7.5 The electromagnetic spectrum

- **Every emitter is a source in a spectrum model:** frequency, bandwidth, power (ERP), antenna pattern, duty cycle and position.
- **Receivers compute signal, noise and interference:** thermal noise plus every jammer and co-channel emitter in band, through the same propagation models (§8.4). Comms, radar, GNSS and ESM all use the same model.
- **GPU fields** (§11) keep jamming and coverage maps current for the UI and for quiet skipping. Exact per-link values are computed on the CPU at the receivers that matter.

---

## 8. The models, domain by domain

Each model lists its method, its main inputs, its sources, and what it costs. Exact formulas and figures live in the figures registry and the model's own doc comments.

### 8.1 Movement

**Ground:**
- **Speed** = min(the vehicle's road speed, the trafficability speed for this cell, the formation's speed, the order's mode). Trafficability follows the NATO Reference Mobility Model's approach: slope, soil strength (from soil type and wetness), vegetation resistance and obstacle spacing, by vehicle class (tracked, wheeled, foot) **[default: simplified NRMM-style curves]**.
- **Routes:** the road graph for march, the cost raster for tactical movement. Cost includes exposure: a route through observed open ground costs more when the unit believes it is watched.
- **Formations and bounding:** sections move in formation with spacing. Bounding overwatch alternates movers and watchers, each with its own posture and signature.
- **Fuel burn** by vehicle, speed, terrain resistance and idling **[P17]**; dismounted pace by load, slope and fatigue **[P26]**.
- **Obstacles and crossings:** fording by depth and bank slope; bridging by load class; breaching as a process (§8.9).

**Air:**
- **A point-mass, three-degree-of-freedom energy model:** thrust, drag, lift limits, a turn-rate envelope, and climb and descent rates from the aircraft's performance record. Fuel flow by altitude and throttle.
- **Flight paths** follow waypoints, orbits, intercepts and terrain following (minimum clearance over the DEM, which affects radar masking).
- **Drones:** multirotors with battery endurance by wind and payload; fixed-wing with glide. Fibre-optic drones are tethered by spool length **[P8]**. RF drones need a link (§8.5) or fall back to autonomy (§8.7).

**Missiles and munitions:**
- **Three-degree-of-freedom flyout:** motor burn and coast, drag by Mach and altitude, maximum g by speed.
- **Guidance laws:** proportional navigation for homing, command guidance, inertial plus GNSS for cruise and ballistic, and a terminal seeker phase with its own sensor (§8.2).
- **Ballistic missiles:** a boost phase, a midcourse arc with Earth rotation, and re-entry. Intercept windows come from the geometry, not a table.
- **Closest point of approach** within each sub-step, so nothing passes through its target.

**Sea:**
- **Surface ships:** speed limited by sea state and hull class; turning circles; fuel by speed (a cubic law).
- **Submarines:** depth, speed and radiated noise by speed, plus snorting and battery for diesel-electric boats.
- **Small craft and USVs:** limited by sea state **[ref R6.3]**.

**Space:**
- **Satellites** are propagated with SGP4 from orbital elements; manoeuvres are delta-v events.
- **Ground tracks, access windows and sensor footprints** are computed per pass, analytically, and scheduled as events.

### 8.2 Sensing

Every sensor works the same way: **can it see** (geometry, masking, field of regard, range limits), **how strongly** (a signal-to-noise or resolved-cycles number), and **how fast it detects** (a hazard rate from that number). Detection is a time-continuous hazard, then identification and classification follow as separate stages.

**Eye, optics, image intensifiers and EO cameras: ACQUIRE / TTP.**
- Resolved cycles across the target: N = (critical dimension / range) × sensor resolution × atmospheric MTF × contrast factor.
- **Contrast** from target-background luminance (camouflage, posture, background class, light level from §7.4) with transmission from §7.3.
- P∞ = (N/N₅₀)^E / (1 + (N/N₅₀)^E), with E = 2.7 + 0.7·(N/N₅₀) **[ref R8, P22]**, and separate N₅₀ values for detect, recognise and identify. TTP is used for modern sensors, mapped at about 1:2.7 **[ref R8]**.
- **A hazard rate** λ = −ln(1 − P∞)/τ, where τ is a search time constant set by field of view, cueing (a sensor told where to look searches a small area) and the observer's workload and fatigue **[default τ]**.

**Thermal and IR sensors and IRST: the same machinery with thermal contrast.** ΔT comes from the target's thermal state against the background's (§7.4), through the band's transmission. A cold, hull-down tank at crossover is nearly invisible. A tank that has just fired is not.

**Radar: the radar range equation.**
- SNR = Pt G² λ² σ / ((4π)³ R⁴ k T B F L), against detection thresholds with Swerling fluctuation models.
- **RCS** by aspect and band from the catalogue, plus clutter: ground clutter by land cover and grazing angle, sea clutter by sea state.
- **Doppler:** moving-target indication and the notch when the target flies tangentially, which pilots exploit **[ref R8: CMO]**.
- **Masking:** terrain and the radar horizon (4/3-Earth) for low flyers; multipath over water.
- **Scan:** revisit time and dwell per track, with scan and track modes for fire-control radars, and an emission signature in every mode.

**ESM, RWR and SIGINT: received power against sensitivity.**
- An emitter is detected when the received power (through §8.4 propagation) exceeds the receiver's sensitivity in band, weighted by duty cycle and the scan illuminating the receiver.
- **Direction finding:** each fix is a bearing with error from baseline and SNR. Two or more fixes give a location with an error ellipse, by triangulation.
- **Time to fix and time to strike** are open variables **[ref R8: DF-to-strike unsourced, P32]**, so they are in the registry with wide ranges and on the sensitivity report.

**Acoustic and sonar: the sonar equations.**
- **Passive:** SE = SL − TL − (NL − DI) − DT.
- **Active:** SE = SL − 2TL + TS − (NL − DI) − DT, or the reverberation-limited form.
- **Transmission loss:** spherical then cylindrical spreading plus absorption by frequency, with layer effects from the sound-speed profile: shadow zones below the layer, surface ducts, and convergence zones at about 30–60 km where the profile supports them **[default: simplified ray-free CZ model]**.
- **Noise:** shipping, sea state and own-ship self-noise by speed.
- **Battlefield acoustic sensors** detect gunfire, drones and vehicles by the same equations in air, with excess attenuation and wind.

**SAR and EO satellites:** they image a footprint during an access window, when the cloud allows (EO) or always (SAR). Images are delivered after the downlink latency and the analysis queue (§8.11), and identification depends on the image's resolution against the target's N₅₀.

**Human observers:** search with attention limits. A crew under fire, suppressed or fatigued searches badly (lower τ⁻¹). A crew on overwatch with a sector searches well.

**Cost:** sensing is the largest per-tick job. It is done in three steps:
1. The GPU produces candidate observer–target pairs that are in range, inside the field of regard and with line of sight (§11).
2. The CPU computes the hazards for those pairs in parallel.
3. The CPU rolls the dice, using keyed streams (§12).

### 8.3 Signatures and deception

- **Signatures change with action:** moving raises visual and acoustic signature and creates dust in dry weather; firing creates muzzle flash, a thermal bloom and acoustic and radar-trackable shells; transmitting creates an RF signature; running engines raise IR.
- **Camouflage and concealment** reduce contrast. Multispectral nets reduce visual and IR contrast, with deployment time and a movement penalty.
- **Decoys:** inflatable and thermal decoys, emitter decoys and corner reflectors with signatures in chosen bands. They draw fire and consume munitions, and are classified as real until an observer resolves them.
- **EMCON:** emitters off means no RF signature, but also no radar picture. The tradeoff is the point **[ref R4.4]**.
- **Smoke:** obscurant clouds drift with the wind and block visual and some IR bands, by type.

### 8.4 Propagation

| Path | Model |
|---|---|
| Optical line of sight | Ray march over the DEM with Earth curvature and refraction; partial masking from canopy and urban cells as a probability |
| Radar and radio horizon | 4/3-Earth effective radius |
| VHF/UHF radio path loss | Free space + terrain diffraction (multiple knife-edge, ITU-R P.526 style) + clutter loss by land cover; an ITM/Longley–Rice-style irregular-terrain model at range **[default: simplified implementation, checked against published ITM curves]** |
| HF | Ground wave by frequency and soil; NVIS skywave for 2–10 MHz, about 35–450 km, with time-of-day ionospheric availability **[P5]** |
| Satellite links | Free space + rain fade by band + blockage by terrain at low elevation |
| Microwave and mmWave links | Line of sight + rain attenuation |
| Acoustic | §8.2 |

Every path loss feeds the same receiver budget: signal, noise, interference and jamming give an SNR, then a bit error rate for the waveform, then message success and throughput.

### 8.5 Communications and networks

- **Bearers** are radios with real budgets (§8.4). SINCGARS-like VHF at about 0.2–40 km by power **[P4]**; HF; satcom (GEO and LEO, with latency **[P7]**); tactical data links with TDMA slots (Link 16-like: 7.8125 ms slots, 12.8 min epoch, 31.6–115.2 kbps **[P6]**); line-of-sight mesh radios; field wire and fibre; and couriers.
- **Messages have sizes:** a voice contact report, a data-link track, an image, a plan. A link has throughput, and a net has a queue and a channel-access scheme (push-to-talk contention on voice; slot assignment on TDMA; random access on mesh). So a busy net delays messages, and a jammed one loses them.
- **Mesh routing:** messages hop across a mesh by a routing table that degrades as links fail, with hop latency. HQs relay between nets.
- **PACE:** each message type has a primary, alternate, contingency and emergency bearer, and the sender falls back when one fails, as today.
- **Signature:** every transmission is an RF event that ESM can detect, locate and attack (§8.2, §8.5 ↔ §8.7).
- **Encryption and authentication:** a delay per message by crypto class, and keys that can be compromised (a cyber effect, §8.12).

### 8.6 Naval and undersea

- **Detection** by radar (with the sea-skimmer horizon), ESM, EO/IR, sonar (§8.2) and offboard sensors (helicopters, drones, satellites, maritime patrol).
- **Salvo combat emerges from flyout and layered defence** **[ref R6.1]**: each missile is an entity that is detected, tracked, assigned and engaged at each layer (long-range SAM, medium, point defence, CIWS, soft kill) **[P28]**, with magazines depleting **[ref R6.2]**.
- **Damage:** hits damage compartments and systems (radar, launchers, propulsion), cause fires and flooding with damage control as a process, and a mission kill comes before sinking **[default model]**.
- **Mines:** fields with types (contact, influence by signature: magnetic, acoustic, pressure), sweeping and hunting rates, and actuation probability per transit against the ship's signatures.
- **Ports and replenishment:** reloading VLS cells needs a port or a capable tender; underway replenishment of fuel and stores takes time.

### 8.7 Drones, counter-drone, EW and PNT

**Drones:**
- **Types:** FPV (RF or fibre), bomber multirotors, reconnaissance multirotors, fixed-wing ISR, loitering munitions, long-range one-way attack (Shahed-like) and interceptor drones **[ref R5.1, R5.2]**.
- **The drone fight is a link fight:** an RF drone needs control and video links through §8.4, against the jamming at both ends. Losing the link triggers its autonomy behaviour: return, hover, continue to the last aim point, or AI terminal guidance if fitted. Fibre drones ignore RF jamming but have spool and snag limits.
- **The FPV strike** is a sequence: find (operator searching with the drone's camera, §8.2), approach (exposed to counter-drone fire and jamming), terminal (a hit probability from operator skill, target motion, wind and link quality), then effect (§8.8). Overall hit rates should fall out at 30–50%, 10% for novices, with 60–80% failing to destroy **[P9]**; those are acceptance checks, not inputs.
- **Production, supply and operators:** drones and batteries are stores; operators are crew with skill and fatigue; launch sites have signatures.

**Counter-drone:**
- **Detection:** RF detectors, radar, acoustic and EO.
- **Defeat:** jammers (a J/S budget at the drone's receiver), guns and shotguns, interceptor drones, nets, directed energy where fitted, and passive measures (cages, netted roads) that change the terminal hit probability.

**EW:**
- **Jammers are emitters:** band, ERP, antenna pattern and duty cycle. The effect is computed at each receiver as J/S through the propagation model, so terrain shields and range matter, and burn-through ranges emerge.
- **Look-through and power limits:** a jammer can't jam every band at full power; it has a signature (§8.3) and is a high-value target.
- **Radar jamming:** noise and deception (range and velocity gate pull-off) against radar modes, with ECCM (frequency agility) where fitted **[ref R8: CMO]**.

**PNT:**
- **GNSS receivers compute C/N₀ against jamming;** below a threshold they lose lock. Spoofing shifts the solution.
- **INS drift** grows with time since the last good fix, by INS grade. Units know their own position with an error covariance that grows, and report positions with that error.
- **Precision munitions' CEP** follows from their navigation state at release and in flight. Excalibur-type Pk falling from >50% to <10% under jamming **[P15]** is an acceptance check. Jamming radii of ≥25 km per module **[P16]** emerge from the jammer power and the receivers.

### 8.8 Weapons and terminal effects

**Direct fire:**
- **The error budget:** each engagement has a fixed bias (range estimation, cant, crosswind, boresight) and each round has random dispersion. Hit probability is the bivariate normal integrated over the target's presented area at that aspect and posture (hull-down halves it). Bias is fixed per engagement, so misses are correlated, which is realistic, and the fire control system's quality shrinks the bias **[P27: Ph >70% at 2 km for MBTs, an acceptance check]**.
- **Time of flight** and target motion add lead error. A moving firer adds dispersion unless stabilised.
- **Penetration against armour:** the round's penetration by range against the target's armour by aspect, zone and type (with ERA and APS where fitted).
- **Kill types:** a hit gives mobility, firepower, catastrophic or crew effects by zone, from vulnerability tables **[default tables]**.
- **Small arms against people:** a suppression and casualty process by volume of fire, range, posture and cover, rather than individual bullets.

**Indirect fire:**
- **Ballistic trajectories** with charge, quadrant elevation and time of flight, plus meteorological error by met data age, survey error from the gun's PNT (§8.7) and dispersion by range.
- **Lethality:** a lethal area by round and fuze against target posture (standing, prone, in a trench, in a vehicle), using a Carleton-style damage function in the JMEM tradition **[default parameters]**.
- **Adjust fire** as a loop of observation, correction and the next round, through the comms (§8.5). Fire for effect follows.
- **Counter-battery:** weapon-locating radars detect shells and backtrack to the firing point with an error. Drones finish the job **[ref R4.3]**. Guns must displace within minutes **[P23]**.
- **Consumption:** shells per mission against stocks and resupply **[P12]**.

**Air-delivered and guided munitions:** release envelopes, seekers (laser, IR, radar, GNSS/INS) with their own detection and guidance (§8.1), and countermeasures (flares, chaff, decoys, jamming).

**Air defence:**
- **The kill chain per threat:** detect, track, classify, assign, launch, guide and assess, each with its time and probability, and each through the comms and EW models **[ref R3]**.
- **Weapon-target assignment** is an optimisation, not a rule. Assign interceptors to threats to maximise expected value defended minus cost, with shoot-look-shoot where time allows, using Kuhn–Munkres or a greedy marginal-return auction under cost pressure **[ref R4.4, P11, P29]**.
- **Interceptor flyout** is modelled (§8.1), so the kinematic no-escape zone emerges, rather than coming from a table.

### 8.9 Engineers and obstacles

- **Minefields** with type, density, depth (about 500 m **[P13]**) and marking. Encounter probability per crossing from density and path. Breaching by type (plough, roller, line charge, manual), with time per lane (about 6 m lanes, about 1.5 h **[P13]**) and exposure while doing it.
- **Obstacles:** ditches, dragon's teeth, wire and craters, each with a crossing time by vehicle class and an engineer reduction time.
- **Gap crossing:** bridging assets, launch times and load classes.
- **Field fortifications:** digging time by soil and tools, giving cover that changes the lethality tables (§8.8).

### 8.10 People: suppression, stress, cohesion, fatigue and casualties

- **Suppression** rises with incoming fire volume and proximity (near misses, not just hits), decays with time, and is reduced by cover. Effects: search slows, accuracy falls, movement stops when pinned **[P31: −50–90%]**.
- **Stress and cohesion:** a stress level per unit accumulates from casualties, suppression time, isolation (no comms, no friends nearby), surprise and being watched by drones. Leadership, training, cohesion and success reduce it. Above thresholds the unit's options change (the template's gates read these facts), and it may break or rout. The halt at about 20% casualties **[P14]** is an acceptance check that should emerge from these rates.
- **Fatigue and sleep:** sleep debt grows over 24–48 h of operations **[P31]**, slowing reactions, search and decisions, and raising errors. Rotation and rest recover it. This is what makes multi-day games realistic.
- **Skill and experience:** crews and operators have skill that sets the error budgets and hit chances, and experience grows with survived engagements **[default]**.
- **Reaction times** are distributions by drill and skill, not constants.
- **Casualties:** wounds by severity (triage categories), with time-dependent mortality without treatment. Evacuation through §8.11 decides survival, against delays of 60–120 minutes under drones **[ref R7]**.

### 8.11 Logistics, maintenance and medical

- **The supply network** is a graph of depots, railheads, ports, airfields and supply points with capacities, joined by road, rail, sea and air links with throughput and transit time. Flows are planned each cycle as a min-cost-flow problem and executed as convoys, trains, sorties and drone deliveries **[ref R7]**.
- **Consumption** comes from activity, not constant rates: fuel per km by terrain, per idle hour and per flight hour; ammunition per engagement; batteries per drone sortie; food and water per person per day.
- **Interdiction:** convoys are entities with signatures. Depots hit by strikes lose stock and throughput. Pulled-back hubs at about 120 km **[P18]** emerge when the attacker's reach makes them untenable.
- **The last mile:** drone and UGV delivery for kill-zone positions **[P19]**.
- **Maintenance:** failures by operating hours (MTBF by class), battle damage by component, recovery assets, repair times at each echelon, and spare parts as stores.
- **Medical:** the chain from point of injury to casualty collection point, aid post, surgical team and hospital, with ambulances and helicopters as entities with signatures and capacities, and triage queues.

### 8.12 Command, control, space and cyber

**C2 as processes with time:**
- **Planning time** per echelon, with the 1/3–2/3 rule **[P1, P25]**; hasty plans when a CCIR fires; battle rhythm events.
- **Staff capacity:** an HQ processes messages from a queue. Overloaded staff are slow and drop information.
- **HQs have signatures** (emitters, vehicles, generators) and survive by moving and dispersing. Losing one hands the seat to its alternate after a delay.
- **Orders** are structured (task, purpose, control measures, timings), C2SIM-shaped **[ref R8]**, and travel as messages (§8.5).

**Space:**
- **Satellites as entities:** SGP4 orbits; ISR passes with access windows, revisit and footprint; communications satellites with beams and capacity; and GNSS constellations as the PNT source.
- **Effects:** tasking queues and downlink latency for imagery; satcom capacity contention; jamming of uplinks and downlinks; dazzling; and ASAT as an event.
- **Political switches:** a commercial provider cuts service at a scenario time **[ref R5.4]**.

**Cyber:**
- **Access campaigns** are processes: months of access building **[P21]**, a probability of discovery, and a one-shot effect window of days.
- **Effects** are time-limited multipliers or outages on other domains' processes: an air-defence network loses track sharing; a logistics system loses its picture; a mobile network goes down; keys are compromised.
- No packet-level simulation **[ref Conclusion: modifiers, not mini-games]**.

### 8.13 Aggregated combat (the far and the rear)

- **Stochastic Lanchester-type attrition** between formations, with posture, terrain, supply and fatigue as modifiers.
- **The coefficients are fitted, not set.** For each pair of formation types, posture and terrain class, run thousands of entity-level vignettes (§15) and fit attrition, movement and consumption rates as distributions. The aggregate model is a meta-model of the entity model, so the two agree. JTLS and SWORD use aggregate attrition **[ref R8]**; fitting it to entity runs is this plan's addition.
- The fit is refreshed when the entity models change, and checked as part of calibration.

---

## 9. Knowledge: tracks, fusion and deception

### 9.1 Tracks

- **Each side has a track store** (the side picture of SIDE_PICTURE.md, generalised). Each track has a state estimate (position and velocity in ENU) with a covariance from a Kalman filter: constant-velocity or constant-turn for movers, near-static for ground positions.
- **Association:** new detections are gated by Mahalanobis distance and assigned by global nearest neighbour (Kuhn–Munkres). Ambiguous cases create a new track and may later merge or split. This is how false and duplicate tracks arise, realistically.
- **Classification** is a Bayesian belief over hypotheses (MBT, IFV, truck, decoy; fighter, bomber, drone, missile), updated by each sensor's identification probability and confusion matrix. A decoy stays a "probable MBT" until a good enough look.
- **Track age and quality:** covariance grows without updates, and the track is dropped when it is too uncertain to act on.

### 9.2 Who knows what

- **Units hold their own sightings** and references to side tracks they have been told about, with the time they heard. They know what reached them, not the whole side picture.
- **HQs fuse what reaches them** (by message, §8.5) into their COP with covariance intersection, so double counting doesn't make them falsely certain.
- **Deciders read a `SideView`:** only the tracks and messages that reached that agent, with their uncertainties. That is the only knowledge the decision code can see (rule 1).

### 9.3 Deception

Decoys, emitter decoys, false transmissions and spoofed tracks inject plausible detections into the opponent's pipeline. The fusion and classification models must be fooled in the right way, and the AAR shows when and why.

---

## 10. Decisions

### 10.1 Templates, gates and options

- **The same behaviour template format** as the TypeScript build (DECISION_FLOW_GRAPHS.md): triggers, branches, choose nodes, patches and inheritance, the fact vocabulary and the action library. Templates written in the current Behaviour Manager load unchanged where their actions exist.
- **Hard gates** (ROE, weapon release authority, EMCON, capability, cohesion state) are applied before options are offered. Jev never sees a removed option.
- **Decision points only:** agents are asked when a trigger fires (contact, under fire, link lost, a call for fire, a CCIR), not every tick. That's how behaviour-tree-driven forces scale to thousands of entities **[ref R8]**.
- **Reaction time:** an answer takes effect after a reaction time drawn from the agent's skill, fatigue and drill (§8.10), not instantly.

### 10.2 Jev

- **Prompts carry beliefs in words:** the agent's situation, its doctrine, its orders and the options with their doctrine bias, as the current `jevRealtimeDecider` and `jevWorldDecider` do.
- **Called remotely,** as the TypeScript build does: OpenRouter's Decisions API with `typesafe/jev-1.13`, one `choice` question per agent, answered with a choice, probabilities and a confidence. The Rust client is a port of `data/jevClient.ts`, and the request and response types are the same.
- **Batching:** all of one side's questions at one moment go in one request, answered in parallel. Requests for both sides, and for several moments when the clock is far ahead, go concurrently.
- **Latency doesn't change the game:** the clock waits in simulated time (§3.2). It only changes how fast the game can play. At high compression, model latency is the limit, so the host shows "waiting for Jev" as today, and a cache (as `data/jevCache.ts` does) answers repeated identical questions.
- **Fallbacks:** below a confidence threshold, on a timeout, on an error or when offline, the template's fallback decides, and the log says why. A game with no network still plays, on the rules.
- **Cost:** each call's reported cost is logged per side and per game, and the batch runner uses the rules decider by default so overnight runs cost nothing.
- **Keys** are kept by the host (the OS keychain, or an environment variable for the CLI), not compiled into the UI. This is safer than the current `VITE_OPENROUTER_API_KEY`, which ships in the bundle.

### 10.3 Claude planners

- **The planner hierarchy** of MULTI_DOMAIN_PLAN §4.4 (overall, land, air, maritime, space and cyber, sustainment) plans from the HQ's fused picture, through the Anthropic Messages API called directly from the host (the TypeScript build goes through a Foundry AIP function), into structured orders that are validated before they're sent.
- **Structured output:** plans are requested as JSON matching the orders schema (generated from the Rust types), so parsing is a validation step, not guesswork.
- **Rate limits and cost:** planning requests are few (on CCIR triggers and the battle rhythm), queued per HQ, retried with backoff, and logged with their cost.
- **The heuristic planner** is the fallback, and the default for batch runs.

### 10.4 People

A person can take any HQ's seat. They see the HQ console (the inbox, the COP with uncertainty, comms state, sent orders). Their orders travel the nets like anyone else's. Staff drafts come from the planners, as W15 built.

### 10.5 Later: local models

Once the engine plays well with remote models, local inference can be added as another implementation of the same `Decider` and planner traits:

- **Serving:** an OpenAI-compatible server (llama.cpp or vLLM). Output is constrained to the option keys by a grammar or JSON schema, and probabilities and confidence come from the token logprobs. vLLM's continuous batching suits per-side batches.
- **Which model:** Jev's own weights if they can be run locally; otherwise an open model, checked against remote Jev on the decision vignettes (same scenes, compare choice distributions) before it's trusted.
- **Memory:** a local model wants most of a GPU's memory (§11.3).
- **Choice per side and per run:** remote, local or rules, in the scenario's settings, so the same game can compare them.

### 10.6 Safeguards

The research's warnings apply **[ref R8]**:
- bounded option lists;
- explicit de-escalation and withdrawal options in every template;
- full decision logging;
- framing-sensitivity tests: the same situation with reworded prompts should give similar choice distributions, and a vignette checks it.

---

## 11. The GPU

### 11.1 What runs there

| Kernel | Input | Output | Used by |
|---|---|---|---|
| Line of sight, batched | DEM tile, observer–target pairs with heights | visible, clearance margin | sensing, direct fire, radios |
| Viewsheds | DEM, observer positions and heights | visibility rasters | overlays, exposure-aware routing, quiet skipping |
| Candidate pairs | entity positions by layer, sensor ranges and fields of regard | pairs in range, in view, unmasked | sensing (§8.2) |
| Path-loss fields | DEM, land cover, emitter positions, bands | path-loss rasters | jamming maps, comms coverage, ESM |
| J/S and coverage maps | path-loss fields, emitter powers | interference rasters per band | the UI, quiet skipping, planners' pictures |
| Cost and flow fields | cost raster, goals | flow fields | many movers to one goal (convoys, formations) |
| Thermal and light fields | land cover, time, weather | background luminance and temperature | sensing contrast |
| Monte Carlo kernels | engagement parameters | hit and kill distributions | calibration, fitted meta-models (§8.13) |

### 11.2 What doesn't

Agent logic, decisions, messages, tracks, logistics flows and anything branchy stay on the CPU, in parallel with `rayon`.

### 11.3 How

- **Data layout:** positions and per-entity sensor parameters in struct-of-arrays GPU buffers, updated incrementally each tick. Only moved entities are uploaded.
- **Terrain:** DEM and land-cover tiles stay resident in GPU memory as texture arrays, paged by area of interest.
- **Asynchrony:** kernels for tick t+1's candidates run while the CPU does tick t's decisions, so latency is hidden.
- **CPU twins:** every kernel has a CPU implementation. Tests compare them on random inputs. The engine runs without a GPU, slower, and batch runs on a GPU-less machine still work.
- **Memory:** with remote models, the GPU belongs to the simulation, whose kernels need a few hundred MB to a few GB. If local models are added later (§10.5), they want most of a card: cap the model's allocation on a shared card, or use two cards, or put the model on another machine.

---

## 12. Determinism and replay

The engine is deterministic for a given seed, build and machine. It isn't trying to match the TypeScript engine.

- **Keyed RNG:** every random draw comes from a stream keyed by `(game seed, purpose, entity, tick or event id)`, so the order threads run in doesn't change the outcome. ChaCha8 seeded from a hash of the key. No global RNG.
- **Ordered iteration:** systems iterate entities in id order where order matters. Parallel loops write only to their own entity's slots. Reductions use a fixed tree order.
- **No hash-order dependence:** `BTreeMap`, or sorted vectors, wherever iteration affects results.
- **Floating point:** f64; no fast-math; `libm` where results must be stable across platforms.
- **The GPU:** kernel outputs that feed decisions are quantised (visible or not; clearance in centimetres) and are robust to the last bit. A replay on a different GPU may still differ, and the record says which machine and build made it.
- **Model answers are recorded** in the game record, so a game with Jev and Claude replays exactly without calling them again. (The TypeScript build couldn't do this.)
- **Snapshots** every N simulated seconds plus the event log allow scrubbing back and forth through a replay, and restarting a game from any point with different decisions ("what if").

---

## 13. Performance targets

Reference machine: 8–16 CPU cores, 32–64 GB RAM, one recent mid-to-high-end GPU (12–16 GB).

| Target | Number |
|---|---|
| Brigade fight, entity level | 2,000 entities at ×10 real time, with the UI at 60 fps |
| Division with aggregation | 10,000 entities (most aggregated) at ×10 |
| Theatre campaign | 50,000 entities and formations at ×100 with quiet skipping |
| Batch | 1,000 runs of a 1-hour battalion vignette in under 10 minutes, rules deciders |
| Memory | Under 4 GB for the division case |
| UI channel | Under 1 MB/s of frame data at 2,000 entities |

**The tick budget** at the brigade target: 100 ms per simulated second at ×10. Roughly 35 ms sensing (with the GPU), 15 movement, 15 tracks and fusion, 10 comms, 10 fires and effects, 15 everything else. These are measured with `tracy` from the first spike (§18, phase 0), and every phase has a benchmark that fails if it regresses.

---

## 14. Data

### 14.1 The catalogue

- **Synthetic and open-source-anchored,** as W14 built: every platform record has its figures with sources, or **[default]**. No real-world-exact or classified data.
- **Stored as JSON** in `data/catalogue/` with a JSON Schema, loaded into SQLite at build time, and validated: units, ranges, references (a mount's munition exists, a radio's band is in the bands table).
- **More physics per record:** RCS by aspect and band, IR signature by state, acoustic signature by speed, radar parameters, sonar parameters, and armour by zone. Defaults per class, so a new platform can be added with a few numbers.

### 14.2 Terrain and environment

| Data | Source | Processing |
|---|---|---|
| Elevation | Copernicus GLO-30 | Offline tool: reproject, tile, pyramid, compress (zstd), metadata |
| Land cover | ESA WorldCover | Reclassify into engine classes, tile |
| Roads, rail, rivers, bridges, urban | OpenStreetMap | Extract by area, clean, build graphs |
| Soil | SoilGrids **[default classes]** | Classify for trafficability |
| Bathymetry | GEBCO | Tile |
| Weather (historical) | ERA5 | Optional import for a date and area |
| Climatology | Monthly norms | Synthetic weather generator |

A "theatre pack" is a folder of tiles and graphs for one area, built once by `tools/`, and loaded lazily by area of interest.

### 14.3 Formats

- **Scenario v4:** the current scenario JSON (forces, scope, world setup, templates, victory, events) with additions for the new physics (emitter settings, weather source, theatre pack). A converter reads the TypeScript v3 files.
- **Game record:** scenario, seed, build, machine, every order and command, every model answer, plus snapshots. Saved as a zip with JSON and binary parts.
- **Event log:** typed events into DuckDB for the AAR and analysis, and JSONL for export.

---

## 15. Calibration and validation

1. **The figures registry** holds every figure with value, unit, range, source, confidence and sensitivity. A generated report lists them, with the research backlog: unsourced figures ranked by sensitivity.
2. **Physics checks against analytic results:** radar range equation, sonar equations, ballistics without drag, Kalman filter consistency (NEES tests), link budgets against published curves, SGP4 against reference vectors. These are unit tests.
3. **Benchmark vignettes,** each a scenario with a hypothesis and an acceptance band from the research: MBT engagement **[P27]**, FPV effectiveness **[P9]**, Shahed raid intercept **[P10]**, ballistic missile defence **[P20]**, breach time **[P13]**, attack halt **[P14]**, precision fires under jamming **[P15]**, detect-to-strike times **[P2]**, counter-battery survival **[P23]**, sortie generation **[P30]**, casualty evacuation delays **[ref R7]**, Red Sea magazine depth **[ref R6.2]**. Run as distributions over hundreds of seeds; the test checks the distribution sits in the band.
4. **Sensitivity analysis:** Morris screening across all figures, then Sobol indices for the top ones, per vignette. This tells you which unsourced numbers matter, and therefore which to research first.
5. **Meta-model fitting** for aggregated combat (§8.13), with its own goodness-of-fit checks.
6. **Historical vignettes:** where open accounts allow, set up known engagements and compare outcome ranges. They're for face validity, not prediction.
7. **Face validation:** structured playtests with people who know the domain, using the AAR to ask "why did that happen?" **[ref R8: SAS-139]**.
8. **Regression:** every vignette's distribution is stored. A change that moves one out of band, or shifts it significantly (KS test), fails CI until it's accepted with an explanation.

---

## 16. UI and tools

- **The React screens carry over:** setup, the map, the feed, the inspectors, the world panel, the HQ console, the Behaviour Manager with its graph view, the AAR. The types come from Rust (`ts-rs`).
- **The map** moves to deck.gl layers on MapLibre for entities, tracks with uncertainty ellipses, sensor footprints, viewsheds, jamming and coverage fields (as GPU textures from §11), kill chains, and the network graph.
- **The Tauri channel:** commands for control (advance, answer, command, seat, save) and a binary event stream for frames (positions as typed arrays, changes since the last frame) and log entries.
- **New views the physics makes possible:**
  - spectrum view: who is emitting what, where, and what is jammed;
  - a link-budget inspector for any message: why it took 40 s or was lost;
  - a detection inspector: why a unit saw or missed a target, with every factor;
  - a timeline scrubber over snapshots;
  - a "what if" branch from any moment.
- **The analysis workbench:** batch runs, distributions, sensitivity charts, and comparing two templates or two catalogues.
- **Scenario editor:** draw areas, place forces from the catalogue, set weather and time, and pick a theatre pack.
- **Later:** a 3D view (terrain, aircraft, missiles) as a separate window using the same frame stream.

---

## 17. Testing

| Kind | What |
|---|---|
| Unit | Every model function, against analytic results and published curves (§15.2) |
| Property (`proptest`) | Invariants: no entity in two places; stores never negative; a killed vehicle stays killed through aggregation; messages never arrive before they were sent; truth never reaches a `SideView` |
| GPU twin | Each kernel against its CPU version on random inputs |
| Determinism | Same seed, build and machine: the same event log hash, single- and multi-threaded |
| Snapshot (`insta`) | Golden decision scenes: the template walk's offers and fallbacks for each situation |
| Statistical | Vignette distributions in band (§15.3), and no unexplained drift |
| Performance (`criterion`, benchmarks) | Per-system and whole-tick budgets (§13); CI fails on regressions beyond a threshold |
| Fuzz (`cargo-fuzz`) | Scenario, template and record loaders never panic on bad input |
| End-to-end | The desktop app plays a preset with recorded answers and matches its event log hash |

---

## 18. Roadmap

Sizes are rough, for one experienced Rust developer working full time. Several phases can run in parallel with two to four people (environment and propagation alongside the land engine, the UI alongside both).

| Phase | Builds | Size | Done when |
|---|---|---|---|
| **0. Spike** | Workspace; `core` (geodesy, figures, keyed RNG); `bevy_ecs` world; terrain tile loading; CPU and GPU line of sight with twin tests; Tauri shell drawing 10,000 Rust-moved entities on deck.gl | 3–4 weeks | 10k movers at 60 fps in the UI; GPU LOS matches CPU; `tracy` shows the tick |
| **1. Land tactical** | Ground movement with trafficability; EO/IR/thermal sensing (ACQUIRE/TTP); tracks with Kalman filters; direct fire with error budgets and penetration; suppression, stress, cohesion; the template walker and rules decider; scenario loader with a v3 converter; headless CLI | 10–14 weeks | Tank and mech company vignettes in band (MBT Ph, halt threshold); the TS Skirmish presets load and play |
| **2. Deciders** | Remote Jev client (OpenRouter Decisions API, a port of `jevClient.ts`) with batching, caching and timeouts; Claude planners through the Anthropic API with structured output; keys in the host; recorded answers in game records; cost logging | 3–4 weeks | A Skirmish with Jev and Claude plays at ×10, falls back to the rules when offline, and replays exactly from its record without calling them |
| **3. Fires, engineers, sustainment at unit level** | Indirect fire with trajectories, meteorology and lethal areas; counter-battery radar; obstacles, minefields, breaching, digging; unit stores and consumption | 6–8 weeks | Breach time and counter-battery survival vignettes in band |
| **4. C2 and comms** | Propagation (diffraction, ITM-style, HF); bearers with budgets; nets with queues, TDMA and mesh; PACE; HQs with staff queues, fusion by covariance intersection; planners in seats; structured orders | 8–10 weeks | Out-of-comms units act on intent; HQ loss slows reporting; detect-to-strike times emerge in band |
| **5. Drones, EW, PNT** | Drone types and link dependence; FPV strike chain; counter-drone; jammers as emitters with J/S; GNSS C/N₀ and INS drift; spectrum view | 8–10 weeks | FPV effectiveness, precision-under-jamming and kill-zone depth vignettes in band |
| **6. Air and air defence** | 3-DoF aircraft and missiles; radar equation with RCS, clutter and Doppler; IADS kill chains; WTA optimisation; airspace control; sorties | 10–12 weeks | Shahed raid intercept and cost exchange, SEAD package and sortie rates in band |
| **7. Logistics, medical, aggregation** | Network flows; convoys and interdiction; maintenance; the medical chain; aggregation and disaggregation; fitted aggregate models | 8–10 weeks | Culmination when supply is cut; casualty evacuation delays in band; aggregate models agree with entity runs |
| **8. Naval and undersea** | Ships and submarines; sonar equations with layers; layered ship defence with magazines; mines; ports | 10–12 weeks | Red Sea magazine-depth and salvo vignettes in band |
| **9. Space, cyber, kill web, theatre scale** | SGP4 satellites and imagery queues; satcom; cyber access campaigns; the joint kill web; theatre packs and ECEF everywhere; quiet skipping to next detectable event | 8–10 weeks | A joint operation runs at ×100; the full kill-chain AAR works |
| **10. People and tools** | HQ console and seats; AAR, timeline scrubbing, "what if" branches; detection and link-budget inspectors; scenario editor; analysis workbench | 6–8 weeks | A person plays a brigade HQ through a combined-arms battle and wins or loses through orders alone |
| **11. Calibration campaign** | Full sensitivity analysis; research backlog worked through; historical vignettes; playtests | 6+ weeks, then ongoing | Every benchmark vignette in band; the top-20 sensitive figures sourced or bounded |
| **Later** | Local models (§10.5), checked against remote Jev on the decision vignettes; multiplayer server (authoritative host over WebSocket, one seat per player); 3D view; in-process inference | — | — |

Total: about 90–110 weeks for one person; about 9–12 months for a team of three or four.

**Each phase ends with a playable scenario**, its vignettes in CI, its benchmark in budget, and its figures in the registry with sources or **[default]**, as the TypeScript milestones did.

---

## 19. What carries over from the TypeScript build

| TypeScript | Rust | How |
|---|---|---|
| `behaviour/` (templates, walk, patches, store) | `bgws-behaviour` | **Port.** Same JSON; the golden decision scenes become `insta` tests with outputs exported from TS |
| `realtime/catalogue/` | `bgws-catalogue` | **Port and extend** with more physics per record |
| `scope.ts`, `scenario.ts`, presets | `bgws-scenario` | **Port**, plus a v3 → v4 converter |
| `world/figures.ts` registry | `bgws-core::figures` | **Port the idea and the entries**; add units and confidence |
| `engine/` land engine | `bgws-sim` land systems | **Redesign** with the physics of §8.1–8.2, §8.8, §8.10 |
| `world/domains/*` | `bgws-sim` domain systems | **Redesign** each with its §8 model; keep the decision points and their options |
| `world/net.ts`, comms | `bgws-prop` + `bgws-sim` comms | **Redesign** with link budgets and queues |
| `engine/picture.ts`, fusion | `bgws-know` | **Redesign** with Kalman tracks and association |
| `world/command/` planners and seats | `bgws-decide` + `bgws-sim` HQs | **Port** the seat model and prompts |
| `engine/jevDecider.ts`, `world/jevWorldDecider.ts` | `bgws-decide` | **Port** the prompt shapes; swap the transport |
| `world/vignettes.ts`, `world/batch.ts` | `bgws-analysis` | **Port** the vignettes as hypotheses; redo the bands with the new models |
| React screens | `web/` | **Keep**, with generated types and a `TauriRunner` behind the `RunnerHandle` interface |
| Research reference | — | **Keep** as the evidence base for both builds |

---

## 20. Risks and open questions

| Risk | Mitigation |
|---|---|
| Physics fidelity without calibration data looks precise but is wrong | Every figure declared with confidence; sensitivity analysis drives research; vignettes checked against the sourced anchors; claim plausibility, not prediction |
| Scope is very large | Phases each ship a playable scenario; aggregation and abstracted domains let early phases play big scenarios with only some domains modelled in detail |
| Performance targets missed | Profile from phase 0; GPU for the bulk numeric parts; aggregation by scope; benchmarks in CI |
| GPU differences across machines | GPU narrows, CPU decides; quantised outputs; records name the machine; CPU twins |
| Remote model latency limits how fast a game with Jev can play | The clock waits in simulated time, so results don't change; batching per side, concurrent requests, a cache, and the rules decider for batch runs |
| Remote services are down, rate-limited or change their API | Fallback to the template's choice, logged; retries with backoff; the client behind a trait, so another provider or a local model can replace it |
| Per-call cost adds up on long games | Cost logged per game; decision points only (not every tick); aggregated and far units use the rules decider; the cache |
| Later local models are worse than remote Jev | Compared on the decision vignettes before use; remote stays available |
| Aggregate models drift from entity models | Refit automatically when entity models change; agreement checks in CI |
| Open data quality (OSM gaps, coarse soil) | Defaults per class; tools to edit a theatre pack; sensitivity tells whether it matters |
| Behaviour templates written for the TS engine need actions the Rust engine lacks | Loader reports unknown actions; an action compatibility table; templates degrade to their fallbacks |

**Open questions:**

- For later: can Jev's own weights run locally, or is it remote-only? This decides whether local play can use Jev itself or needs a substitute model.
- How many of the research reference's unsourced areas can be closed with open sources: DF-to-strike, Western F2T2EA timings, sortie and tanker rates, ASW detection probabilities, radar and anti-ship missile detection ranges, jamming radius by power, readiness and repair rates?
- Multiplayer: is it needed early enough to shape phase 4's host design? The sans-IO core supports it either way.
- Should the browser build (the sim compiled to WASM with WebGPU) be maintained alongside the desktop one, or only the desktop?
