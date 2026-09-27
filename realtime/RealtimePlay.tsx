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
import { activityOf, createRealtimeState, describeOrder } from "./engine/engine";
import { agoBand, beliefsOf, selfBeliefOf, trackErrorM } from "./engine/knowledge";
import { offsetBy } from "./engine/geometry";
import { jevRealtimeDecider } from "./engine/jevDecider";
import { applyOrders, commanderOrders, heuristicOrders, type OrdersResult } from "./engine/orders";
import { EVENT_STREAM_RID, STREAM_MESSAGES, foundryStreamPublisher } from "../data/eventStream";
import { EventStreamQueue } from "../data/eventStreamQueue";
import { describeEntry, jevFailure, streamRow } from "./engine/feed";
import { OVERLAY_SOURCES, overlayLayerSpecs, SOURCE } from "./overlayLayers";
import {
  fireOverlay,
  newWrecks,
  planOverlay,
  radioOverlay,
  spottingOverlay,
  wreckOverlay,
  type Wreck,
} from "./overlays";
import { RealtimeRunner, type RunnerOptions } from "./engine/runner";
import { clock, DEFAULT_TIMING, PINNED_AT, SUPPRESSED_AT } from "./engine/timing";
import type { RtConfig, RtState } from "./engine/types";

type Phase = "setup" | "ordered" | "running";
type Viewpoint = Side | "both";

const BOARD_SOURCE = "rt-board";
const ORDER_SOURCE = "rt-orders";
/** Where faded contacts were last seen. */
const LAST_KNOWN_SOURCE = "rt-last-known";
/** The selected unit's own picture: where it believes each enemy is, and how sure. */
const PICTURE_SOURCE = "rt-picture";
const SPEEDS = [1, 5, 10, 30, 60];
/** How long a decision's tag stays beside its counter, in simulated seconds. */
const DECISION_TAG_S = 40;
const SIDE_COLOUR: Record<Side, string> = { blue: "#8fc2ff", red: "#ff9e8f" };

/** The ground choices offered here: real land cover, or generated. */
type GroundChoice = Extract<TerrainSource, "generated" | "raster+relief" | "raster">;

/** The counter shake when a vehicle is knocked out: defined once for the page. */
if (typeof document !== "undefined" && !document.getElementById("rt-keyframes")) {
  const style = document.createElement("style");
  style.id = "rt-keyframes";
  style.textContent =
    "@keyframes rt-shake{0%,100%{transform:translate(0,0)}20%{transform:translate(-3px,1px)}40%{transform:translate(3px,-1px)}" +
    "60%{transform:translate(-2px,-1px)}80%{transform:translate(2px,1px)}}";
  document.head.appendChild(style);
}

let counter = 0;
const nextId = () => `rt-${(counter += 1)}`;

function liveMap(map: maplibregl.Map | null): maplibregl.Map | null {
  if (!map) return null;
  return (map as unknown as { style?: unknown }).style ? map : null;
}

function emptyCollection(): GeoJSON.FeatureCollection {
  return { type: "FeatureCollection", features: [] };
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

/** What the map's marks mean (overlayLayers.ts). Collapsible, and remembered closed for the page. */
function MapKey() {
  const [open, setOpen] = useState(true);
  const swatch = (style: React.CSSProperties) => (
    <span style={{ display: "inline-block", width: 22, height: 10, marginRight: 6, verticalAlign: "middle", ...style }} />
  );
  const lineSwatch = (colour: string, dashed = false, width = 2) =>
    swatch({ borderTop: `${width}px ${dashed ? "dashed" : "solid"} ${colour}`, height: 0, marginTop: 5 });
  const dot = (colour: string, ring = false) =>
    swatch({
      width: 10,
      borderRadius: "50%",
      background: ring ? "transparent" : colour,
      border: `2px solid ${colour}`,
      marginLeft: 6,
      marginRight: 12,
    });
  const rows: [React.ReactNode, string][] = [
    [lineSwatch("#c7ccdb", true, 1), "shot that missed"],
    [lineSwatch("#c7ccdb", false, 2), "shot that struck"],
    [dot("#d7dbe6", true), "round bounced off"],
    [dot("#ff9f43", true), "round got through"],
    [dot("#ff4d4d"), "vehicle knocked out"],
    [dot("#8a8f99"), "wreck (smoke for a minute)"],
    [lineSwatch("#e8c547", true), "spotted an enemy"],
    [lineSwatch("#ff6b6b", true), "found who was firing on it"],
    [swatch({ background: "rgba(255,159,67,0.35)" }), "fired on from here, shooter unseen"],
    [lineSwatch("#5fd4c0", true, 1), "radio: contact report (faint: to all)"],
    [lineSwatch("#ff9f43", true, 1), "radio: under fire"],
    [lineSwatch("#e879f9", true, 2), "radio: request for help"],
    [lineSwatch("#c4b5fd", true, 2), "radio: reply"],
    [lineSwatch("#ffffff", false, 2), "selected: what it is doing now"],
    [lineSwatch("#ffffff", true, 2), "selected: its next steps"],
    [lineSwatch("#ff6b6b", true, 2), "selected: a line it may not cross"],
    [swatch({ border: "1px dashed #e8c547", background: "rgba(232,197,71,0.1)" }), "selected: its trigger"],
  ];
  return (
    <>
      <div style={{ ...groupTitle, marginTop: 10, cursor: "pointer" }} onClick={() => setOpen((o) => !o)}>
        Map key {open ? "▾" : "▸"}
      </div>
      {open &&
        rows.map(([mark, text]) => (
          <div key={text} style={{ ...subtle, color: "#b8bdd0", lineHeight: 1.7, display: "flex", alignItems: "center" }}>
            {mark}
            {text}
          </div>
        ))}
    </>
  );
}

/** One labelled line in the inspector. */
function Line({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div style={{ display: "flex", gap: 6, lineHeight: 1.45, marginBottom: 2 }}>
      <span style={{ ...subtle, width: 74, flex: "0 0 auto" }}>{label}</span>
      <span style={{ ...subtle, color: "#c7ccdb" }}>{children}</span>
    </div>
  );
}

const ROE_WORDS: Record<string, string> = {
  never: "weapons hold",
  ifFiredUpon: "only once fired upon",
  withinShortRange: "anything inside short range",
  always: "anything in reach",
};

/**
 * One unit, as it stands: what it is doing, its orders and how far it has
 * got, its state, what it knows and thinks, its fire, and its last decisions.
 * An enemy seen through one side's eyes shows only what that side knows.
 */
function UnitInspector(props: {
  state: RtState;
  id: string;
  viewpoint: Viewpoint;
  awaitingOrders: boolean;
  onClose: () => void;
}) {
  const { state, id, viewpoint } = props;
  const fe = state.game.forceElements[id];
  const unit = state.units[id];
  if (!fe) return null;
  const header = (
    <div style={{ ...row, justifyContent: "space-between", marginBottom: 6 }}>
      <span style={{ fontSize: 11, fontWeight: 700, color: SIDE_COLOUR[fe.side] }}>
        {fe.label} <span style={{ ...subtle }}>({fe.id})</span>
      </span>
      <button onClick={props.onClose} style={{ ...linkButton, color: "#8a91a8" }} aria-label="Close">
        ✕
      </button>
    </div>
  );

  // Fog of war: an enemy is only what this side has seen of it.
  if (viewpoint !== "both" && fe.side !== viewpoint) {
    const level = state.game.sighting[viewpoint][id] ?? "none";
    const lost = state.lastKnown?.[viewpoint]?.[id];
    return (
      <>
        {header}
        <Line label="known as">
          {level === "full" ? "identified" : level === "none" ? (lost ? "lost contact" : "not seen") : "located, type unknown"}
        </Line>
        {level === "none" && lost && <Line label="last seen">{agoBand(state.time - lost.time)}</Line>}
        {level === "full" && unit && (
          <Line label="vehicles">
            {unit.vehicles.fit} of {unit.vehicles.total} still fighting
            {unit.cohesion !== "steady" ? ` · visibly ${unit.cohesion}` : ""}
          </Line>
        )}
        <div style={{ ...subtle, lineHeight: 1.5, marginTop: 4 }}>Switch to the umpire view to see its orders and state.</div>
      </>
    );
  }

  if (fe.combatStrength <= 0 || !unit) {
    return (
      <>
        {header}
        <div style={{ ...subtle, color: "#e07a5f" }}>Destroyed.</div>
      </>
    );
  }

  const orders = unit.orders;
  const suppression =
    unit.suppression >= PINNED_AT ? "pinned" : unit.suppression >= SUPPRESSED_AT ? "suppressed" : unit.suppression > 0 ? "under some pressure" : "none";
  const belief = selfBeliefOf(unit, state.time);
  const e = unit.engagement;
  const firing = e && state.time - e.lastShotAt <= 60 ? e : null;
  const incoming = Object.entries(unit.incoming).filter(([, fire]) => state.time - fire.last <= 120);
  const knows = beliefsOf(state, id);

  return (
    <>
      {header}
      <Line label="doing">
        {activityOf(unit)} — {describeOrder(unit.order)}
      </Line>
      <Line label="state">
        {unit.vehicles.fit} of {unit.vehicles.total} vehicles · {unit.cohesion} · suppression {suppression} · {unit.posture}
      </Line>

      <div style={{ ...groupTitle, marginTop: 8 }}>Orders</div>
      {orders ? (
        <>
          <Line label="task">{orders.task}</Line>
          <Line label="step">
            {orders.done
              ? "all done: needs new orders"
              : `${orders.phase + 1} of ${orders.phases.length}: ${orders.phases[orders.phase]?.label ?? "—"}`}
            {orders.blocked?.phase === orders.phase ? ` — blocked (${orders.blocked.why})` : ""}
          </Line>
          {orders.phases.length > 1 && (
            <Line label="then">
              {orders.phases
                .slice(orders.phase + 1)
                .map((p) => p.label)
                .join(" → ") || "—"}
            </Line>
          )}
          {orders.intent && <Line label="intent">{orders.intent}</Line>}
          <Line label="urgency">{orders.urgency === "now" ? "now" : "when able"}</Line>
          <Line label="fires at">{ROE_WORDS[orders.roe] ?? orders.roe}</Line>
          <Line label="on contact">{orders.onContact}</Line>
          {orders.supports && <Line label="supports">{orders.supports}: its requests come here first</Line>}
          {orders.boundaries.length > 0 && (
            <Line label="limits">{orders.boundaries.map((b) => `stay ${b.keep} of ${b.label}`).join("; ")}</Line>
          )}
          <Line label="from">
            {orders.by === "claude" ? "the commander" : orders.by} at {clock(orders.issuedAt)}
          </Line>
        </>
      ) : (
        <Line label="purpose">{unit.mission.purpose}</Line>
      )}
      {props.awaitingOrders && <div style={{ ...subtle, color: "#e8c547" }}>New orders waiting: it is being asked how to comply.</div>}

      <div style={{ ...groupTitle, marginTop: 8 }}>What it knows</div>
      <Line label="been seen?">
        {belief === "knownSeen" ? "yes: it has been fired on" : belief === "possiblySeen" ? `perhaps: ${unit.lastCue?.enemyId} ${unit.lastCue?.cue}` : "no sign of it"}
      </Line>
      {knows.length === 0 ? (
        <Line label="enemies">none known</Line>
      ) : (
        knows.map((k) => (
          <Line key={k.id} label={k.belief}>
            {"unit" in k && k.unit ? `${k.unit} ` : ""}
            {k.belief === "suspected" ? `to the ${"bearing" in k ? k.bearing : "?"} (${"why" in k ? k.why : ""})` : k.id}
            {"range" in k && k.range ? ` · ${k.range}` : ""}
            {"lastSeen" in k && k.lastSeen ? ` · last seen ${k.lastSeen}` : ""}
            {"source" in k && k.source ? ` · ${k.source}` : ""}
          </Line>
        ))
      )}

      {(unit.heard.length > 0 || unit.requests.length > 0 || Object.keys(unit.friendStatus).length > 0) && (
        <div style={{ ...groupTitle, marginTop: 8 }}>Radio</div>
      )}
      {unit.requests.map((r) => (
        <Line key={`r${r.id}`} label="asked">
          <span style={{ color: "#e8c547" }}>{r.text}</span> ({agoBand(state.time - r.time)})
        </Line>
      ))}
      {[...unit.heard].reverse().map((h, index) => (
        <Line key={`h${index}`} label={clock(h.time)}>
          {h.text}
        </Line>
      ))}
      {Object.entries(unit.friendStatus).map(([friend, status]) => (
        <Line key={`f${friend}`} label={friend}>
          {status.text.replace(`${friend}: `, "")} <span style={{ color: "#6a7292" }}>({agoBand(state.time - status.time)})</span>
        </Line>
      ))}

      {(firing || incoming.length > 0) && <div style={{ ...groupTitle, marginTop: 8 }}>Fire</div>}
      {firing && (
        <Line label="firing on">
          {firing.targetId} for {clock(state.time - firing.since)}: {firing.shots} volleys, {firing.hits} hits, {firing.damage} damage
        </Line>
      )}
      {incoming.map(([from, fire]) => (
        <Line key={from} label="fired on by">
          {state.game.sighting[fe.side][from] || unit.ownSeen[from] ? from : "an unseen enemy"}: {fire.shots} volleys, {fire.damage} damage
        </Line>
      ))}

      {unit.history.length > 0 && <div style={{ ...groupTitle, marginTop: 8 }}>Last decisions</div>}
      {[...unit.history].reverse().map((memory, index) => (
        <Line key={index} label={clock(memory.time)}>
          {memory.chose} <span style={{ color: memory.by === "jev" ? "#e8c547" : "#6a7292" }}>({memory.by}; {memory.because})</span>
        </Line>
      ))}
    </>
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
  /** Place it as an HQ: it relays radio traffic for its side (comms). */
  const [asHq, setAsHq] = useState(false);

  // Setup. Real land cover by default: the basemap under the counters is the
  // real world, and a game on generated ground over it has units crossing
  // rivers the rules do not know are there.
  const [groundChoice, setGroundChoice] = useState<GroundChoice>("raster+relief");
  const [groundSeed, setGroundSeed] = useState(STANDARD_GROUND.seed ?? "baltic-v1");
  const [gameSeed, setGameSeed] = useState("1");
  /** Win conditions: a side is beaten at this % of its strength destroyed or broken; the game ends at this many minutes. */
  const [breakpointPct, setBreakpointPct] = useState(50);
  /** How information travels between units: a radio net, or the old perfect sharing. */
  const [commsMode, setCommsMode] = useState<"radio" | "perfect">("radio");
  const [timeLimitMin, setTimeLimitMin] = useState(90);
  const [useJev, setUseJev] = useState(true);
  const [useCommander, setUseCommander] = useState(true);
  /** Stream the feed to the Foundry event stream as it happens. */
  const [streamLog, setStreamLog] = useState(true);
  const [model, setModel] = useState<CommanderModelName>(COMMANDER_MODELS[0]);
  const [blueDirective, setBlueDirective] = useState("");
  const [redDirective, setRedDirective] = useState("");
  /** Orders written and waiting for review: at the start, or while paused. */
  const [draft, setDraft] = useState<OrdersResult[]>([]);
  const [orderSides, setOrderSides] = useState<Viewpoint>("both");
  const [guidance, setGuidance] = useState("");

  // Running.
  const [speed, setSpeed] = useState(10);
  const speedRef = useRef(speed);
  speedRef.current = speed;
  const [playing, setPlaying] = useState(false);
  const [viewpoint, setViewpoint] = useState<Viewpoint>("both");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** The unit shown in the inspector under the feed; chosen by clicking its counter. */
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selectedRef = useRef<string | null>(null);
  selectedRef.current = selectedId;
  /** The end-of-game overlay, closed to look at the battlefield. */
  const [endDismissed, setEndDismissed] = useState(false);
  /** Bumped a few times a second so the panel reads the runner again. */
  const [, setFrame] = useState(0);

  const mapRef = useRef<maplibregl.Map | null>(null);
  const [mapEpoch, setMapEpoch] = useState(0);
  const markersRef = useRef(new Map<string, { marker: maplibregl.Marker; update: (s: RtState) => void }>());
  /** Wrecks seen so far this run, kept though the feed drops old lines. */
  const wrecksRef = useRef<{ list: Wreck[]; after: number }>({ list: [], after: -1 });
  /** The selected unit's step labels on the map. */
  const planLabelsRef = useRef<{ key: string; markers: maplibregl.Marker[] }>({ key: "", markers: [] });
  const runnerRef = useRef<RealtimeRunner | null>(null);
  const orderedRef = useRef<RtState | null>(null);
  const phaseRef = useRef(phase);
  phaseRef.current = phase;
  const brushRef = useRef({ side, platform, count, quality, asHq });
  brushRef.current = { side, platform, count, quality, asHq };
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
      timing: { ...DEFAULT_TIMING, breakpoint: breakpointPct / 100, maxDurationS: timeLimitMin * 60 },
      planner,
      isPassable,
      comms: commsMode,
    }),
    [terrain, gameSeed, planner, isPassable, breakpointPct, timeLimitMin, commsMode],
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
      if (phaseRef.current !== "setup") {
        setSelectedId(null);
        return;
      }
      if (!isOnBoard(at, boundsRef.current)) return;
      const brush = brushRef.current;
      const snapshot = PLATFORM_SNAPSHOT[brush.platform];
      if (!snapshot) return;
      setPlaced((current) => [
        ...current,
        {
          id: nextId(),
          label: `${brush.asHq ? "HQ " : ""}${snapshot.displayName} (${brush.count})`,
          side: brush.side,
          platform: brush.platform,
          platformCount: brush.count,
          troopQuality: brush.quality,
          position: at,
          ...(brush.asHq ? { commandRating: 3 } : {}),
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
  /** The current run's stream queue: rows waiting to go to Foundry. */
  const streamRef = useRef<EventStreamQueue | null>(null);
  const stopStream = useCallback(() => {
    void streamRef.current?.stop();
    streamRef.current = null;
  }, []);
  useEffect(() => stopStream, [stopStream]);

  const start = useCallback(() => {
    const initial = orderedRef.current;
    if (!initial) return;
    const config = makeConfig(runsRef.current);
    runsRef.current += 1;
    // Every run is its own run in the stream, "Run scenario again" included.
    stopStream();
    let onEntry: RunnerOptions["onEntry"];
    if (streamLog) {
      const queue = new EventStreamQueue(foundryStreamPublisher());
      queue.start();
      streamRef.current = queue;
      const run = { runId: `${gameSeed}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`, gameSeed };
      let sequence = 0;
      onEntry = (entry, state) => {
        if (entry.type === "message" && !STREAM_MESSAGES) return;
        queue.push(streamRow(entry, state, run, sequence++));
      };
    }
    const deciders = useJev
      ? {
          blue: jevRealtimeDecider({ side: "blue", call: jevCall, directive: blueDirective }),
          red: jevRealtimeDecider({ side: "red", call: jevCall, directive: redDirective }),
        }
      : undefined;
    const runner = new RealtimeRunner(initial, config, { deciders, onEntry });
    wrecksRef.current = { list: [], after: -1 };
    runnerRef.current = runner;
    setEndDismissed(false);
    setDraft([]);
    setPhase("running");
    setPlaying(true);
  }, [makeConfig, useJev, jevCall, blueDirective, redDirective, streamLog, gameSeed, stopStream]);

  const reset = useCallback(() => {
    stopStream();
    for (const m of planLabelsRef.current.markers) m.remove();
    planLabelsRef.current = { key: "", markers: [] };
    runsRef.current = 0;
    setPlaying(false);
    runnerRef.current = null;
    orderedRef.current = null;
    setDraft([]);
    setEndDismissed(false);
    setSelectedId(null);
    setPhase("setup");
  }, [stopStream]);

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
        if (!map.getSource(PICTURE_SOURCE)) {
          map.addSource(PICTURE_SOURCE, { type: "geojson", data: emptyCollection() });
          map.addLayer({
            id: `${PICTURE_SOURCE}-area`,
            type: "fill",
            source: PICTURE_SOURCE,
            filter: ["==", ["geometry-type"], "Polygon"],
            paint: { "fill-color": "#e8c547", "fill-opacity": 0.08 },
          });
          map.addLayer({
            id: `${PICTURE_SOURCE}-edge`,
            type: "line",
            source: PICTURE_SOURCE,
            filter: ["==", ["geometry-type"], "Polygon"],
            paint: { "line-color": "#e8c547", "line-width": 1, "line-dasharray": [2, 2], "line-opacity": 0.7 },
          });
          // A dot where it was reported (text needs map fonts the basemap may not have).
          map.addLayer({
            id: `${PICTURE_SOURCE}-dot`,
            type: "circle",
            source: PICTURE_SOURCE,
            filter: ["==", ["geometry-type"], "Point"],
            paint: { "circle-radius": 4, "circle-color": "#e8c547", "circle-stroke-color": "#0d1017", "circle-stroke-width": 1 },
          });
        }
        // The overlays (overlayLayers.ts): impacts, wrecks, spotting, bearings, radio, the selected unit's plan.
        for (const id of OVERLAY_SOURCES) if (!map.getSource(id)) map.addSource(id, { type: "geojson", data: emptyCollection() });
        for (const spec of overlayLayerSpecs(SIDE_COLOUR)) {
          if (!map.getLayer(spec.id as string)) map.addLayer(spec as unknown as maplibregl.LayerSpecification);
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
      // Once the game is built: show this unit in the inspector (again to close it).
      else setSelectedId((current) => (current === id ? null : id));
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
      // A vehicle just knocked out: the counter shakes, for a moment.
      const lastLoss = unit?.losses.length ? Math.max(...unit.losses) : -Infinity;
      const shaking = state.time - lastLoss <= 2 * Math.max(1, speedRef.current / 5);
      inner.style.animation = shaking ? "rt-shake 0.35s ease-in-out 3" : "";
      // The unit in the inspector is ringed.
      inner.style.outline = selectedRef.current === id ? "2px solid #e8c547" : "none";
      inner.style.outlineOffset = "3px";
      inner.style.borderRadius = "3px";
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

    // The selected unit's picture: each enemy it only knows by report, where
    // it was reported, ringed by how far off that may be by now.
    const chosen = selectedRef.current ? state.units[selectedRef.current] : undefined;
    const chosenFe = selectedRef.current ? state.game.forceElements[selectedRef.current] : undefined;
    const pictureFeatures =
      chosen && chosenFe && own(chosenFe.side)
        ? Object.entries(chosen.picture ?? {})
            .filter(([enemyId]) => chosen.ownSeen[enemyId] == null)
            .flatMap(([enemyId, track]) => {
              const radius = Math.max(50, trackErrorM(track, state.time));
              const ring = Array.from({ length: 33 }, (_, i) => {
                const p = offsetBy(track.at, (i * 360) / 32, radius);
                return [p.lng, p.lat];
              });
              return [
                { type: "Feature" as const, properties: {}, geometry: { type: "Polygon" as const, coordinates: [ring] } },
                {
                  type: "Feature" as const,
                  properties: { label: `${enemyId}? (${track.from}, ${clock(state.time - track.seenAt)} ago)` },
                  geometry: { type: "Point" as const, coordinates: [track.at.lng, track.at.lat] },
                },
              ];
            })
        : [];
    (map.getSource(PICTURE_SOURCE) as maplibregl.GeoJSONSource | undefined)?.setData({
      type: "FeatureCollection",
      features: pictureFeatures,
    });

    // ── What happens, where it happens (overlays.ts) ──────────────────────
    const runner = runnerRef.current;
    const log = runner?.log ?? [];
    const overlay = {
      own,
      // At high speed an effect lasts longer in simulated time, so the eye still catches it.
      life: (baseS: number) => baseS * Math.max(1, speedRef.current / 5),
    };
    const set = (source: string, data: unknown) =>
      (map.getSource(source) as maplibregl.GeoJSONSource | undefined)?.setData(data as GeoJSON.FeatureCollection);
    const fire = fireOverlay(state, log, overlay);
    set(SOURCE.fire, fire.lines);
    set(SOURCE.impact, fire.impacts);
    // Wrecks stay for the whole game, though the feed drops old lines.
    const fresh = newWrecks(state, log, wrecksRef.current.after);
    if (fresh.length) {
      wrecksRef.current.list.push(...fresh);
      wrecksRef.current.after = Math.max(...fresh.map((w) => w.time));
    }
    // Every wreck is seen by one side or the other: the one that lost it, or the one that knocked it out.
    set(SOURCE.wreck, wreckOverlay(state, wrecksRef.current.list));
    const spotting = spottingOverlay(state, log, overlay);
    set(SOURCE.spot, spotting.lines);
    set(SOURCE.pop, spotting.pops);
    set(SOURCE.wedge, spotting.wedges);
    const radioTraffic = radioOverlay(state, log, overlay);
    set(SOURCE.radio, radioTraffic.lines);
    set(SOURCE.pulse, radioTraffic.pulses);
    const plan = planOverlay(state, selectedRef.current, overlay);
    set(SOURCE.plan, plan.lines);
    set(SOURCE.planZone, plan.zones);
    // Step numbers as small labels (map text needs fonts the basemap may not have).
    const key = plan.steps.map((s) => `${s.label}@${s.at.lat.toFixed(5)},${s.at.lng.toFixed(5)}${s.current ? "*" : ""}`).join("|");
    if (key !== planLabelsRef.current.key) {
      for (const m of planLabelsRef.current.markers) m.remove();
      planLabelsRef.current = {
        key,
        markers: plan.steps.map((step) => {
          const node = document.createElement("div");
          node.textContent = step.label;
          node.style.cssText =
            "font:600 9px/1 var(--font-mono, monospace);padding:2px 4px;border-radius:8px;pointer-events:none;" +
            `background:${step.current ? "#e8c547" : "rgba(13,16,23,0.9)"};color:${step.current ? "#0d1017" : "#f2f4fa"};` +
            "border:1px solid rgba(255,255,255,0.6)";
          return new maplibregl.Marker({ element: node }).setLngLat([step.at.lng, step.at.lat]).addTo(map);
        }),
      };
    }
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

  // Orders generated, the viewpoint or the selected unit changed: redraw once.
  useEffect(() => {
    if (phase !== "setup") draw();
  }, [phase, viewpoint, draw, mapEpoch, selectedId]);

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
            <label style={{ ...row, gap: 6, color: asHq ? "#e8c547" : "#8a91a8", cursor: "pointer" }}>
              <input type="checkbox" checked={asHq} onChange={(e) => setAsHq(e.target.checked)} />
              HQ: passes on its side's radio traffic
            </label>
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

            <label style={{ ...row, gap: 6, marginTop: 8, color: streamLog ? "#e8c547" : "#8a91a8", cursor: "pointer" }}>
              <input type="checkbox" checked={streamLog} onChange={(e) => setStreamLog(e.target.checked)} />
              Stream the event log to Foundry
            </label>
            <div style={{ ...subtle, lineHeight: 1.5 }}>
              Every line of the feed, as it happens, to <code>{EVENT_STREAM_RID.slice(-12)}</code>: one run per Play or
              &ldquo;Run scenario again&rdquo;.
            </div>

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

            <div style={{ ...groupTitle, marginTop: 14 }}>4 &middot; Communications</div>
            <div style={row}>
              <span style={{ ...subtle, width: 46 }}>comms</span>
              <select value={commsMode} onChange={(e) => setCommsMode(e.target.value as "radio" | "perfect")} style={select}>
                <option value="radio">radio net</option>
                <option value="perfect">perfect (instant sharing)</option>
              </select>
            </div>
            <div style={{ ...subtle, lineHeight: 1.5 }}>
              {commsMode === "radio"
                ? "Each unit knows what it has seen and what reaches it by radio: reports take time, go through an HQ if " +
                  "there is one, queue when the net is busy, and do not carry beyond about 5 km. Units ask each other for cover and fire."
                : "Every sighting reaches the whole side after 15 s, whatever the range. Units still ask each other for help."}
            </div>

            <div style={{ ...groupTitle, marginTop: 14 }}>5 &middot; Win conditions</div>
            <div style={row}>
              <span style={{ ...subtle, width: 96 }}>breakpoint %</span>
              <input
                type="number"
                min={10}
                max={100}
                step={5}
                value={breakpointPct}
                onChange={(e) => setBreakpointPct(Math.max(10, Math.min(100, Number(e.target.value) || 50)))}
                style={select}
              />
            </div>
            <div style={row}>
              <span style={{ ...subtle, width: 96 }}>time limit min</span>
              <input
                type="number"
                min={5}
                max={600}
                step={5}
                value={timeLimitMin}
                onChange={(e) => setTimeLimitMin(Math.max(5, Math.min(600, Number(e.target.value) || 90)))}
                style={select}
              />
            </div>
            <div style={{ ...subtle, lineHeight: 1.5 }}>
              A side is beaten once {breakpointPct}% of its starting strength is destroyed or broken (a broken unit counts in
              full). If neither is by {timeLimitMin} min, the ground and what is left decide: holding your objective
              uncontested wins, then a broken enemy, then a clear lead in strength.
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
                      ? ` A side is beaten when ${breakpointPct}% of its strength is destroyed or broken; with one troop a side, one troop breaking can end it.`
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
                  {streamRef.current && (
                    <span style={{ color: streamRef.current.lastError ? "#e07a5f" : undefined }}>
                      {" "}&middot; streamed {streamRef.current.sent}
                      {streamRef.current.dropped ? `, ${streamRef.current.dropped} dropped` : ""}
                      {streamRef.current.lastError ? ` — stream failing: ${streamRef.current.lastError}` : ""}
                    </span>
                  )}
                </div>
                <button onClick={reset} style={{ ...chip, marginTop: 6 }}>
                  back to setup
                </button>
              </>
            )}

            {phase === "running" && <MapKey />}

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
                fighting strength (a side breaks below {100 - breakpointPct}%)
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
        <div style={{ ...rightPane, overflowY: "hidden", display: "flex", flexDirection: "column" }}>
          <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
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
                        : entry.type === "message"
                          ? "#8fbfb0"
                          : "#8a91a8",
              }}
            >
              {describeEntry(entry)}
            </div>
          ))}
          </div>
          <div style={{ flex: "0 0 auto", maxHeight: "48%", overflowY: "auto", borderTop: "1px solid #191e37", paddingTop: 8, marginTop: 6 }}>
            {selectedId && state ? (
              <UnitInspector
                state={state}
                id={selectedId}
                viewpoint={viewpoint}
                awaitingOrders={runner?.awaitingOrders.includes(selectedId) ?? false}
                onClose={() => setSelectedId(null)}
              />
            ) : (
              <div style={{ ...subtle, lineHeight: 1.5 }}>Click a unit on the map to see what it is doing, its orders and what it knows.</div>
            )}
          </div>
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
