/**
 * BgwsApp — the Battlegroup Wargame System board.
 *
 * The spike: a 10 x 10 km sheet on real terrain, the 1 km grid a player reads
 * grid references off, and Force Elements placed from the equipment profiles
 * in the [SIM] L6 datasets.
 *
 * What is deliberately NOT here yet: the rules. Every resolution table BGWS
 * uses — Fire Results, Terrain Effects, Assault Column Shifts, Morale,
 * Initiative, Sighting — lives on Player Aids 1-7, which the Core Rulebook
 * references constantly and does not reproduce. Building a sequence of play
 * around tables nobody has would mean inventing the numbers, and invented
 * numbers in a training tool are worse than an unfinished one. So this stops
 * at the board and the force pool, both of which are real.
 *
 * ⚠ The force pool needs two things to show data:
 *   1. the L6 datasets added as Resources on this app in Developer Console
 *      (the SQL scopes alone return 403);
 *   2. the L6 pipeline PR merged — the SQL API reads master.
 * Both failure modes are reported in the panel rather than logged, because
 * "empty list" and "you have not been granted the dataset" look identical
 * otherwise.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import maplibregl from "maplibre-gl";
import ms from "milsymbol";

import { DechoBasemap } from "@acc/decho-basemap/react";

import { AppSwitcher } from "@/components/AppSwitcher";
import { initMgrsOverlay, lngLatToMgrs } from "@/shared/components/MgrsOverlay";
import { planetStore } from "@/shared/map/dechoBasemapSetup";
import { SqlError } from "@/shared/lib/sqlClient";

import {
  BOARD_SIZE_M,
  boardBounds,
  boardRing,
  isOnBoard,
  type BoardBounds,
  type LatLng,
} from "./lib/board";
import { counterLabel, counterSidc, moveTypeLabel, type Side } from "./lib/counterSymbol";
import { projectForSide } from "./lib/fogOfWar";
import { proceduralTerrain, STANDARD_GROUND } from "./lib/proceduralTerrain";
import {
  sightLinesFrom,
  sightingFromLineOfSight,
  toGameState,
  type PlacedUnit,
  type SightLine,
} from "./lib/spikeGame";
import type { PlatformProfile } from "./data/profiles";
import { loadPlatformProfiles } from "./data/profilesClient";

// ── The sheet ──────────────────────────────────────────────────────────────
//
// Kaliningrad by default, matching DEFAULT_TERRAIN in shared/routing — the
// terrain raster and pathfinding graph exist there, which is what movement
// will need next. Overridable by query string so a different AO can be tried
// without a rebuild.

function centreFromUrl(): LatLng {
  const params = new URLSearchParams(window.location.search);
  const num = (key: string, fallback: number) => {
    const raw = params.get(key);
    if (raw === null) return fallback;
    const parsed = Number(raw);
    return Number.isFinite(parsed) ? parsed : fallback;
  };
  return { lat: num("lat", 54.71), lng: num("lon", 20.51) };
}

const BOARD_SOURCE = "bgws-board";
const BOARD_OUTLINE_LAYER = "bgws-board-outline";
const BOARD_HAZE_LAYER = "bgws-board-haze";
const SIGHT_SOURCE = "bgws-sight-lines";
const SIGHT_LAYER = "bgws-sight-lines-layer";

/**
 * Whose eyes the board is seen through.
 *
 * Not a display filter: picking Blue runs the same fog-of-war projection the
 * AI opponent will be given, so what is on screen is exactly what that side
 * knows. Umpire sees the ground truth.
 */
type Viewpoint = Side | "umpire";

/**
 * The terrain the rules are resolved against.
 *
 * Generated ground, not flat ground, and the difference is a third of the
 * rulebook: on flat featureless terrain nothing is ever in cover, no ridge
 * ever breaks a sight line, and `targetInCover`, `defenderInCover` and every
 * elevation effect are unreachable by construction.
 *
 * The real raster is wired into the PLAY screen (see lib/rasterTerrain.ts and
 * Play.tsx), where there is a client to load it with and a control to say
 * which ground a game was fought over. This screen is the inspector: it wants
 * ground that needs no dataset and is the same every time, which is exactly
 * what the standard generated ground is for.
 */
const TERRAIN = proceduralTerrain(STANDARD_GROUND);

function newId(): string {
  // Math.random() is blocked by the code scan, and rightly: every identifier
  // and every die roll in this app has to come from somewhere accountable.
  const bytes = new Uint8Array(8);
  window.crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

export default function BgwsApp() {
  const centre = useMemo(centreFromUrl, []);
  const bounds = useMemo<BoardBounds>(() => boardBounds(centre), [centre]);

  const mapRef = useRef<maplibregl.Map | null>(null);
  const markersRef = useRef<Map<string, maplibregl.Marker>>(new Map());

  const [platforms, setPlatforms] = useState<PlatformProfile[]>([]);
  const [poolError, setPoolError] = useState<string | null>(null);
  const [loadingPool, setLoadingPool] = useState(true);
  const [search, setSearch] = useState("");
  const [side, setSide] = useState<Side>("blue");
  const [selected, setSelected] = useState<PlatformProfile | null>(null);
  const [units, setUnits] = useState<PlacedUnit[]>([]);
  const [cursor, setCursor] = useState<string>("");
  const [note, setNote] = useState<string | null>(null);
  const [viewpoint, setViewpoint] = useState<Viewpoint>("umpire");
  const [observerId, setObserverId] = useState<string | null>(null);

  // The rules' view of the board. Rebuilt from the placed units on every
  // change — cheap at this scale, and it keeps one source of truth rather than
  // a game state that can drift from what is on screen.
  const game = useMemo(() => {
    const base = toGameState(units, (unit) => counterSidc(unit.platform, unit.side));
    return sightingFromLineOfSight(base, TERRAIN);
  }, [units]);

  /** Which unit ids this viewpoint may see, and how well. */
  const visibility = useMemo(() => {
    if (viewpoint === "umpire") {
      return { ids: new Set(units.map((u) => u.id)), identified: new Set(units.map((u) => u.id)) };
    }
    const view = projectForSide(game, viewpoint);
    const ids = new Set<string>([
      ...view.own.map((fe) => fe.id),
      ...view.contacts.map((c) => c.id),
    ]);
    const identified = new Set<string>([
      ...view.own.map((fe) => fe.id),
      ...view.contacts.filter((c) => c.sighting === "full").map((c) => c.id),
    ]);
    return { ids, identified };
  }, [game, units, viewpoint]);

  const sightLines = useMemo<SightLine[]>(
    () => (observerId ? sightLinesFrom(game, observerId, TERRAIN) : []),
    [game, observerId],
  );

  // Kept in a ref so the map's click handler — registered once — always sees
  // the current selection without being torn down and rebuilt on every click.
  const selectedRef = useRef<PlatformProfile | null>(null);
  const sideRef = useRef<Side>("blue");
  useEffect(() => {
    selectedRef.current = selected;
  }, [selected]);
  useEffect(() => {
    sideRef.current = side;
  }, [side]);

  // ── Force pool ───────────────────────────────────────────────────────────

  useEffect(() => {
    const controller = new AbortController();
    setLoadingPool(true);
    setPoolError(null);

    loadPlatformProfiles({ search: search.trim() || undefined, limit: 120 }, controller.signal)
      .then((rows) => {
        setPlatforms(rows);
        setLoadingPool(false);
      })
      .catch((err: unknown) => {
        if (controller.signal.aborted) return;
        setLoadingPool(false);
        if (err instanceof SqlError && err.permissionDenied) {
          setPoolError(
            "No access to the equipment datasets. Add [SIM] L6 bgws_platform_profile " +
              "and bgws_capability_profile as Resources on this app in Developer Console.",
          );
        } else {
          setPoolError(err instanceof Error ? err.message : String(err));
        }
      });

    return () => controller.abort();
  }, [search]);

  // ── Map ──────────────────────────────────────────────────────────────────

  const onMapReady = useCallback(
    (map: maplibregl.Map) => {
      mapRef.current = map;
      initMgrsOverlay(map);

      const draw = () => {
        if (map.getSource(BOARD_SOURCE)) return;
        map.addSource(BOARD_SOURCE, {
          type: "geojson",
          data: {
            type: "Feature",
            properties: {},
            geometry: { type: "Polygon", coordinates: [boardRing(bounds)] },
          },
        });
        // Everything outside the sheet is dimmed rather than hidden: a player
        // needs to see what the ground beyond the boundary looks like without
        // being able to mistake it for playable.
        map.addLayer({
          id: BOARD_HAZE_LAYER,
          type: "fill",
          source: BOARD_SOURCE,
          paint: { "fill-color": "#0b1220", "fill-opacity": 0 },
        });
        map.addLayer({
          id: BOARD_OUTLINE_LAYER,
          type: "line",
          source: BOARD_SOURCE,
          paint: { "line-color": "#e8c547", "line-width": 2, "line-dasharray": [3, 2] },
        });

        map.addSource(SIGHT_SOURCE, {
          type: "geojson",
          data: { type: "FeatureCollection", features: [] },
        });
        map.addLayer({
          id: SIGHT_LAYER,
          type: "line",
          source: SIGHT_SOURCE,
          paint: {
            "line-color": ["case", ["get", "visible"], "#5cd992", "#d95c5c"],
            "line-width": 1.5,
            "line-opacity": 0.8,
            "line-dasharray": ["case", ["get", "visible"], ["literal", [1, 0]], ["literal", [2, 2]]],
          },
        });
        map.fitBounds(
          [
            [bounds.west, bounds.south],
            [bounds.east, bounds.north],
          ],
          { padding: 40, duration: 0 },
        );
      };

      if (map.isStyleLoaded()) draw();
      else map.on("load", draw);

      map.on("mousemove", (e) => {
        setCursor(lngLatToMgrs(e.lngLat.lng, e.lngLat.lat, 4));
      });

      map.on("click", (e) => {
        const platform = selectedRef.current;
        if (!platform) return;
        const position = { lat: e.lngLat.lat, lng: e.lngLat.lng };
        if (!isOnBoard(position, bounds)) {
          setNote("Off the sheet — a Force Element has to start on the board.");
          return;
        }
        setNote(null);
        setUnits((prev) => [
          ...prev,
          { id: newId(), side: sideRef.current, platform, position },
        ]);
      });
    },
    [bounds],
  );

  // Counters follow the VIEWPOINT, not the unit list: what is on screen is
  // what the chosen side knows. Rebuilt wholesale on every change — at a
  // battlegroup's worth of counters that costs nothing, and it removes a class
  // of bug where a marker survives a change in what its owner can see.
  //
  // milsymbol's asDOM() rather than asSVG(): it returns a real element, so no
  // HTML string is ever assigned into the document — which the code scan
  // blocks, and which would otherwise need sanitising.
  useEffect(() => {
    const map = mapRef.current;
    if (!map) return;

    const live = markersRef.current;
    for (const marker of live.values()) marker.remove();
    live.clear();

    for (const unit of units) {
      if (!visibility.ids.has(unit.id)) continue;

      const element = document.createElement("div");
      element.style.cursor = "pointer";

      if (visibility.identified.has(unit.id)) {
        const symbol = new ms.Symbol(counterSidc(unit.platform, unit.side), {
          size: 28,
          uniqueDesignation: counterLabel(unit.platform),
        });
        element.title = `${unit.platform.displayName} · ${moveTypeLabel(unit.platform.moveType)}`;
        element.appendChild(symbol.asDOM());
      } else {
        // A contact that has been seen but not identified. A position and
        // nothing else — drawing the counter greyed out would still say what
        // it is, which is the thing a partial sighting does not tell you.
        const dot = document.createElement("div");
        dot.style.width = "14px";
        dot.style.height = "14px";
        dot.style.borderRadius = "50%";
        dot.style.border = "2px dashed #e9ecfb";
        dot.style.background = "rgba(212,217,232,0.15)";
        element.title = "Unidentified contact";
        element.appendChild(dot);
      }

      element.addEventListener("click", (event) => {
        // Without this the click also lands on the map and places a unit.
        event.stopPropagation();
        setObserverId((current) => (current === unit.id ? null : unit.id));
      });

      const marker = new maplibregl.Marker({ element })
        .setLngLat([unit.position.lng, unit.position.lat])
        .addTo(map);
      live.set(unit.id, marker);
    }
  }, [units, visibility]);

  // Sight lines from the selected counter: green where the line is clear, red
  // where something blocks it.
  useEffect(() => {
    const map = mapRef.current;
    if (!map?.getSource(SIGHT_SOURCE)) return;
    const observer = observerId ? game.forceElements[observerId] : undefined;
    const source = map.getSource(SIGHT_SOURCE) as maplibregl.GeoJSONSource;

    source.setData({
      type: "FeatureCollection",
      features:
        observer == null
          ? []
          : sightLines.map((line) => ({
              type: "Feature" as const,
              properties: { visible: line.visible },
              geometry: {
                type: "LineString" as const,
                coordinates: [
                  [observer.position.lng, observer.position.lat],
                  [line.to.lng, line.to.lat],
                ],
              },
            })),
    });
  }, [game, observerId, sightLines]);

  useEffect(() => {
    const live = markersRef.current;
    return () => {
      for (const marker of live.values()) marker.remove();
      live.clear();
    };
  }, []);

  // ── Render ───────────────────────────────────────────────────────────────

  return (
    <DechoBasemap
      tiles={planetStore}
      spawnLat={centre.lat}
      spawnLong={centre.lng}
      spawnZoom={12}
      onMapReady={onMapReady}
      style={{ height: "100vh" }}
    >
      <div style={header}>
        <span style={{ fontWeight: 700, letterSpacing: "0.12em" }}>BGWS</span>
        <span style={subtle}>
          {(BOARD_SIZE_M / 1000).toFixed(0)} × {(BOARD_SIZE_M / 1000).toFixed(0)} km sheet ·
          1 km grid · {units.length} FE placed
        </span>

        <span style={{ ...subtle, marginLeft: 8 }}>view as</span>
        {(["umpire", "blue", "red"] as const).map((point) => (
          <button
            key={point}
            onClick={() => setViewpoint(point)}
            style={{
              ...headerButton,
              borderColor:
                viewpoint === point
                  ? point === "umpire"
                    ? "#e8c547"
                    : sideColour[point]
                  : "rgba(255,255,255,0.15)",
              color:
                viewpoint === point
                  ? point === "umpire"
                    ? "#e8c547"
                    : sideColour[point]
                  : "rgba(255,255,255,0.45)",
            }}
          >
            {point.toUpperCase()}
          </button>
        ))}

        <span style={{ flex: 1 }} />
        <span style={subtle}>{cursor}</span>
        <AppSwitcher />
      </div>

      <div style={panel}>
        <div style={panelTitle}>Force pool</div>

        <div style={{ display: "flex", gap: 6, marginBottom: 8 }}>
          {(["blue", "red"] as const).map((s) => (
            <button
              key={s}
              onClick={() => setSide(s)}
              style={{
                ...sideButton,
                borderColor: side === s ? sideColour[s] : "rgba(255,255,255,0.15)",
                color: side === s ? sideColour[s] : "rgba(255,255,255,0.5)",
              }}
            >
              {s === "blue" ? "BLUE" : "RED"}
            </button>
          ))}
        </div>

        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search equipment…"
          style={input}
        />

        {loadingPool && <div style={subtle}>Loading…</div>}

        {poolError && (
          <div style={{ ...subtle, color: "#e07a5f", lineHeight: 1.5 }}>{poolError}</div>
        )}

        {!loadingPool && !poolError && platforms.length === 0 && (
          <div style={subtle}>
            No platforms. The L6 datasets may not be built on master yet.
          </div>
        )}

        <div style={{ overflowY: "auto", flex: 1, marginTop: 6 }}>
          {platforms.map((platform) => {
            const isSelected = selected?.assetId === platform.assetId;
            return (
              <button
                key={platform.assetId}
                onClick={() => setSelected(isSelected ? null : platform)}
                style={{
                  ...row,
                  background: isSelected ? "rgba(232,197,71,0.12)" : "transparent",
                  borderColor: isSelected ? "#e8c547" : "transparent",
                }}
              >
                <div style={{ fontSize: 12, color: "#e9ecfb" }}>
                  {counterLabel(platform)}
                </div>
                <div style={subtle}>
                  {moveTypeLabel(platform.moveType)}
                  {platform.massCombatT != null && ` · ${platform.massCombatT.toFixed(1)} t`}
                  {platform.hpPerTonne != null && ` · ${platform.hpPerTonne} hp/t`}
                  {platform.capabilities.length > 0 &&
                    ` · ${platform.capabilities.join(" ")}`}
                </div>
              </button>
            );
          })}
        </div>

        <div style={{ ...subtle, borderTop: "1px solid #191e37", paddingTop: 8, marginTop: 8 }}>
          {selected
            ? `Click the sheet to place ${counterLabel(selected)}.`
            : "Select equipment, then click the sheet. Click a counter for its sight lines."}
          {note && <div style={{ color: "#e07a5f", marginTop: 4 }}>{note}</div>}

          {viewpoint !== "umpire" && (
            <div style={{ marginTop: 6 }}>
              Seeing as {viewpoint.toUpperCase()}: {visibility.ids.size} of {units.length}{" "}
              counters. Unsighted force elements are not drawn — the same
              projection an AI opponent is given.
            </div>
          )}

          {observerId && game.forceElements[observerId] && (
            <div style={{ marginTop: 6 }}>
              <div style={{ color: "#e9ecfb" }}>
                Sight lines from {counterLabel(
                  units.find((u) => u.id === observerId)!.platform,
                )}
              </div>
              {sightLines.length === 0 && <div>No opposing force elements on the sheet.</div>}
              {sightLines.map((line) => (
                <div key={line.targetId} style={{ color: line.visible ? "#5cd992" : "#d95c5c" }}>
                  {line.visible ? "clear" : line.reason} · {line.rangeM} m
                </div>
              ))}
              <div style={{ marginTop: 4 }}>
                Geometry only — the Sighting test needs Player Aid 6. Terrain is
                flat until the DEM is granted, so nothing blocks yet but woods
                and counters.
              </div>
            </div>
          )}
        </div>
      </div>
    </DechoBasemap>
  );
}

// ── Styles ─────────────────────────────────────────────────────────────────

const sideColour: Record<Side, string> = { blue: "#4A90D9", red: "#d95c5c" };

const header: React.CSSProperties = {
  position: "absolute",
  top: 0,
  left: 0,
  right: 0,
  height: 40,
  display: "flex",
  alignItems: "center",
  gap: 12,
  padding: "0 12px",
  background: "rgba(9,12,20,0.92)",
  borderBottom: "1px solid #191e37",
  color: "#e9ecfb",
  font: "11px/1 var(--font-mono, monospace)",
  zIndex: 2,
};

const panel: React.CSSProperties = {
  position: "absolute",
  top: 52,
  left: 12,
  bottom: 12,
  width: 290,
  display: "flex",
  flexDirection: "column",
  padding: 10,
  background: "rgba(9,12,20,0.92)",
  border: "1px solid #191e37",
  borderRadius: 6,
  color: "#e9ecfb",
  font: "11px/1.4 var(--font-mono, monospace)",
  zIndex: 2,
};

const panelTitle: React.CSSProperties = {
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.12em",
  textTransform: "uppercase",
  color: "rgba(255,255,255,0.45)",
  marginBottom: 8,
};

const subtle: React.CSSProperties = {
  fontSize: 10,
  color: "#6a7292",
};

const input: React.CSSProperties = {
  width: "100%",
  boxSizing: "border-box",
  padding: "6px 8px",
  marginBottom: 6,
  background: "rgba(255,255,255,0.04)",
  border: "1px solid rgba(255,255,255,0.12)",
  borderRadius: 3,
  color: "#e9ecfb",
  font: "inherit",
};

const row: React.CSSProperties = {
  display: "block",
  width: "100%",
  textAlign: "left",
  padding: "6px 8px",
  marginBottom: 2,
  border: "1px solid transparent",
  borderRadius: 3,
  cursor: "pointer",
  font: "inherit",
};

const headerButton: React.CSSProperties = {
  padding: "3px 8px",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid",
  borderRadius: 3,
  cursor: "pointer",
  font: "inherit",
  fontSize: 10,
  fontWeight: 700,
  letterSpacing: "0.08em",
};

const sideButton: React.CSSProperties = {
  flex: 1,
  padding: "5px 0",
  background: "rgba(255,255,255,0.04)",
  border: "1px solid",
  borderRadius: 3,
  cursor: "pointer",
  font: "inherit",
  fontWeight: 700,
  letterSpacing: "0.1em",
};
