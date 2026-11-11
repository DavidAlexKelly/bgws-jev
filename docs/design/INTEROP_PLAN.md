# Interoperability: DIS and HLA 4

Status: **phases 1–3 built**. See the README's *Interoperability: DIS and
HLA 4*.
- Phase 1: DIS codes, the geoid, Entity State export with dead reckoning,
  real-time play.
- Phase 2: two-way DIS, remote entities, Fire and Detonation both ways,
  simulation management, terrain correlation.
- Phase 3: HLA 4 through the Federate Protocol with the RPR FOM, time
  management, a mock RTI. Tested only against the mock so far.

Phase 4 is planned.

Phase 1 as built differs from this plan in four places:
- **Dead-reckoning thresholds** are publisher settings (`DrSettings`), not
  sim figures. Every game's event log records the figure count, so new
  figures would change every replay fingerprint for a network option.
- **The geoid** lives in `bgws-interop` (`geoid.rs`), not `bgws-core`: the
  conversion only ever happens at the boundary.
- **PDUs** are built with `dis-rs` 0.14 (MIT), checked byte by byte against
  the standard's offsets. Its marking writer underflows on more than 11
  bytes, so markings are always cut first.
- **Heights** are placed by latitude and longitude from the east/north
  position plus the sim's height plus N. Running the 3-D vector through the
  origin's tangent plane would add the Earth's curvature (8 m at 10 km).

Phase 3 as built differs from this plan in these places:
- **No real RTI was available here.** Everything was tested against a mock
  Federate Protocol server with a small RTI behind it (`hla/mock.rs`,
  `bgws mock-rti`), written from FedProClient's session code.
  - Pitch's own Java client (FedProClient's Chat sample) runs against the
    mock end to end: `tools/fedpro-chat-check.sh`.
  - Our client against pRTI, and against a third-party RPR federate, is
    still to do.
- **The RPR FOM files are not vendored.** SISO publishes no licence terms
  for them, so `--fom` passes the federation's own. The class, attribute and
  parameter names and the datatypes are coded from RPR 2.0/3.0, and should
  be checked against the partner's FOM on first use.
- **Transport is TCP only** (no TLS or WebSocket yet). The session is plain
  threads, not async.
- **Time is HLAfloat64Time only**, in seconds.
- **Start synchronization was added:** `--federates N`, a member count, then
  the `ReadyToRun` synchronization point. Without it the first federate to
  join runs ahead, and time-managed runs differ.
- **The analytic mode of section 3.4 is `play_lockstep`.** Deciders stop the
  clock, and inputs apply at whole seconds. The record notes it, so replays
  use it too.
- **Every attribute is sent with each update**, there is no attribute
  ownership, and the mock has no class hierarchy (a subscription must name
  the class published).
- **No connection state in the viewer yet** (task 7).

Phase 2 as built differs from this plan in these places:
- **Correlation commands** are `bgws correlate <scenario> <partner.tif>`
  and `bgws export-ground <scenario> <out.tif>`, not `bgws terrain
  correlate <pack>`: `bgws terrain` already takes a pack folder to build,
  and a scenario covers generated ground as well as packs. Sampling is on a
  grid only (not along roads), features are not compared, and the "map" is
  the list of places in the JSON report.
- **Duplicate detonations** are recognised by (event, target, burst point
  to the metre), not by event ID alone: a salvo's rounds share one event.
- **Remove Entity** is not decoded; an entity whose state says it is
  deactivated (appearance bit 23), or that goes quiet for 12 s, is taken
  off. DIS v6 Entity State PDUs are read as well as v7 (the parser takes
  both), so there is no option for it.
- **The real-terrain rule** is applied when a game joins an exercise
  (`--dis`), not when any scenario with an `interop` block loads: the block
  alone federates nothing, and replays of a federated game need no partner.
  `"sharedGround": true` in the block allows generated ground.
- **No smoothing on correction:** a remote entity jumps to its owner's new
  state. With states at least every 5 s and dead reckoning between, the
  jump is small; smoothing would make our picture lag theirs.

This plan connects BGWS to other simulations over DIS (IEEE 1278.1) and HLA 4
(IEEE 1516-2025, through its Federate Protocol). It is in four phases. Each
phase is useful on its own, and each is built so the next one does not have
to redo it.

## 1. Decisions taken

| Decision | Choice | Why |
|---|---|---|
| Standards | DIS v7 (IEEE 1278.1-2012) and the RPR FOM 3.0 (SISO-STD-001.1) | Current versions; RPR mirrors DIS, so one mapping serves both. v6 receivers are added only if a partner needs them. |
| RTI | Develop against Pitch pRTI, behind our own interface | Pitch wrote the Federate Protocol client this follows; the protocol is a standard, so another RTI that serves it should need no code change. |
| Time | Real time first; HLA time management for analytic runs later | Real time is what partner simulations expect, and DIS can do nothing else; time management keeps repeatable, faster-than-real-time runs when the federation allows it. |
| Direction | Export first, designed for two-way; two-way follows | Export proves the mapping, where most interoperability bugs live; two-way is the goal, so the identifier registry and mapping are two-way from the start. |

Testing comes first against open tools (Wireshark's DIS dissector, a DIS
viewer), so no partner or licence is needed until phase 3.

## 2. What already exists

- **Geodesy.** `bgws_core::geo` converts the sim's local east/north/up frame
  to and from geocentric (ECEF) coordinates (`Frame::to_ecef`,
  `vec_to_ecef`), which both standards use.
- **Truth per entity.** `Ident` (platform, side, name), `Kin` (position,
  velocity, heading), `Health` (fit, start, destroyed), `Act` (posture,
  activity), `Crew` (suppression, morale).
- **Events.** The event log already records what DIS and RPR carry as
  events: `fire.launch`, `fire.hit`, `fire.kill`, `fire.miss` (munition,
  range, rounds, hits, kills), `fires.fire` and `fires.impact` (indirect
  fire), the drone and air strike events, and `unit.destroyed`.
- **Entities made during a game.** Dismounts (`crates/bgws-sim/src/dismount.rs`)
  and munitions in flight are created mid-game; the sim handles a growing
  entity list. Remote entities can be created the same way.
- **A record of every game.** `GameRecord`
  (`crates/bgws-scenario/src/record.rs`) holds the scenario, seed, figures,
  commands and answers; a replay must match the event-log fingerprint.
- **Async runtime.** The server already runs on tokio.
- **The HLA 4 wire definitions.** Pitch's FedProClient (Apache-2.0) ships
  the IEEE Federate Protocol messages as three proto3 files
  (`RTIambassador.proto`, `FederateAmbassador.proto`, `datatypes.proto`,
  about 2,500 lines, under an IEEE royalty-free licence with attribution)
  and a small session layer (12 message types: new session, heartbeat,
  resume, terminate, HLA call request and response, callback request and
  response) over TCP, TLS or WebSocket.

What is missing: DIS enumerations ("DIS codes") for the catalogue; a way to
pace the sim to the wall clock without pausing for decisions; a notion of
an entity the sim does not own; and a place in the game record for what
arrives from outside.

One thing is wrong for interoperability today: heights.
- `Geodetic` treats height as above the WGS-84 ellipsoid, which DIS and RPR
  expect. But the elevation the terrain is built from (Mapterhorn) gives
  height above mean sea level (the geoid), and nothing converts between
  the two.
- Inside a game everything is offset alike, so it does no harm. Published,
  every entity would be off by the geoid undulation, about 30 m near the
  theatre origin (54.2° N, 18.6° E): buried or floating in a partner's
  world.
- Phase 1 fixes this at the boundary (section 4, task 4).

### Terrain does not match between simulations

DIS and HLA exchange where things are, never the ground. Each simulation
keeps its own terrain, and the databases differ in:
- **source** (our open data, or synthetic ground for generated scenarios,
  against a partner's DTED, imagery or survey data);
- **resolution** (our 30 m cells against a simulator's individual buildings
  and trees);
- **processing** (smoothing, canopy correction, roads cut into the ground,
  water and bridges);
- **height reference** (above).

The symptoms: vehicles floating or buried; line of sight one side has and
the other does not; cover that exists in one world only; movement through
rivers or buildings that exist in one world only; and disagreeing pitch
and roll.

The plan handles this in four ways:
1. **Clamp** remote ground entities to our ground (phase 2).
2. **Test correlation** with a partner's terrain at sample points and
   along sight lines (phase 2).
3. **Require real-world terrain packs** for interoperable scenarios (phase 2).
4. **Build packs from a partner's terrain data** when a federation agrees
   a common source (phase 4).

What still differs is documented in the after-action review.

## 3. Architecture

A new crate, `crates/bgws-interop`, in three layers. The sim stays free of
any protocol; the host owns the connection.

```
bgws-sim  ── truth (entities, events) ──►  bgws-interop::model   (protocol-free)
          ◄── external inputs ───────────         │
                                                  ├── dis   (PDUs over UDP)
                                                  └── hla   (Federate Protocol + RPR FOM)
bgws-run Host: paces the sim, drives the adapter, records inbound traffic
```

### 3.1 `model`: the shared mapping (protocol-free)

- `enums`: the SISO-REF-010 values we use. Entity type is (kind, domain,
  country, category, subcategory, specific, extra); also munition types,
  force IDs, detonation results and the appearance bits. We include only
  the values we use, each with its SISO-REF-010 table reference.
- `EntityTruth`: one entity as published. It holds:
  - identity: our `EntityId` and its external ID (DIS site, application,
    entity number; or the HLA object handle);
  - type: entity type and force ID;
  - motion: ECEF position, orientation (ψ, θ, φ), ECEF velocity and
    acceleration, and the dead-reckoning algorithm;
  - state: damage state and appearance;
  - marking: the label, up to 11 characters.
- `FireEvent` and `DetonationEvent`: firer, target, munition type, fuze,
  warhead, quantity, rate, location, velocity and result.
- `Registry`: two-way map between our `EntityId` and external IDs, the
  owner of each entity (us or a named remote), and the last state sent or
  received.
- `DeadReckoning`: the standard algorithms (DRM static, FPW and RVW at
  first), extrapolation, and the send test (position error over 1 m,
  orientation error over 3°, or the 5 s heartbeat; all figures).
- `from_sim(&Sim) -> Vec<EntityTruth>` and `events_from_log(&[LogEvent])`:
  read truth, never a side's picture.

### 3.2 Sim hooks (in `bgws-sim`)

- **`Remote` component.** It marks an entity owned elsewhere and holds its
  owner, external ID and last received dead-reckoning state. Systems skip a
  remote entity in movement (its owner moves it), in decisions, the people
  step, C2, logistics and dismounting. It stays a target for sensing and
  fire, and can be seen and reported like any enemy.
- **`Sim::apply_external(ExternalInput)`.** This is the only way anything
  from outside changes the sim. Inputs:
  - `EntityState`: create or update a remote entity;
  - `EntityRemoved`;
  - `Detonation`: on or near our entities;
  - `SimControl`: start, stop or freeze.
- **Damage ownership (the DIS and RPR convention).** The entity's owner
  decides its damage. When we hit a remote entity we publish Fire and
  Detonation and apply nothing. When a Detonation lands on our entity, we
  resolve it with our own model (`direct_fire::strike_odds` for an impact,
  the Carleton function for a burst), using the catalogue munition mapped
  from the DIS munition type.
- **Strength.** Remote entities are left out of our side strength and
  victory conditions unless the scenario says otherwise.

### 3.3 Determinism and the game record

- `GameRecord` gets `external: Vec<RecordedExternal { at, input }>`. Every
  input applied through `apply_external` is recorded, in order, at the tick
  it was applied. The record version goes up, and old records read
  `external` as empty.
- **Replay** feeds the recorded inputs back at their ticks without opening
  a connection. The fingerprint must still match; a test guards this.
- **Inputs are applied only at tick boundaries.** Adapters queue what
  arrives; the host drains the queue before each tick, sorted by arrival
  order. This keeps a run's input sequence well defined.

### 3.4 Time: two pacing modes in the host

- **Real time** (`--realtime[=scale]`). The host ticks the sim to the wall
  clock.
  - Questions no longer stop the clock: `Step::NeedsAnswers` becomes
    non-blocking in this mode. A unit with a question out carries on with
    what it was doing (it already does while `deciding`).
  - The answer is applied when it comes back, after at least the crew's
    reaction time (already modelled). A slow decider therefore costs time
    in the world, as it would for a real crew.
  - Answers are recorded with the tick they were applied at, so replays
    stay exact.
- **Analytic** (HLA time management, phase 3). The sim is time-regulating
  and time-constrained with a lookahead of one base tick (1 s); each tick
  waits for a time-advance grant. A Jev pause holds the federation. Runs are
  repeatable and can go faster than real time.

## 4. Phase 1: DIS codes and Entity State export

**Goal:** our battles appear correctly in any DIS tool. One-way, real time.

Tasks:

1. **DIS codes in the catalogue.**
   - Add an optional `dis` field to platforms and munitions in
     `crates/bgws-catalogue` (types, schema, and the `platform` CLI report).
   - Fill it for every land platform (about 30), then air, sea, drones and
     munitions. Kind and domain come from the record. Country is set per
     force in the scenario (generic platforms carry country 0, "Other").
     Category through specific come from SISO-REF-010.
   - Infantry, ATGM teams and other foot units are life forms (kind 3).
   - A test fails on any platform or munition without a code, or with a
     kind or domain that contradicts its catalogue domain.
2. **Scenario setting.** An `interop` block in the scenario: exercise ID,
   site and application numbers, the country per side, and the force ID per
   side (blue 1 friendly, red 2 opposing, neutral 3 by default).
3. **Mapping layer** (`model`).
   - Registry; truth from `Ident`, `Kin`, `Health` and `Act`.
   - Orientation from heading, plus pitch and roll from the ground slope
     under the entity.
   - Damage state from strength: none, slight, moderate, destroyed.
   - Appearance bits: destroyed, smoke, hatch, and the engine on or off
     (from `SigState`).
4. **Height datum at the boundary.**
   - Add a geoid model (EGM2008, or EGM96 as a smaller fallback) to
     `bgws-core::geo`: the undulation N at a latitude and longitude, by
     bilinear interpolation of a gridded model at 2.5′ or 15′.
   - Convert going out: ellipsoidal height h = MSL height H + N, before ECEF.
   - Convert coming in: H = h − N, before placing a remote entity.
   - Inside the sim nothing changes: terrain and positions stay in MSL
     heights, so games and replays are unaffected.
   - The grid is vendored, or downloaded once, as its licence allows; the
     NGA models are public.
   - Name the convention in `Geodetic`'s documentation: inside the sim,
     height is above mean sea level; the interop boundary converts.
5. **Dead reckoning.** Extrapolation and send thresholds as figures
   (`interop.dr.position_m` 1, `interop.dr.orientation_deg` 3,
   `interop.dis.heartbeat_s` 5), and algorithm FPW for moving entities.
6. **DIS encoding.**
   - The PDU header and the Entity State PDU, written by hand (fixed
     big-endian layout).
   - First check whether an existing Rust DIS crate (for example `dis-rs`)
     is correct and maintained enough to use instead; decide by testing it
     against our fixtures.
7. **UDP sender.** Broadcast or unicast (multicast in phase 2), port 3000
   by default. It is a `tokio` task fed by the host after each tick.
8. **Real-time pacing** in the host, with the non-blocking questions of
   section 3.4: a `bgws play --realtime --dis <addr>` command and a server
   option.

Tests and acceptance:

- Byte-exact fixtures: encode known entities and compare with captures
  checked by Wireshark's DIS dissector (the captures are committed).
- A geodesy check: a published ECEF position converts back to the
  entity's latitude, longitude and height within 0.1 m.
- Geoid: the undulation at reference points matches NGA's published
  values within 0.1 m (EGM2008) or 1 m (EGM96). A vehicle on our ground at
  the theatre origin is published at H + N, not H.
- By hand: our vehicles sit on the ground (not 30 m above or below it) in a
  DIS viewer using real-world elevation.
- Dead reckoning: a recorded track sent through the DR filter never strays
  past the thresholds, and sends far fewer PDUs than one per tick.
- A real-time run keeps to the wall clock within 5% over 10 minutes, with
  Jev answering asynchronously.
- By hand: a company attack seen in a DIS viewer with correct symbols,
  positions, headings and destruction.

Risks:

- Wrong enumerations. These are caught only by a viewer showing the wrong
  symbol, hence the by-hand check and the catalogue test.
- Real-time pacing changes how Jev's latency counts. Keep the analytic
  (paused) mode the default for experiments.

## 5. Phase 2: two-way DIS

**Goal:** other simulations' entities fight ours, over DIS.

Tasks:

1. **Fire and Detonation PDUs out.**
   - From the fire events: direct fire, missile launch and arrival (the
     in-flight queue gives both), indirect fire and impacts, drone strikes.
   - Event ID numbering and the munition descriptor (type, warhead, fuze,
     quantity, rate).
   - Detonation result from our resolution: entity impact, entity proximate
     detonation, ground impact.
2. **PDU decoding and the receiver.**
   - Entity State, Fire, Detonation, Remove Entity, Start/Resume and
     Stop/Freeze.
   - Filter on exercise ID; ignore our own site and application.
3. **Remote entities.**
   - The `Remote` component and `apply_external` (section 3.2).
   - Platform from the DIS entity type by reverse lookup in the catalogue:
     exact code first, then the nearest by kind, domain and category, then
     a generic stand-in per kind and domain; every stand-in is logged.
   - Dead reckoning between updates, clamped to our ground for land
     entities.
   - Time-out after 12 s with no update (2.4 heartbeats, a figure); the
     entity is then removed.
4. **Detonations on our entities.** Resolve them with our damage models
   (section 3.2). Log `interop.detonation` with the result, so the AAR and
   `bgws why` explain external damage like our own.
5. **Simulation management.**
   - Start/Resume and Stop/Freeze from outside pause and resume the
     real-time clock.
   - We answer Acknowledge PDUs.
6. **Record and replay.** Inbound inputs are recorded (section 3.3).
   `bgws replay` replays them with no network.
7. **Multicast**, and an option to receive DIS v6 Entity State PDUs if a
   partner needs it.
8. **Terrain correlation.**
   - A `bgws terrain correlate <pack> <partner-dem> [--features <file>]`
     command that compares our ground with a partner's elevation (GeoTIFF;
     we already read GeoTIFF) and, where given, their features:
     - sample points on a grid and along roads: the height difference;
     - sight lines between sampled pairs: does each world see through?
   - The report gives:
     - the height error distribution, and places beyond a tolerance
       (2 m by default, a figure);
     - the share of sight lines that disagree;
     - a map of where they differ.
   - In a run, a remote ground entity whose reported height is more than
     the tolerance off our ground is logged (`interop.terrain_mismatch`),
     so the AAR shows where the worlds differ.
9. **Real-world terrain for interoperable scenarios.**
   - A scenario with an `interop` block must use a real-world terrain pack;
     the loader refuses synthetic ground unless the scenario says every
     simulation in the exercise uses the same generated terrain (it is
     exported for them as GeoTIFF elevation plus land-cover rasters).

Tests and acceptance:

- **Two BGWS processes over localhost:** one runs blue, the other red, each
  owning its side. They must:
  - see each other's units;
  - fire at them, each side deciding damage to its own entities;
  - reach a plausible result, with no entity owned twice and no detonation
    applied twice.
- **Replay:** a recorded two-way game replays to the same fingerprint with
  no network.
- **Remote immunity:** a remote entity is never moved, decided for or
  damaged by our own systems (unit tests on each system's filter).
- **Correlation tool:**
  - correlating a pack with its own exported elevation reports zero height
    error and no disagreeing sight lines;
  - correlating it with the same elevation shifted 3 m, or with a ridge
    removed, reports the shift and exactly the sight lines the ridge
    blocked.
- **By hand:** against at least one third-party DIS simulation or generator.

Risks:

- **Terrain does not match between simulations** (section 2). Land entities
  are clamped to our ground and line of sight is ours. The correlation tool
  (task 8) measures the difference before an exercise, and the
  mismatch log shows it during one.
- **Remote tanks move only on their owner's updates.** At long heartbeats
  they jump. Mitigated by dead reckoning and a smoothing step on correction.
- **Misbehaving partners** (duplicate IDs, bad codes). These are logged and
  dropped, never allowed to fail the game.

## 6. Phase 3: HLA 4 through the Federate Protocol, with the RPR FOM

**Goal:** join an HLA 4 federation with the same truth as DIS; optional
time-managed analytic runs.

Tasks:

1. **Wire messages.** Vendor the three IEEE proto files with the
   attribution the licence asks for, and generate Rust code with `prost` in
   a `build.rs`.
2. **The session layer** (`hla::session`), following FedProClient's C++
   `session/` code:
   - the message header (size, sequence number, session ID, message type);
   - new session, heartbeats (interval and time-out figures);
   - resume after a dropped connection;
   - call request and response matching, callback request and response.
   - Transport: TCP first, then TLS and WebSocket.
   - A mock Federate Protocol server in Rust for tests.
3. **The RTI interface** (`hla::rti`), a typed async Rust API over the
   calls we need:
   - connect, create, join, resign and destroy the federation;
   - publish and subscribe object classes and interactions; register,
     update, reflect and delete object instances; send and receive
     interactions;
   - time management (phase 3b).
   - The rest of the HLA services are added as needed.
4. **RPR FOM 3.0.**
   - Vendor the FOM modules (from SISO) and load them at create or join.
   - Encoders and decoders for the HLA data encoding (fixed records,
     variant records, arrays, big-endian) and the RPR structures:
     - entity identifier and entity type;
     - world location, orientation and velocity;
     - the spatial variant record by dead-reckoning algorithm;
     - marking; munition descriptor; event identifier.
5. **Mapping RPR to the model.**
   - Objects: `BaseEntity.PhysicalEntity.Platform.GroundVehicle`,
     `.Aircraft`, `.SurfaceVessel`, `Lifeform.Human`, `Munition`.
   - Interactions: `WeaponFire` and `MunitionDetonation`.
   - Ownership: attribute updates from the same registry and dead-reckoning
     rules as DIS. Reflected objects and received interactions go through
     `apply_external`, exactly as DIS does.
6. **Phase 3b: time management.**
   - Time-regulating and time-constrained, with a lookahead of 1 s, using
     the 64-bit float time representation (or integer, as the federation
     agrees).
   - Advance by time-advance request per base tick; updates and
     interactions are timestamped.
   - This is the analytic mode of section 3.4, chosen per run.
7. **CLI and server.** `--hla <address>` and `--federation <name>`;
   connection state in the viewer.

Tests and acceptance:

- **Session layer against the mock server:**
  - heartbeats;
  - a dropped connection resumed without losing messages;
  - call responses matched to their requests under load.
- **Round trips:** RPR structures encode and decode losslessly, checked
  against byte fixtures.
- **Two BGWS federates through pRTI:**
  - the phase 2 two-way test, over HLA;
  - with time management, two runs with the same seeds give identical
    fingerprints.
- **By hand:** a third-party RPR federate (for example a viewer) shows our
  entities, and we see its own.

Risks:

- **Which RTIs serve the Federate Protocol.** Only Pitch's is known; to be
  checked before this phase. Fallback: link a C++ RTI's local library
  through FFI behind the same `hla::rti` interface.
- **FOM versions.** Partner federations may use RPR 2.0 or extensions. The
  mapping is data-driven per FOM version, and RPR 2.0 is added if needed.
- **HLA data encoding is fiddly**, so fixtures come first.

## 7. Phase 4: extras

Each piece is independent; order them by partner need.

- **Radio.**
  - Transmitter PDUs (and the RPR `RadioTransmitter` object) from our comms
    model: frequency, power, antenna and state per net.
  - Signal PDUs carry our message traffic as data, not voice.
  - Inbound transmitters feed our EW model, as jammers and emitters to
    detect.
- **Electromagnetic emissions.** Electromagnetic Emission PDUs (and RPR
  `EmitterSystem`) for our radars and jammers; inbound emitters for our
  ESM.
- **Minefields.** Minefield PDUs (the state of our obstacles, and lanes
  when breached).
- **Aggregates.** Aggregate State PDUs (and the RPR aggregate module) for
  our aggregated formations (LOD Aggregated), so they appear at all; split
  to entities when they disaggregate.
- **Environment.** Environmental Process PDUs for smoke screens.
- **Designator.** Designator PDUs for laser designation (for
  laser-guided munitions).
- **Terrain from a partner's data.** When a federation agrees a common
  terrain source, `bgws terrain build` takes it in place of our open
  sources, so every simulation starts from the same ground:
  - elevation as GeoTIFF;
  - land cover and features as vector files (GeoPackage or shapefile);
  - roads and buildings.
  - OGC CDB or OpenFlight import only if a partner gives nothing simpler.

## 8. Cross-cutting

- **Security.** DIS has no security; send only on networks meant for it,
  and default to localhost. The Federate Protocol uses TLS outside a lab.
  Inbound data is always validated: lengths, enumerations, finite numbers.
- **Performance.** At 600 entities and the 5 s heartbeat, outbound DIS
  traffic is small. Dead reckoning keeps updates proportionate. The
  adapter runs off the sim thread and never blocks a tick.
- **Licences and attribution.**
  - The IEEE proto files carry their attribution.
  - The SISO-REF-010 values we use are cited by table.
  - RPR FOM modules are vendored with SISO's notice, or downloaded at build
    if their terms do not allow vendoring (to be checked).
  - FedProClient is read as a reference only; nothing is copied. If any
    code is taken from it, keep its Apache-2.0 notice.
- **Documentation.**
  - A README section per phase.
  - The `interop` scenario block in the scenario format document.
  - A table of our platforms and their DIS codes, generated from the
    catalogue.

## 9. Order of work and rough size

| Phase | Size | Depends on |
|---|---|---|
| 1. DIS codes, export, real-time pacing | Medium | — |
| 2. Two-way DIS, remote entities, record and replay | Large | 1 |
| 3. HLA 4 Federate Protocol, RPR FOM, time management | Large | 2's model and `apply_external` |
| 4. Radio, emissions, minefields, aggregates, smoke, designators | Medium each | 2 (DIS) or 3 (HLA) |

## 10. Open questions

- Partner simulations and federations, when known, fix the DIS version, the
  RPR FOM version and any FOM extensions.
- Which RTIs your partners run, and whether they serve the Federate
  Protocol.
- Which terrain source each federation uses, and whether partners can
  share it (elevation and features) for a common build.
- The country codes to give each side when scenarios are not about a real
  nation (default 0, "Other").
- Whether remote entities should count toward victory conditions in mixed
  federations.
