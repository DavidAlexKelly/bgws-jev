# The BGWS protocol

`bgws-server` hosts one game and talks to any number of frontends over a
WebSocket and a few HTTP routes. The React viewer in `frontend/viewer` is
one such frontend. It has no privileges the protocol does not give, so it
can be replaced by any other client: another web app, a desktop or game
engine front end, a Python notebook, a bot. This document is the contract.

- **Types:** the data structures (frames, questions, events, outcomes) are
  generated from the Rust structs with `ts-rs`, in `protocol/types`. Other
  languages can read them as JSON Schema-like references. Regenerate them
  after changing a Rust type:

  ```sh
  TS_RS_EXPORT_DIR=$PWD/protocol/types cargo test -p bgws-core -p bgws-sim --features ts --lib export_bindings
  ```

- **Client:** `protocol/client/bgwsClient.ts` is a small typed TypeScript
  client for the WebSocket.
- **Server:** `apps/server/src/lib.rs` (`Inbound` is every message it
  accepts).

The server listens on `127.0.0.1:8765` by default (`--port`). Every HTTP
response carries `Access-Control-Allow-Origin: *`, so a frontend may be
served from anywhere (a dev server, a file) and still use the server.

## Serving a frontend

| How | What `/` is |
|---|---|
| default (`bundled-viewer` feature) | the React viewer, embedded in the binary |
| `bgws-server <scenario> --web <dir>` | `<dir>/index.html`. Any path that is not one of the server's own is a file in `<dir>`, else `index.html` again (single-page apps). |
| built `--no-default-features` | a note saying to pass `--web` |

## WebSocket: `/ws`

Text frames are JSON objects with a `kind`. Binary frames are position
updates (below). A client sends `hello` once it is open. The server answers
with the game, the clock, any questions waiting and the event feed so far,
then streams.

### Client → server

| `kind` | Fields | Effect |
|---|---|---|
| `hello` | | the game, state, frame, waiting questions and the feed so far |
| `play` / `pause` | | run or stop the clock |
| `speed` | `speed` (simulated s per real s) | |
| `advance` | `seconds` | step the clock that far, then reply `advanced` (a client that runs the clock itself) |
| `viewer` | `viewer`: `umpire`, `blue`, `red` | whose picture frames show (truth, or a side's belief) |
| `answer` | `question` (id), `choice` (option id) | answer a question put to a person's seat |
| `command` | `side`, `plan` (the orders schema) | a person's plan for a side, sent over its HQ's nets |
| `seat` | `side`, `seat`: `planner`, `person`, `rules` | who sits in a side's HQ seat |
| `seek` | `t` | show the keyframe at or before `t` (pauses) |
| `live` | | back to the live game |
| `branch` | `t` | restart at `t` and play on ("what if") |
| `save` | | write the game record (`--record`) |
| `labKeep` | | keep the game so far in the decision lab (`status` says how many decisions) |
| `inspect` | `unit` | one unit in detail (`unit`) |
| `why` | `unit` | what happened to a unit and why (`report`) |
| `review` | | the after-action review so far (`report`) |
| `detect` | `observer`, `target` | why one unit would see another now, every factor (`report`; umpire) |
| `messages` | `unit` | its latest messages and why each was slow or lost (`report`) |
| `viewshed` | `unit` | what it can see (`viewshed`; own units in a side's view) |
| `scenarios` | | the shipped scenarios and the theatre packs (`scenarios`) |
| `open` | `name`, `pack`? | replace the game with a shipped scenario, moved onto a pack if given |
| `check` | `scenario` (v4 JSON) | check an edited scenario (`checked`) |
| `load` | `scenario` | replace the game with an edited scenario |
| `terrain` | `name`, `bbox` [w, s, e, n], `cell_m`? | build a theatre pack in the background (`terrain` progress) |

### Server → client

| `kind` | Fields |
|---|---|
| `game` | `name`, `description`, `notes`, `scope`, `startUtc`, `victory`, `origin` {lat, lon}, `forces` {blue, red}, `deciders`, `flat`, `limitS`, `pack` (below) |
| `state` | `playing`, `speed`, `viewer` |
| `frame` | `t`, `live`, `frame`: a `FrameView` (`protocol/types/FrameView.ts`): entities, tracks, emitters, links, sensors, kill chains, `over` |
| `events` | `events`: [{ `clock`, `t`, `kind`, `text`, `side`, `entity`, `target`, `data` }], `replay`? (the feed so far) |
| `questions` | `questions`: `Question[]` waiting for a person (`protocol/types/Question.ts`) |
| `unit` | the unit in detail (orders, phases, people, weapons, what it knows), or `gone: true` |
| `report` | `title`, `text` (the inspectors and the review) |
| `viewshed` | `unit`, `side`, `cols`, `rows`, `cellM`, `rangeM`, `weaponM`, `lon`, `lat`, `west`, `south`, `east`, `north`, `surfaceM`, `coverage`, `runs`: each cell's chance of a clear line in tenths (0–9), row by row from the south-west, as `[tenths, count, …]` |
| `scenarios` | `scenarios`: [{ `file`, `name`, `description`, `type`, `durationMin`, `units` }], `packs` |
| `terrain` | `name`, `state`: `running` (`stage`, `fraction`), `done` (`pack`), `failed` or `busy` (`text`) |
| `checked` | `errors`: [string], `loaded` |
| `branched` | `t`, `new`? (a new game when `new`) |
| `advanced` | `t`, `over` (an `Outcome` or null) |
| `status` | `text`: a line for the status bar (game over, errors) |

A `pack` is { `name`, `bbox`, `cellM`, `basemap`, `dem`, `preview` (URLs
under `/packs/`), `roadsKm`, `towns`: [{ name, lat, lon, radiusM }],
`sizeKm`, `analysis` }. `analysis`, when the pack has a terrain analysis, is
{ `exposure`, `going` (RGBA PNG overlays over `bbox`), `keyTerrain` (JSON:
`key`: [{ kind: observation | chokepoint | crossing, lat, lon, score,
classes, note }]) }. A `unit` reply carries `terrain`: { `exposure` (0 dead
ground to 1 open), `deadGround`, `near`: [{ kind, note, distM }] } on such a
pack.

### Binary frames: positions

Between full frames (one a second while nothing appears or disappears),
positions arrive as little-endian binary:

```
u8  1                     message type
f64 t                     simulated seconds
u32 n                     entities
n × { u32 id, f64 lat, f64 lon, f32 alt, f32 heading_deg, u8 status }
                          status: 0 ok, 1 suppressed, 2 pinned, 3 broken, 4 destroyed
```

`id` matches `FrameEntity.id` in the last full frame.

## HTTP

| Route | |
|---|---|
| `GET /packs/:pack/*file` | a theatre pack's files (`basemap.pmtiles`, `dem.pmtiles`, `preview.png`, …), with byte ranges |
| `GET /world.pmtiles` | the world overview: the Protomaps planet, zoom ≤ 9, fetched once and cached; byte ranges only |
| `GET /editor` | the scenario editor (`frontend/editor`) |
| `GET /behaviour` | the Behaviour Manager (`frontend/behaviour`) |
| `GET /api/behaviour` | its data: every template, its graphs |
| `POST /api/templates` | save a custom behaviour template |
| `DELETE /api/templates/:name` | remove one |
| `GET /classic` | the earlier single-file viewer |
| `GET /lab` | the decision lab |
| `GET /api/lab/moments`, `GET /api/lab/moments/:id` | its moments: units' decisions with what each knew, play-outs, labels, graded answers |
| `POST /api/lab/moments/:id/label` | `{ best, acceptable, flagged, note }` |
| `POST /api/lab/moments/:id/playout`, `POST /api/lab/playout` | play out one moment, or up to `limit` not yet played out: `{ runs, horizonMin, limit?, point? }` |
| `POST /api/lab/collect` | `{ scenario, seeds, minutes, deciders }`: play games and keep their decisions |
| `POST /api/lab/grade` | `{ deciders, model?, prose?, judgedOnly? }`: ask a decider every moment afresh |
| `GET /api/lab/report` | how each decider does, overall and per decision point |
| `GET /api/lab/cases` | the decision cases (each with its options as built) and the test runs kept |
| `POST /api/lab/test` | `{ deciders, model?, prose? }`: test a decider on every case |
| `POST /api/lab/moments/:id/case` | `{ why }`: make a moment a decision case |
| `GET /api/lab/suite`, `POST /api/lab/suite` | the engagement suite's runs and scenarios; play it: `{ deciders, model?, prose?, seeds, keepWorst, all }` |

The map data is PMTiles: any client with a PMTiles reader (MapLibre with
the `pmtiles` protocol, `pmtiles` for Python, …) reads the packs and the
world overview straight from these routes.
