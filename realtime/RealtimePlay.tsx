/**
 * RealtimePlay — the real-time mode.
 *
 * Place units, have the commander write orders, press Play. Every unit
 * moves, looks and shoots on one clock. docs/REALTIME_COMMAND_DESIGN.html:
 *
 *   commander (Claude)   writes mission orders, ONLY when the player pauses
 *                        and presses "Generate orders" (or before the start)
 *   unit leader (Jev)    makes the call at a decision point, inside them
 *   game logic           everything else, every second
 *
 * While the clock runs, the game raises flags for the player — a unit out of
 * orders, heavy losses, an objective taken — and never calls the commander on
 * its own. Pause, generate, review, resume: a unit in a fight is asked how to
 * comply (D0); one out of contact takes its new orders at once.
 *
 * A separate screen from the turn game on purpose — see Play.tsx. It shares
 * the rules (fire, sighting, terrain, fog of war) and the Jev client, and no
 * screen code.
 *
 * HOW THE CLOCK DRIVES THE SCREEN
 *
 * The simulation lives in a RealtimeRunner held in a ref, not in React state:
 * it advances many times a second, and re-rendering the whole screen for each
 * simulated second would be the slowest possible way to animate a map. An
 * animation frame loop turns wall time × speed into simulated seconds, asks
 * the runner to advance, then moves the existing markers in place
 * (`setLngLat`) — nothing is rebuilt unless a unit appears or disappears. The
 * side panel re-renders a few times a second from a counter.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import maplibregl from "maplibre-gl";
import ms from "milsymbol";

import { DechoBasemap } from "@acc/decho-basemap/react";
import { AppSwitcher } from "@/components/AppSwitcher";
import { planetStore } from "@/shared/map/dechoBasemapSetup";
import { foundryFileClient } from "@/shared/routing/foundryFileClient";
import { DEFAULT_TERRAIN } from "@/shared/routing/terrainDatasets";
import { useTerrainRaster } from "@/shared/routing/useTerrainRaster";

import { boardBounds, boardRing, isOnBoard, type LatLng } from "../lib/board";
import {
  DEFAULT_ORIGIN,
  placedFromList,
  toGameStateFromPlaced,
  type PlacedElement,
} from "../lib/forceBuilder";
import {
  centreOf,
  coversBoard,
  describeTerrainSource,
  rasterTerrain,
  type TerrainSource,
} from "../lib/rasterTerrain";
import { rasterRoutePlanner } from "../lib/routePlan";
import { mobilityClassFor } from "../lib/terrainAdapter";
import { moveBoard, onBoard } from "./board";
import { projectForSide } from "../lib/fogOfWar";
import { proceduralTerrain, STANDARD_GROUND } from "../lib/proceduralTerrain";
import type { Side } from "../lib/state";
import { COMMANDER_MODELS, foundryModelCall, type CommanderModelName } from "../data/commanderClient";
import { jevConfigured, openRouterJevCall } from "../data/jevClient";
import { createRng } from "../rules/dice";
import {
  FORCE_LISTS,
  PLATFORM_SNAPSHOT,
  playablePlatforms,
  TROOP_QUALITY,
  type TroopQualityName,
} from "../rules/forceList";
import { HOUSE_V1 } from "../rules/ruleset";
import { activityOf, createRealtimeState } from "./engine/engine";
import { jevRealtimeDecider } from "./engine/jevDecider";
import { applyOrders, commanderOrders, heuristicOrders, type OrdersResult } from "./engine/orders";
import { RealtimeRunner, type RtLogEntry } from "./engine/runner";
import { clock, DEFAULT_TIMING, PINNED_AT, SUPPRESSED_AT } from "./engine/timing";
import type { RtConfig, RtState } from "./engine/types";

type Phase = "setup" | "ordered" | "running";
type Viewpoint = Side | "both";

const BOARD_SOURCE = "rt-board";
const FIRE_SOURCE = "rt-fire";
const ORDER_SOURCE = "rt-orders";
/** Where faded contacts were last seen. */
const LAST_KNOWN_SOURCE = "rt-last-known";
const SPEEDS = [1, 5, 10, 30, 60];
/** How long a shot's line stays on the map, in simulated seconds. */
const FIRE_LINE_S = 12;
/** How long a decision's tag stays beside its counter, in simulated seconds. */
const DECISION_TAG_S = 40;
const SIDE_COLOUR: Record<Side, string> = { blue: "#8fc2ff", red: "#ff9e8f" };

/** The ground choices offered here: real land cover, or generated. */
type GroundChoice = Extract<TerrainSource, "generated" | "raster+relief" | "raster">;

let counter = 0;
const nextId = () => `rt-${(counter += 1)}`;

function liveMap(map: maplibregl.Map | null): maplibregl.Map | null {
  if (!map) return null;
  return (map as unknown as { style?: unknown }).style ? map : null;
}

function emptyCollection(): GeoJSON.FeatureCollection {
  return { type: "FeatureCollection", features: [] };
}

/** Why a Jev call failed, from the trace: the error it reported, or that it did not answer. */
function jevFailure(rationale: string | undefined): string {
  const text = rationale ?? "";
  const reported = /Jev unavailable \((.*)\) — the rules decided/.exec(text)?.[1];
  if (reported) return reported;
  if (/gave no answer/.test(text)) return "no answer for this unit in the reply";
  return "unknown";
}

/** One line of the feed, in words. */
function describeEntry(entry: RtLogEntry): string {
  const at = clock(entry.time);
  if (entry.type === "shot") return `${at}  ${entry.shot.firerId} fires on ${entry.shot.targetId}: ${entry.shot.result}`;
  if (entry.type === "event") return `${at}  ${entry.event.unitId} ${entry.event.kind}: ${entry.event.detail}`;
  if (entry.type === "flag") return `${at}  ⚑ ${entry.flag.text}`;
  const d = entry.decision;
  const p = d.trace.probabilities?.[d.optionId];
  const who =
    d.trace.chosenBy === "jev"
      ? `Jev${p != null ? ` ${Math.round(p * 100)}%` : ""}`
      : `rules${d.trace.fallback ? `, Jev ${d.trace.fallback === "error" ? `failed: ${jevFailure(d.trace.rationale)}` : d.trace.fallback}` : ""}`;
  const point = d.trace.question?.split(" ")[0] ?? "";
  return `${at}  ${d.unitId} ${point} → ${entry.summary} (${who}; asked ${clock(entry.askedAt)})`;
}

/** Orders for review, one unit a line. */
function OrdersReview({ results, state }: { results: OrdersResult[]; state: RtState | null }) {
  return (
    <>
      {results.map((result) => (
        <div key={result.side} style={{ marginBottom: 8 }}>
          <div style={{ ...subtle, color: SIDE_COLOUR[result.side], lineHeight: 1.5 }}>
            {result.side} · {result.by === "claude" ? "commander" : "heuristic"}: {result.plan}
          </div>
          {Object.entries(result.orders).map(([id, orders]) => (
            <div key={id} style={{ ...subtle, color: "#c7ccdb", lineHeight: 1.45, margin: "2px 0 4px 6px" }}>
              <b>{state?.game.forceElements[id]?.label ?? id}</b>: {orders.task}
              {orders.intent ? <span style={{ color: "#8a91a8" }}> — {orders.intent}</span> : null}
              <div style={{ color: "#6a7292" }}>
                {orders.urgency === "now" ? "NOW" : "when able"} · ROE {orders.roe} · on contact {orders.onContact}
                {orders.boundaries.length ? ` · stay ${orders.boundaries.map((b) => `${b.keep} of ${b.label}`).join(", ")}` : ""}
              </div>
            </div>
          ))}
          {result.warnings.map((warning, index) => (
            <div key={index} style={{ ...subtle, color: "#e8945a", lineHeight: 1.45 }}>
              ⚠ {warning}
            </div>
          ))}
        </div>
      ))}
    </>
  );
}

/** How one side ended: vehicles destroyed, and strength destroyed or broken, of what it started with. */
function sideTally(state: RtState, side: Side) {
  const own = Object.values(state.game.forceElements).filter((fe) => fe.side === side);
  const start = Math.max(1, state.startStrength?.[side] ?? own.reduce((sum, fe) => sum + fe.combatStrengthStart, 0));
  const alive = own.filter((fe) => fe.combatStrength > 0);
  const destroyed = 1 - alive.reduce((sum, fe) => sum + fe.combatStrength, 0) / start;
  const broken = alive.filter((fe) => state.units[fe.id]?.cohesion === "broken").reduce((sum, fe) => sum + fe.combatStrength, 0) / start;
  const vehicles = own.reduce(
    (sum, fe) => {
      const v = state.units[fe.id]?.vehicles;
      return { total: sum.total + (v?.total ?? 1), lost: sum.lost + (v ? v.total - v.fit : fe.combatStrength > 0 ? 0 : 1) };
    },
    { total: 0, lost: 0 },
  );
  const units = { total: own.length, destroyed: own.length - alive.length, broken: alive.filter((fe) => state.units[fe.id]?.cohesion === "broken").length };
  return { destroyed, broken, vehicles, units };
}

/** Why the loser lost, in a sentence. */
function howItEnded(state: RtState): { headline: string; why: string } {
  const over = state.over!;
  const pct = (x: number) => `${Math.round(x * 100)}%`;
  const why = (side: Side) => {
    const t = sideTally(state, side);
    const parts = [
      ...(t.destroyed > 0.005 ? [`${pct(t.destroyed)} destroyed`] : []),
      ...(t.broken > 0.005 ? [`${pct(t.broken)} broken and falling back`] : []),
    ];
    const what =
      t.destroyed >= 0.5 && t.broken < 0.005 ? "destroyed" : t.broken > 0.005 && t.destroyed < 0.005 ? "broken" : "destroyed and broken";
    return `${side} was ${what}: ${parts.join(", ") || "past its breakpoint"} of its strength.`;
  };
  if (over.reason === "time limit") {
    return {
      headline: over.winner ? `${over.winner.toUpperCase()} wins on time` : "Time ran out: a draw",
      why: `Neither side reached its breakpoint before the time limit; ${over.winner ? `${over.winner} held the better position` : "neither held the better position"}.`,
    };
  }
  if (!over.winner) return { headline: "Both sides broke: a draw", why: `${why("blue")} ${why("red")}` };
  const loser: Side = over.winner === "blue" ? "red" : "blue";
  return { headline: `${over.winner.toUpperCase()} WINS`, why: why(loser) };
}

/** The end of a game: who won and why, over the whole screen, with what to do next. */
function EndOverlay(props: {
  state: RtState;
  decisions: number;
  byJev: number;
  onRunAgain: () => void;
  onEdit: () => void;
  onNew: () => void;
  onClose: () => void;
}) {
  const { state } = props;
  const { headline, why } = howItEnded(state);
  const colour = state.over?.winner ? SIDE_COLOUR[state.over.winner] : "#e8c547";
  return (
    <div style={overlay} role="dialog" aria-modal="true" aria-label="Game ended">
      <div style={overlayCard}>
        <div style={{ ...groupTitle, borderBottom: "none", marginBottom: 2 }}>Game ended · {clock(state.time)}</div>
        <div style={{ fontSize: 26, fontWeight: 700, letterSpacing: "0.06em", color: colour, margin: "4px 0 8px" }}>{headline}</div>
        <div style={{ fontSize: 12, color: "#c7ccdb", lineHeight: 1.5, marginBottom: 14 }}>{why}</div>
        <div style={{ display: "flex", gap: 10, marginBottom: 14 }}>
          {(["blue", "red"] as const).map((side) => {
            const t = sideTally(state, side);
            return (
              <div key={side} style={{ flex: 1, padding: 8, border: `1px solid ${SIDE_COLOUR[side]}55`, borderRadius: 3 }}>
                <div style={{ ...subtle, color: SIDE_COLOUR[side], fontWeight: 700, textTransform: "uppercase", marginBottom: 4 }}>{side}</div>
                <div style={{ ...subtle, color: "#c7ccdb", lineHeight: 1.6 }}>
                  {t.vehicles.lost} of {t.vehicles.total} vehicles lost
                  <br />
                  {t.units.destroyed} of {t.units.total} units destroyed{t.units.broken ? `, ${t.units.broken} broken` : ""}
                </div>
              </div>
            );
          })}
        </div>
        <div style={{ ...subtle, marginBottom: 14 }}>
          {props.decisions} decisions, {props.byJev} by Jev
        </div>
        <div style={{ display: "flex", gap: 8 }}>
          <button
            onClick={props.onRunAgain}
            style={{ ...primary, marginTop: 0, flex: 1 }}
            title="Start again from the same setup and opening orders, with fresh dice: anything can go differently"
          >
            ↻ Run scenario again
          </button>
          <button onClick={props.onEdit} style={{ ...overlayButton, flex: 1 }} title="Back to placement with these units where they started">
            ✎ Edit scenario
          </button>
          <button onClick={props.onNew} style={{ ...overlayButton, flex: 1 }} title="Back to placement with nothing placed">
            + New scenario
          </button>
        </div>
        <button onClick={props.onClose} style={{ ...linkButton, marginTop: 12, color: "#8a91a8" }}>
          close and look at the battlefield
        </button>
      </div>
    </div>
  );
}

export default function RealtimePlay() {
  const [, setParams] = useSearchParams();
  const [phase, setPhase] = useState<Phase>("setup");
  const [placed, setPlaced] = useState<PlacedElement[]>([]);

  // Placement brush.
  const [side, setSide] = useState<Side>("blue");
  const [platform, setPlatform] = useState<string>("var_11_default");
  const [count, setCount] = useState(4);
  const [quality, setQuality] = useState<TroopQualityName>("regular");

  // Setup. Real land cover by default: the basemap under the counters is the
  // real world, and a game on generated ground over it has units crossing
  // rivers the rules do not know are there.
  const [groundChoice, setGroundChoice] = useState<GroundChoice>("raster+relief");
  const [groundSeed, setGroundSeed] = useState(STANDARD_GROUND.seed ?? "baltic-v1");
  const [gameSeed, setGameSeed] = useState("1");
  const [useJev, setUseJev] = useState(true);
  const [useCommander, setUseCommander] = useState(true);
  const [model, setModel] = useState<CommanderModelName>(COMMANDER_MODELS[0]);
  const [blueDirective, setBlueDirective] = useState("");
  const [redDirective, setRedDirective] = useState("");
  /** Orders written and waiting for review: at the start, or while paused. */
  const [draft, setDraft] = useState<OrdersResult[]>([]);
  const [orderSides, setOrderSides] = useState<Viewpoint>("both");
  const [guidance, setGuidance] = useState("");

  // Running.
  const [speed, setSpeed] = useState(10);
  const [playing, setPlaying] = useState(false);
  const [viewpoint, setViewpoint] = useState<Viewpoint>("both");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The end-of-game overlay, closed to look at the battlefield. */
  const [endDismissed, setEndDismissed] = useState(false);
  /** Bumped a few times a second so the panel reads the runner again. */
  const [, setFrame] = useState(0);

  const mapRef = useRef<maplibregl.Map | null>(null);
  const [mapEpoch, setMapEpoch] = useState(0);
  const markersRef = useRef(new Map<string, { marker: maplibregl.Marker; update: (s: RtState) => void }>());
  const runnerRef = useRef<RealtimeRunner | null>(null);
  const orderedRef = useRef<RtState | null>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const brushRef = useRef({ side, platform, count, quality });
  brushRef.current = { side, platform, count, quality };
  const viewpointRef = useRef(viewpoint);
  viewpointRef.current = viewpoint;

  // ── Ground ──────────────────────────────────────────────────────────────
  // The same raster, router and recentring the turn game uses, so a board
  // on real ground is the same board in either mode.
  const wantsRaster = groundChoice !== "generated";
  const { raster, isLoaded: rasterLoaded, error: rasterError } = useTerrainRaster(
    wantsRaster ? foundryFileClient : null,
    DEFAULT_TERRAIN.rasterRid,
  );
  const origin = useMemo(() => {
    if (!wantsRaster || !rasterLoaded || !raster?.bounds) return DEFAULT_ORIGIN;
    const centre = centreOf(raster.bounds);
    return coversBoard(raster, centre) ? centre : DEFAULT_ORIGIN;
  }, [wantsRaster, rasterLoaded, raster]);
  const rasterUsable = wantsRaster && rasterLoaded && !!raster && coversBoard(raster, origin);
  const effectiveGround: GroundChoice = rasterUsable ? groundChoice : "generated";
  const bounds = useMemo(() => boardBounds(origin), [origin]);
  const terrain = useMemo(() => {
    if (rasterUsable && raster) {
      return rasterTerrain(raster, {
        relief: groundChoice === "raster+relief" ? { ...STANDARD_GROUND, seed: groundSeed, origin } : null,
      });
    }
    return proceduralTerrain({ ...STANDARD_GROUND, seed: groundSeed, origin });
  }, [rasterUsable, raster, groundChoice, groundSeed, origin]);
  /** A* on the raster when there is one; otherwise the engine's bearing planner. */
  const planner = useMemo(
    () => (rasterUsable && raster ? rasterRoutePlanner(raster, mobilityClassFor) : undefined),
    [rasterUsable, raster],
  );
  /** Rivers are rivers: the raster's own passability, where it offers one. */
  const isPassable = useMemo(() => {
    const check = rasterUsable ? (raster as { isPassable?: (lat: number, lon: number) => boolean } | null)?.isPassable : undefined;
    return check ? (point: LatLng) => check.call(raster, point.lat, point.lng) : undefined;
  }, [rasterUsable, raster]);
  // Not cached, here or across page loads: a moment that recurs in another
  // run is asked afresh, so running a scenario again can go differently.
  const jevCall = useMemo(() => openRouterJevCall({ cache: false }), []);
  const platforms = useMemo(() => playablePlatforms(), []);

  /** `run` > 0: another run of the same scenario, with its own dice. */
  const makeConfig = useCallback(
    (run = 0): RtConfig => ({
      ruleset: HOUSE_V1,
      terrain,
      rng: createRng(run > 0 ? `${gameSeed}:realtime:run${run}:${Math.random().toString(36).slice(2)}` : `${gameSeed}:realtime`),
      timing: DEFAULT_TIMING,
      planner,
      isPassable,
    }),
    [terrain, gameSeed, planner, isPassable],
  );

  // ── Placement ─────────────────────────────────────────────────────────────

  // ⚠ READ THROUGH A REF. The map's click handler is registered once, when
  // the map is ready, and closes over whatever it saw then. Reading `bounds`
  // directly meant a board that moved (switching to real ground recentres it)
  // still accepted clicks only inside the OLD board, far away.
  const boundsRef = useRef(bounds);
  boundsRef.current = bounds;

  // Units already placed move with the board, keeping their layout, rather
  // than being stranded off the edge of it.
  const originRef = useRef(origin);
  useEffect(() => {
    const from = originRef.current;
    originRef.current = origin;
    if (phaseRef.current === "setup") setPlaced((current) => moveBoard(current, from, origin));
  }, [origin]);

  const place = useCallback(
    (at: LatLng) => {
      if (phaseRef.current !== "setup") return;
      if (!isOnBoard(at, boundsRef.current)) return;
      const brush = brushRef.current;
      const snapshot = PLATFORM_SNAPSHOT[brush.platform];
      if (!snapshot) return;
      setPlaced((current) => [
        ...current,
        {
          id: nextId(),
          label: `${snapshot.displayName} (${brush.count})`,
          side: brush.side,
          platform: brush.platform,
          platformCount: brush.count,
          troopQuality: brush.quality,
          position: at,
        },
      ]);
    },
    [],
  );
  const placeRef = useRef(place);
  placeRef.current = place;

  // ── Orders and the clock ──────────────────────────────────────────────────

  /**
   * Orders for one side: the commander's (Claude, through the published
   * query) or the heuristic's. Never throws: an unreachable commander means
   * the heuristic's orders, with a warning saying so.
   */
  const ordersFor = useCallback(
    async (state: RtState, s: Side, config: RtConfig, extra: { guidance?: string; recent?: { time: number; text: string }[] }) => {
      if (!useCommander) return heuristicOrders(state, s);
      const directive = s === "blue" ? blueDirective : redDirective;
      return commanderOrders(foundryModelCall(model, directive, "realtime"), state, s, config, extra);
    },
    [useCommander, model, blueDirective, redDirective],
  );

  /** Build the game from what is placed and have the commander write every unit's orders. */
  const generateOrders = useCallback(async () => {
    setError(null);
    setBusy(true);
    try {
      const config = makeConfig();
      let state = createRealtimeState(toGameStateFromPlaced(placed, HOUSE_V1, { gameId: gameSeed }));
      const results: OrdersResult[] = [];
      for (const s of ["blue", "red"] as const) {
        const result = await ordersFor(state, s, config, {});
        results.push(result);
        state = applyOrders(state, result, config);
      }
      orderedRef.current = state;
      setDraft(results);
      setPhase("ordered");
    } catch (thrown) {
      setError(thrown instanceof Error ? thrown.message : String(thrown));
    } finally {
      setBusy(false);
    }
  }, [makeConfig, placed, gameSeed, ordersFor]);

  /** Paused: new orders for one side or both, for review before the clock resumes. */
  const generatePausedOrders = useCallback(async () => {
    const runner = runnerRef.current;
    if (!runner || playing) return;
    setError(null);
    setBusy(true);
    try {
      const sides = orderSides === "both" ? (["blue", "red"] as const) : ([orderSides] as const);
      const results: OrdersResult[] = [];
      for (const s of sides) {
        results.push(await ordersFor(runner.state, s, runner.config, { guidance, recent: runner.recentFor(s) }));
      }
      setDraft(results);
    } catch (thrown) {
      setError(thrown instanceof Error ? thrown.message : String(thrown));
    } finally {
      setBusy(false);
    }
  }, [playing, orderSides, ordersFor, guidance]);

  /** Resume. Reviewed orders go to the runner first: in contact, each unit is asked how to comply (D0). */
  const resume = useCallback(() => {
    const runner = runnerRef.current;
    if (!runner) return;
    for (const result of draft) runner.issueOrders(result.orders, { side: result.side, text: result.plan });
    setDraft([]);
    setPlaying(true);
  }, [draft]);

  /** How many times this scenario has been run: the first uses the seed, each later one fresh dice. */
  const runsRef = useRef(0);
  const start = useCallback(() => {
    const initial = orderedRef.current;
    if (!initial) return;
    const config = makeConfig(runsRef.current);
    runsRef.current += 1;
    const deciders = useJev
      ? {
          blue: jevRealtimeDecider({ side: "blue", call: jevCall, directive: blueDirective }),
          red: jevRealtimeDecider({ side: "red", call: jevCall, directive: redDirective }),
        }
      : undefined;
    const runner = new RealtimeRunner(initial, config, { deciders });
    runnerRef.current = runner;
    setEndDismissed(false);
    setDraft([]);
    setPhase("running");
    setPlaying(true);
  }, [makeConfig, useJev, jevCall, blueDirective, redDirective]);

  const reset = useCallback(() => {
    runsRef.current = 0;
    setPlaying(false);
    runnerRef.current = null;
    orderedRef.current = null;
    setDraft([]);
    setEndDismissed(false);
    setPhase("setup");
  }, []);

  /**
   * The same scenario again: the same units where they started, with the same
   * opening orders, but fresh dice and Jev asked afresh — not a replay, so
   * anything may go differently. Nothing from the last run is kept.
   */
  const runAgain = useCallback(() => {
    setEndDismissed(false);
    setError(null);
    start();
  }, [start]);

  /** Nothing placed, nothing ordered: a blank board. */
  const newScenario = useCallback(() => {
    reset();
    setPlaced([]);
  }, [reset]);

  // The loop: wall time × speed → simulated seconds. One advance in flight at
  // a time; if the runner is waiting on a decision, frames simply pass.
  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    let last = performance.now();
    let owed = 0;
    let advancing = false;
    let lastPanel = 0;

    const loop = (now: number) => {
      const runner = runnerRef.current;
      if (!runner) return;
      owed += ((now - last) / 1000) * speed;
      last = now;
      if (!advancing && owed >= 1) {
        const step = Math.min(Math.floor(owed), 120);
        owed -= step;
        advancing = true;
        runner
          .advance(step)
          .then(() => {
            advancing = false;
            drawRef.current();
            if (runner.state.over) setPlaying(false);
          })
          .catch((thrown: unknown) => {
            advancing = false;
            setError(thrown instanceof Error ? thrown.message : String(thrown));
            setPlaying(false);
          });
      }
      if (now - lastPanel > 250) {
        lastPanel = now;
        setFrame((n) => n + 1);
      }
      frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame);
  }, [playing, speed]);

  // ── The map ───────────────────────────────────────────────────────────────

  const onMapReady = useCallback(
    (map: maplibregl.Map) => {
      mapRef.current = map;
      markersRef.current.clear();
      setMapEpoch((n) => n + 1);
      const draw = () => {
        if (!map.getSource(BOARD_SOURCE)) {
          map.addSource(BOARD_SOURCE, {
            type: "geojson",
            data: { type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [boardRing(bounds)] } },
          });
          map.addLayer({
            id: `${BOARD_SOURCE}-line`,
            type: "line",
            source: BOARD_SOURCE,
            paint: { "line-color": "#e8c547", "line-width": 2, "line-dasharray": [3, 2] },
          });
        }
        if (!map.getSource(ORDER_SOURCE)) {
          map.addSource(ORDER_SOURCE, { type: "geojson", data: emptyCollection() });
          map.addLayer({
            id: `${ORDER_SOURCE}-line`,
            type: "line",
            source: ORDER_SOURCE,
            paint: {
              "line-color": ["match", ["get", "side"], "blue", SIDE_COLOUR.blue, SIDE_COLOUR.red],
              "line-width": 1.5,
              "line-opacity": 0.6,
              "line-dasharray": [2, 2],
            },
          });
        }
        if (!map.getSource(LAST_KNOWN_SOURCE)) {
          map.addSource(LAST_KNOWN_SOURCE, { type: "geojson", data: emptyCollection() });
          map.addLayer({
            id: `${LAST_KNOWN_SOURCE}-circle`,
            type: "circle",
            source: LAST_KNOWN_SOURCE,
            paint: {
              "circle-radius": 7,
              "circle-color": "rgba(0,0,0,0)",
              "circle-stroke-width": 1.5,
              "circle-stroke-color": ["match", ["get", "side"], "blue", SIDE_COLOUR.blue, SIDE_COLOUR.red],
              "circle-stroke-opacity": ["get", "opacity"],
            },
          });
        }
        if (!map.getSource(FIRE_SOURCE)) {
          map.addSource(FIRE_SOURCE, { type: "geojson", data: emptyCollection() });
          map.addLayer({
            id: `${FIRE_SOURCE}-line`,
            type: "line",
            source: FIRE_SOURCE,
            paint: {
              "line-color": ["match", ["get", "side"], "blue", SIDE_COLOUR.blue, SIDE_COLOUR.red],
              "line-width": 2.5,
              "line-opacity": ["get", "opacity"],
            },
          });
        }
      };
      if (map.isStyleLoaded()) draw();
      else map.once("load", draw);
      map.on("click", (event) => placeRef.current({ lat: event.lngLat.lat, lng: event.lngLat.lng }));
    },
    [bounds],
  );

  /** Build a counter for one unit, and a function that keeps it up to date. */
  const makeMarker = useCallback((map: maplibregl.Map, id: string, sideOf: Side, label: string, at: LatLng) => {
    const node = document.createElement("div");
    node.style.cursor = "pointer";
    const inner = document.createElement("div");
    inner.style.cssText = "position:relative;display:inline-block;line-height:0";
    node.appendChild(inner);
    const symbol = new ms.Symbol(sideOf === "blue" ? "10031000141211000000" : "10061000141211000000", {
      size: 24,
    }).asDOM();
    inner.appendChild(symbol);

    // ── Label, strength bar and status, under the counter ─────────────────
    // The turn game's bar, same look: absolute so it cannot push the symbol
    // about, inline-block so the bar is exactly as wide as the name, side
    // colour for the fill (length carries the health), and ONLY on own units
    // or in the umpire's view — how badly the enemy is hurt is not something
    // this side can see. Here the bar counts VEHICLES still fighting, and a
    // thin amber line under it shows suppression.
    const halo = "text-shadow:0 0 2px #0d1017,0 0 2px #0d1017,0 0 2px #0d1017";
    const caption = document.createElement("div");
    caption.style.cssText =
      "position:absolute;top:100%;left:50%;transform:translate(-50%,3px);" +
      "display:inline-block;white-space:nowrap;pointer-events:none;line-height:1";
    const name = document.createElement("div");
    name.style.cssText = `font-size:9px;text-align:center;color:#e6e9f2;${halo}`;
    caption.appendChild(name);

    const track = document.createElement("div");
    track.style.cssText =
      "position:relative;width:100%;min-width:34px;height:9px;margin-top:1px;" +
      "background:rgba(13,16,23,0.85);border:1px solid rgba(255,255,255,0.22);" +
      "border-radius:1px;overflow:hidden";
    const fill = document.createElement("div");
    fill.style.cssText = `position:absolute;inset:0 auto 0 0;background:${SIDE_COLOUR[sideOf]}`;
    track.appendChild(fill);
    const figure = document.createElement("div");
    figure.style.cssText =
      "position:absolute;inset:0;display:flex;align-items:center;justify-content:center;" +
      `font-size:7px;font-weight:600;color:#f2f4fa;${halo}`;
    track.appendChild(figure);
    caption.appendChild(track);

    const pinned = document.createElement("div");
    pinned.style.cssText = "height:2px;margin-top:1px;background:#e8c547;width:0";
    caption.appendChild(pinned);

    const status = document.createElement("div");
    status.style.cssText = `font-size:8px;text-align:center;color:#c7ccdb;margin-top:1px;${halo}`;
    caption.appendChild(status);
    inner.appendChild(caption);

    const tag = document.createElement("div");
    tag.style.cssText =
      "position:absolute;right:100%;bottom:100%;transform:translate(4px,4px);font-size:8px;" +
      "line-height:1;padding:1px 3px;border-radius:2px;white-space:nowrap;pointer-events:none;" +
      "background:rgba(13,16,23,0.9);display:none";
    inner.appendChild(tag);

    node.addEventListener("click", (event) => {
      event.stopPropagation();
      if (phaseRef.current === "setup") setPlaced((current) => current.filter((one) => one.id !== id));
    });

    const marker = new maplibregl.Marker({ element: node }).setLngLat([at.lng, at.lat]).addTo(map);

    const update = (state: RtState) => {
      const fe = state.game.forceElements[id];
      if (!fe) return;
      const view = viewpointRef.current;
      const visible =
        view === "both" || fe.side === view || (state.game.sighting[view][id] ?? "none") !== "none";
      node.style.display = fe.combatStrength > 0 && visible ? "" : "none";
      marker.setLngLat([fe.position.lng, fe.position.lat]);
      const own = view === "both" || fe.side === view;
      const unit = state.units[id];
      node.style.opacity = unit?.cohesion === "broken" ? "0.45" : unit?.cohesion === "shaken" ? "0.7" : "1";
      name.textContent = own ? label : fe.id;
      const fit = unit?.vehicles.fit ?? fe.combatStrength;
      const total = unit?.vehicles.total ?? fe.combatStrengthStart;
      const fraction = total > 0 ? Math.max(0, Math.min(1, fit / total)) : 0;
      track.style.display = own ? "" : "none";
      fill.style.width = `${(fraction * 100).toFixed(1)}%`;
      // Below a third it dims — the turn game's one health cue that needs no new colour.
      fill.style.opacity = fraction < 1 / 3 ? "0.55" : "1";
      figure.textContent = `${fit}/${total}`;
      track.title = `${fit} of ${total} vehicles still fighting · strength ${fe.combatStrength}/${fe.combatStrengthStart}`;
      pinned.style.display = own ? "" : "none";
      pinned.style.width = `${Math.round(Math.min(100, unit?.suppression ?? 0))}%`;
      pinned.style.opacity = (unit?.suppression ?? 0) >= PINNED_AT ? "1" : "0.6";
      status.style.display = own ? "" : "none";
      status.textContent = unit
        ? [
            ...(unit.cohesion !== "steady" ? [unit.cohesion.toUpperCase()] : []),
            ...(unit.suppression >= PINNED_AT ? ["pinned"] : unit.suppression >= SUPPRESSED_AT ? ["suppressed"] : []),
            ...(unit.posture === "hullDown" ? ["hull-down"] : []),
            unit.orders && (unit.orders.done || unit.orders.blocked?.phase === unit.orders.phase)
              ? unit.orders.blocked ? "stuck: needs orders" : "needs orders"
              : activityOf(unit),
          ].join(" · ")
        : "";

      // The latest decision, for a while after it was made. Own side only:
      // what the enemy decided is not something this side can see.
      const runner = runnerRef.current;
      const decision = own
        ? [...(runner?.log ?? [])]
            .reverse()
            .find((entry) => entry.type === "decision" && entry.decision.unitId === id)
        : undefined;
      if (decision && decision.type === "decision" && state.time - decision.time <= DECISION_TAG_S) {
        const d = decision.decision;
        const p = d.trace.probabilities?.[d.optionId];
        tag.textContent = `${decision.summary.slice(0, 28)}${p != null ? ` ${Math.round(p * 100)}%` : ""}`;
        tag.title = describeEntry(decision);
        tag.style.display = "";
        tag.style.color = d.trace.chosenBy === "jev" ? "#e8c547" : "#8a91a8";
        tag.style.border = d.trace.chosenBy === "jev" ? "1px solid rgba(232,197,71,0.5)" : "1px dashed rgba(255,255,255,0.3)";
      } else {
        tag.style.display = "none";
      }
    };
    return { marker, update };
  }, []);

  /** Move every counter to where it is now, and redraw the order and fire lines. */
  const draw = useCallback(() => {
    const map = liveMap(mapRef.current);
    if (!map) return;
    const state = runnerRef.current?.state ?? orderedRef.current;
    if (!state) return;

    for (const fe of Object.values(state.game.forceElements)) {
      let entry = markersRef.current.get(fe.id);
      if (!entry) {
        entry = makeMarker(map, fe.id, fe.side, fe.label, fe.position);
        markersRef.current.set(fe.id, entry);
      }
      entry.update(state);
    }

    const view = viewpointRef.current;
    const own = (s: Side) => view === "both" || s === view;
    const orders = Object.entries(state.units).flatMap(([id, unit]) => {
      const fe = state.game.forceElements[id];
      if (!fe || fe.combatStrength <= 0 || !own(fe.side)) return [];
      if (unit.order.kind !== "move" && unit.order.kind !== "withdraw") return [];
      // The route it will actually walk, not a straight line to the goal.
      const path = [fe.position, ...(unit.order.route ?? []), unit.order.to];
      return [
        {
          type: "Feature" as const,
          properties: { side: fe.side },
          geometry: {
            type: "LineString" as const,
            coordinates: path.map((point) => [point.lng, point.lat]),
          },
        },
      ];
    });
    (map.getSource(ORDER_SOURCE) as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: orders,
    });

    // Last-known positions: where a contact was when it was last seen.
    const ghosts = (["blue", "red"] as const)
      .filter((viewer) => own(viewer))
      .flatMap((viewer) =>
        Object.entries(state.lastKnown?.[viewer] ?? {})
          .filter(([id]) => (state.game.sighting[viewer][id] ?? "none") === "none")
          .map(([id, seen]) => ({
            type: "Feature" as const,
            properties: {
              id,
              side: viewer === "blue" ? "red" : "blue",
              opacity: Math.max(0.2, 1 - (state.time - seen.time) / 600),
            },
            geometry: { type: "Point" as const, coordinates: [seen.at.lng, seen.at.lat] },
          })),
      );
    (map.getSource(LAST_KNOWN_SOURCE) as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: ghosts,
    });

    const runner = runnerRef.current;
    const fire = (runner?.log ?? [])
      .filter((entry) => entry.type === "shot" && state.time - entry.time <= FIRE_LINE_S)
      .flatMap((entry) => {
        if (entry.type !== "shot") return [];
        const from = state.game.forceElements[entry.shot.firerId];
        const to = state.game.forceElements[entry.shot.targetId];
        if (!from || !to) return [];
        // A shot is seen by its target's side as well as its own.
        if (!own(from.side) && !own(to.side)) return [];
        return [
          {
            type: "Feature" as const,
            properties: { side: from.side, opacity: 1 - (state.time - entry.time) / FIRE_LINE_S },
            geometry: {
              type: "LineString" as const,
              coordinates: [
                [from.position.lng, from.position.lat],
                [to.position.lng, to.position.lat],
              ],
            },
          },
        ];
      });
    (map.getSource(FIRE_SOURCE) as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: fire,
    });
  }, [makeMarker]);
  const drawRef = useRef(draw);
  drawRef.current = draw;

  // Setup: counters for what is placed. Rebuilt on change, which is fine at setup.
  useEffect(() => {
    const map = liveMap(mapRef.current);
    if (!map || phase !== "setup") return;
    for (const entry of markersRef.current.values()) entry.marker.remove();
    markersRef.current.clear();
    const preview = createRealtimeState(toGameStateFromPlaced(placed, HOUSE_V1, { gameId: "preview" }));
    for (const fe of Object.values(preview.game.forceElements)) {
      const entry = makeMarker(map, fe.id, fe.side, fe.label, fe.position);
      entry.update(preview);
      markersRef.current.set(fe.id, entry);
    }
  }, [placed, phase, mapEpoch, makeMarker]);

  // The board follows the data: on real ground it recentres on the raster's
  // coverage, and the outline and the camera have to come along.
  useEffect(() => {
    const map = liveMap(mapRef.current);
    if (!map) return;
    (map.getSource(BOARD_SOURCE) as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "Feature",
      properties: {},
      geometry: { type: "Polygon", coordinates: [boardRing(bounds)] },
    });
    map.flyTo({ center: [origin.lng, origin.lat], zoom: 12, duration: 600 });
  }, [bounds, origin, mapEpoch]);

  // Orders generated or the viewpoint changed: redraw once.
  useEffect(() => {
    if (phase !== "setup") draw();
  }, [phase, viewpoint, draw, mapEpoch]);

  // ── Panel ─────────────────────────────────────────────────────────────────

  const runner = runnerRef.current;
  const state = runner?.state ?? orderedRef.current;
  const strength = (s: Side) =>
    state
      ? Object.values(state.game.forceElements)
          .filter((fe) => fe.side === s)
          .reduce((sum, fe) => sum + fe.combatStrength, 0)
      : 0;
  const feed = (runner?.log ?? [])
    .filter((entry) => viewpoint === "both" || entry.side === viewpoint)
    .slice(-60)
    .reverse();
  const counts = { blue: placed.filter((p) => p.side === "blue").length, red: placed.filter((p) => p.side === "red").length };
  const decisions = (runner?.log ?? []).filter((entry) => entry.type === "decision");
  const byJev = decisions.filter((entry) => entry.type === "decision" && entry.decision.trace.chosenBy === "jev").length;
  // The latest decision, if Jev failed on it: the reason, on screen rather than only in the console.
  const lastDecision = decisions[decisions.length - 1];
  const jevProblem =
    useJev && lastDecision?.type === "decision" && lastDecision.decision.trace.fallback === "error"
      ? jevFailure(lastDecision.decision.trace.rationale)
      : null;
  const view = state && viewpoint !== "both" ? projectForSide(state.game, viewpoint) : null;

  return (
    <DechoBasemap
      tiles={planetStore}
      spawnLat={DEFAULT_ORIGIN.lat}
      spawnLong={DEFAULT_ORIGIN.lng}
      spawnZoom={12}
      onMapReady={onMapReady}
      style={{ height: "100vh" }}
    >
      <div style={header}>
        <button onClick={() => setParams({})} style={{ ...linkButton }}>
          &larr; modes
        </button>
        <Link to="/bgws" style={{ ...subtle, color: "#e8c547", textDecoration: "none" }}>
          BGWS
        </Link>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>REAL-TIME</span>
        <span style={subtle}>
          {phase === "setup"
            ? `placing · blue ${counts.blue} · red ${counts.red} units`
            : `${clock(state?.time ?? 0)} · blue ${strength("blue")} · red ${strength("red")} CS`}
        </span>
        {runner?.thinking && <span style={{ ...subtle, color: "#e8c547" }}>Jev deciding&hellip;</span>}
        {jevProblem && (
          <span style={{ ...subtle, color: "#e07a5f", maxWidth: 520, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }} title={jevProblem}>
            Jev not answering: {jevProblem}
          </span>
        )}
        {state?.over && (
          <span style={{ ...subtle, color: "#e8c547" }}>
            {state.over.winner ?? "drawn"} &mdash; {state.over.reason}
          </span>
        )}
        <span style={{ flex: 1 }} />
        <AppSwitcher />
      </div>

      <div style={leftPane}>
        {phase === "setup" && (
          <>
            <div style={groupTitle}>1 &middot; Place units</div>
            <div style={row}>
              {(["blue", "red"] as Side[]).map((s) => (
                <button
                  key={s}
                  onClick={() => setSide(s)}
                  style={{ ...chip, flex: 1, borderColor: side === s ? SIDE_COLOUR[s] : "transparent", color: side === s ? SIDE_COLOUR[s] : "#8a91a8" }}
                >
                  {s}
                </button>
              ))}
            </div>
            <select value={platform} onChange={(e) => setPlatform(e.target.value)} style={{ ...select, marginBottom: 4 }}>
              {platforms.map((p) => (
                <option key={p.assetId} value={p.assetId}>
                  {p.displayName}
                </option>
              ))}
            </select>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>count</span>
              <input type="number" min={1} max={12} value={count} onChange={(e) => setCount(Math.max(1, Number(e.target.value)))} style={select} />
            </div>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>quality</span>
              <select value={quality} onChange={(e) => setQuality(e.target.value as TroopQualityName)} style={select}>
                {(Object.keys(TROOP_QUALITY) as TroopQualityName[]).map((q) => (
                  <option key={q} value={q}>
                    {q}
                  </option>
                ))}
              </select>
            </div>
            <div style={{ ...subtle, lineHeight: 1.5, margin: "4px 0 8px" }}>
              Click the map to place; click a counter to remove it. Or start from a force list:
            </div>
            {Object.values(FORCE_LISTS).map((list) => (
              <button key={list.id} onClick={() => setPlaced(onBoard(placedFromList(list), origin))} style={{ ...chip, display: "block", width: "100%", textAlign: "left", marginBottom: 2 }}>
                {list.name}
              </button>
            ))}
            {placed.length > 0 && (
              <button onClick={() => setPlaced([])} style={{ ...chip, marginTop: 4 }}>
                clear all ({placed.length})
              </button>
            )}

            <div style={{ ...groupTitle, marginTop: 14 }}>2 &middot; Who decides</div>
            <label style={{ ...row, gap: 6, color: useCommander ? "#e8c547" : "#8a91a8", cursor: "pointer" }}>
              <input type="checkbox" checked={useCommander} onChange={(e) => setUseCommander(e.target.checked)} />
              Commander writes the orders
            </label>
            {useCommander && (
              <select value={model} onChange={(e) => setModel(e.target.value as CommanderModelName)} style={{ ...select, marginBottom: 4 }}>
                {COMMANDER_MODELS.map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
              </select>
            )}
            <div style={{ ...subtle, lineHeight: 1.5, marginBottom: 6 }}>
              {useCommander
                ? "The commander writes each unit's mission orders — task, intent, urgency, limits — before the start and " +
                  "whenever you pause and ask. It never runs while the clock does."
                : "Simple orders: everyone advances on the objective and holds it."}
            </div>
            <label style={{ ...row, gap: 6, color: useJev ? "#e8c547" : "#8a91a8", cursor: "pointer" }}>
              <input type="checkbox" checked={useJev} onChange={(e) => setUseJev(e.target.checked)} />
              Jev leads each unit
            </label>
            <div style={{ ...subtle, lineHeight: 1.5, color: useJev && !jevConfigured() ? "#e07a5f" : undefined }}>
              {useJev
                ? jevConfigured()
                  ? "Jev makes the call at each decision point — new contact, fired on, a trigger met, a sign it " +
                    "has been seen — always inside the orders. Every call is printed to the browser console."
                  : "No OpenRouter key: every Jev decision will fall back to the rules."
                : "The rules make every call, following each order's actions on contact."}
            </div>
            {([
              ["blue", blueDirective, setBlueDirective],
              ["red", redDirective, setRedDirective],
            ] as const).map(([s, value, set]) => (
              <textarea
                key={s}
                value={value}
                onChange={(e) => set(e.target.value)}
                placeholder={`${s} directive — mission, doctrine, temperament`}
                rows={2}
                style={{ ...select, width: "100%", marginTop: 4, resize: "vertical" }}
              />
            ))}

            <div style={{ ...groupTitle, marginTop: 14 }}>3 &middot; Ground and dice</div>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>terrain</span>
              <select
                value={groundChoice}
                onChange={(e) => setGroundChoice(e.target.value as GroundChoice)}
                style={select}
              >
                <option value="raster+relief">real cover + generated relief</option>
                <option value="raster">real cover, flat</option>
                <option value="generated">generated ground</option>
              </select>
            </div>
            <div style={{ ...subtle, lineHeight: 1.5, paddingBottom: 4 }}>
              {describeTerrainSource(effectiveGround, DEFAULT_TERRAIN.label, groundSeed)}
              {effectiveGround !== "generated"
                ? " \u2014 routes are planned on the raster, and rivers are impassable."
                : " \u2014 not the map underneath: routes are planned on the generated ground."}
              {wantsRaster && !rasterLoaded && !rasterError && " \u2014 loading the raster\u2026"}
              {rasterError && (
                <span style={{ color: "#e8945a" }}>
                  {" "}&mdash; raster unavailable ({rasterError.message}); using generated ground. The app
                  needs {DEFAULT_TERRAIN.label} added as a permitted resource in Developer Console.
                </span>
              )}
              {wantsRaster && rasterLoaded && !rasterUsable && !rasterError && (
                <span style={{ color: "#e8945a" }}> &mdash; the raster does not cover a full board; using generated ground.</span>
              )}
            </div>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>relief</span>
              <input value={groundSeed} onChange={(e) => setGroundSeed(e.target.value)} style={select} />
            </div>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>seed</span>
              <input value={gameSeed} onChange={(e) => setGameSeed(e.target.value)} style={select} />
            </div>

            <button
              onClick={generateOrders}
              disabled={busy || counts.blue === 0 || counts.red === 0}
              style={{ ...primary, marginTop: 12, opacity: busy || counts.blue === 0 || counts.red === 0 ? 0.5 : 1 }}
            >
              {busy ? "writing orders…" : "Generate orders"}
            </button>
          </>
        )}

        {phase !== "setup" && (
          <>
            <div style={groupTitle}>Plan</div>
            {(["blue", "red"] as Side[]).map((s) => (
              <div key={s} style={{ ...subtle, color: SIDE_COLOUR[s], lineHeight: 1.5, marginBottom: 4 }}>
                {s}: {state?.plan[s] ?? "—"}
              </div>
            ))}

            <div style={{ ...groupTitle, marginTop: 10 }}>Clock</div>
            {phase === "ordered" ? (
              <>
                <div style={{ ...subtle, lineHeight: 1.5, marginBottom: 6 }}>
                  Opening orders are drawn on the map. Review them, then press Play to start the clock, or go back and change them.
                </div>
                <OrdersReview results={draft} state={state} />
                <button onClick={start} style={primary}>
                  &#9654; Play
                </button>
                <button onClick={() => setPhase("setup")} style={{ ...chip, marginTop: 6 }}>
                  back to setup
                </button>
              </>
            ) : (
              <>
                <div style={row}>
                  <button
                    onClick={() => (playing ? setPlaying(false) : resume())}
                    disabled={!!state?.over || busy}
                    style={{ ...primary, flex: 1, marginTop: 0 }}
                  >
                    {state?.over ? "Game over" : playing ? "❚❚ Pause" : draft.length ? "▶ Issue orders and resume" : "▶ Play"}
                  </button>
                </div>
                {state?.over && (
                  <div style={{ margin: "6px 0 8px", padding: 6, border: "1px solid rgba(232,197,71,0.5)", borderRadius: 3, ...subtle, color: "#e8c547", lineHeight: 1.5 }}>
                    Game over at {clock(state.time)}: {state.over.winner ? `${state.over.winner} wins` : "drawn"} — {state.over.reason}.
                    {state.over.reason.includes("breakpoint")
                      ? " A side is beaten when half its strength is destroyed or broken; with one troop a side, one troop breaking ends it."
                      : ""}
                  </div>
                )}
                {!playing && !state?.over && (
                  <div style={{ margin: "6px 0 8px", padding: 6, border: "1px solid #191e37", borderRadius: 3 }}>
                    <div style={{ ...subtle, color: "#c7ccdb", marginBottom: 4 }}>Paused: new orders?</div>
                    <div style={row}>
                      {(["both", "blue", "red"] as Viewpoint[]).map((v) => (
                        <button
                          key={v}
                          onClick={() => setOrderSides(v)}
                          style={{ ...chip, flex: 1, borderColor: orderSides === v ? "#e8c547" : "transparent", color: orderSides === v ? "#e8c547" : "#8a91a8" }}
                        >
                          {v === "both" ? "both sides" : v}
                        </button>
                      ))}
                    </div>
                    <textarea
                      value={guidance}
                      onChange={(e) => setGuidance(e.target.value)}
                      placeholder="your guidance (optional): hold the bridge; the northern troop is too exposed"
                      rows={2}
                      style={{ ...select, width: "100%", resize: "vertical" }}
                    />
                    <button onClick={generatePausedOrders} disabled={busy} style={{ ...primary, opacity: busy ? 0.5 : 1 }}>
                      {busy ? "writing orders…" : "Generate orders"}
                    </button>
                    {draft.length > 0 && (
                      <>
                        <div style={{ ...subtle, lineHeight: 1.5, margin: "6px 0 4px" }}>
                          Review, then resume. A unit in a fight will be asked how to comply; one out of contact switches at once.
                        </div>
                        <OrdersReview results={draft} state={state} />
                        <button onClick={() => setDraft([])} style={{ ...chip, marginTop: 2 }}>
                          discard these orders
                        </button>
                      </>
                    )}
                  </div>
                )}
                <div style={row}>
                  {SPEEDS.map((x) => (
                    <button key={x} onClick={() => setSpeed(x)} style={{ ...chip, flex: 1, borderColor: speed === x ? "#e8c547" : "transparent", color: speed === x ? "#e8c547" : "#8a91a8" }}>
                      &times;{x}
                    </button>
                  ))}
                </div>
                <div style={{ ...subtle, lineHeight: 1.5 }}>
                  {clock(state?.time ?? 0)} simulated &middot; {decisions.length} decisions ({byJev} by Jev
                  {useJev && decisions.length > 0 && byJev === 0 ? " — every one fell back to the rules; see the reason above" : ""})
                  {runner && runner.waits > 0 ? ` · clock waited for Jev ${runner.waits}×` : ""}
                </div>
                <button onClick={reset} style={{ ...chip, marginTop: 6 }}>
                  back to setup
                </button>
              </>
            )}

            <div style={{ ...groupTitle, marginTop: 10 }}>View</div>
            <div style={row}>
              {(["both", "blue", "red"] as Viewpoint[]).map((v) => (
                <button key={v} onClick={() => setViewpoint(v)} style={{ ...chip, flex: 1, borderColor: viewpoint === v ? "#e8c547" : "transparent", color: viewpoint === v ? "#e8c547" : "#8a91a8" }}>
                  {v === "both" ? "umpire" : `${v} eyes`}
                </button>
              ))}
            </div>
            {view && (
              <div style={{ ...subtle, lineHeight: 1.5 }}>
                {view.contacts.length} enemy contact{view.contacts.length === 1 ? "" : "s"} in sight
                {state && viewpoint !== "both"
                  ? ` · ${Object.keys(state.lastKnown?.[viewpoint] ?? {}).filter((id) => (state.game.sighting[viewpoint][id] ?? "none") === "none").length} lost (rings)`
                  : ""}
              </div>
            )}
            {state && phase === "running" && (
              <div style={{ ...subtle, lineHeight: 1.5 }}>
                {(["blue", "red"] as const).map((side) => {
                  const fighting = Object.values(state.game.forceElements)
                    .filter((fe) => fe.side === side && fe.combatStrength > 0 && state.units[fe.id]?.cohesion !== "broken")
                    .reduce((sum, fe) => sum + fe.combatStrength, 0);
                  const start = Math.max(1, state.startStrength?.[side] ?? 1);
                  return `${side} ${Math.round((fighting / start) * 100)}%`;
                }).join(" · ")}{" "}
                fighting strength (a side breaks below 50%)
              </div>
            )}
          </>
        )}
        {error && (
          <div style={{ ...subtle, color: "#e07a5f", marginTop: 6, lineHeight: 1.5 }}>
            {phase === "running" ? "The clock stopped because of an error: " : ""}
            {error}
          </div>
        )}
      </div>

      {phase === "running" && (
        <div style={rightPane}>
          {runner && runner.flags.length > 0 && (
            <>
              <div style={groupTitle}>Flags</div>
              {runner.flags
                .filter((flag) => viewpoint === "both" || flag.side === viewpoint)
                .slice(-6)
                .reverse()
                .map((flag, index) => (
                  <div key={`${flag.time}-${index}`} style={{ ...subtle, color: "#e8c547", lineHeight: 1.45 }}>
                    {clock(flag.time)} ⚑ {flag.text}
                  </div>
                ))}
              <div style={{ ...subtle, lineHeight: 1.45, marginBottom: 8 }}>Pause to give new orders.</div>
            </>
          )}
          <div style={groupTitle}>What is happening</div>
          {feed.length === 0 && <div style={subtle}>Nothing yet.</div>}
          {feed.map((entry, index) => (
            <div
              key={`${entry.time}-${index}`}
              style={{
                ...subtle,
                lineHeight: 1.45,
                color:
                  entry.type === "decision"
                    ? entry.decision.trace.chosenBy === "jev"
                      ? "#e8c547"
                      : "#b8bdd0"
                    : entry.type === "shot"
                      ? SIDE_COLOUR[entry.side]
                      : entry.type === "flag"
                        ? "#e8c547"
                        : "#8a91a8",
              }}
            >
              {describeEntry(entry)}
            </div>
          ))}
        </div>
      )}
      {phase === "running" && state?.over && !endDismissed && (
        <EndOverlay
          state={state}
          decisions={decisions.length}
          byJev={byJev}
          onRunAgain={runAgain}
          onEdit={reset}
          onNew={newScenario}
          onClose={() => setEndDismissed(true)}
        />
      )}
    </DechoBasemap>
  );
}

// ── Styles ──────────────────────────────────────────────────────────────────

const OVERLAY = "rgba(6,13,24,0.92)";
const MONO = "11px/1.4 var(--font-mono, monospace)";

const header: React.CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  right: 0,
  height: 40,
  zIndex: 3,
  display: "flex",
  alignItems: "center",
  gap: 12,
  padding: "0 12px",
  background: OVERLAY,
  borderBottom: "1px solid #191e37",
  color: "#e9ecfb",
  font: MONO,
};

const leftPane: React.CSSProperties = {
  position: "absolute",
  top: 40,
  left: 0,
  bottom: 0,
  width: 262,
  zIndex: 3,
  padding: 10,
  overflowY: "auto",
  background: OVERLAY,
  borderRight: "1px solid #191e37",
  color: "#e9ecfb",
  font: MONO,
};

const rightPane: React.CSSProperties = {
  ...leftPane,
  left: "auto",
  right: 0,
  width: 340,
  borderRight: "none",
  borderLeft: "1px solid #191e37",
};

const groupTitle: React.CSSProperties = {
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.12em",
  textTransform: "uppercase",
  color: "rgba(255,255,255,0.45)",
  borderBottom: "1px solid #191e37",
  paddingBottom: 4,
  marginBottom: 6,
};

const row: React.CSSProperties = { display: "flex", alignItems: "center", gap: 4, marginBottom: 4 };

const subtle: React.CSSProperties = { fontSize: 10, color: "#6a7292" };

const select: React.CSSProperties = {
  boxSizing: "border-box",
  padding: "4px 6px",
  background: "rgba(255,255,255,0.06)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
  font: "inherit",
  fontSize: 10,
  width: "100%",
};

const chip: React.CSSProperties = {
  padding: "3px 7px",
  background: "rgba(255,255,255,0.06)",
  border: "1px solid",
  borderColor: "transparent",
  borderRadius: 3,
  color: "#8a91a8",
  cursor: "pointer",
  font: "inherit",
  fontSize: 10,
};

const primary: React.CSSProperties = {
  width: "100%",
  marginTop: 6,
  padding: "7px 10px",
  background: "rgba(232,197,71,0.14)",
  border: "1px solid rgba(232,197,71,0.5)",
  borderRadius: 3,
  color: "#e8c547",
  cursor: "pointer",
  font: "inherit",
  fontSize: 11,
  fontWeight: 700,
};

const overlay: React.CSSProperties = {
  position: "absolute",
  inset: 0,
  zIndex: 10,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  padding: 16,
  background: "rgba(4,8,15,0.72)",
  backdropFilter: "blur(2px)",
  font: MONO,
};

const overlayCard: React.CSSProperties = {
  width: "min(520px, 100%)",
  padding: "18px 20px",
  background: "rgba(10,16,28,0.98)",
  border: "1px solid #2a3150",
  borderRadius: 4,
  boxShadow: "0 12px 40px rgba(0,0,0,0.5)",
  color: "#e9ecfb",
};

const overlayButton: React.CSSProperties = {
  padding: "7px 10px",
  background: "rgba(255,255,255,0.06)",
  border: "1px solid rgba(255,255,255,0.2)",
  borderRadius: 3,
  color: "#e9ecfb",
  cursor: "pointer",
  font: "inherit",
  fontSize: 11,
  fontWeight: 700,
};

const linkButton: React.CSSProperties = {
  background: "none",
  border: "none",
  padding: 0,
  color: "#e8c547",
  cursor: "pointer",
  font: "inherit",
  fontSize: 10,
};
