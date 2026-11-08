// The BGWS viewer on bgws-server: the map in the middle, the game and its
// clock on the left, what is happening and the selected unit on the right.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { buildLayers, DEFAULT_LAYERS, tooltip, type LayerKey } from "./layers";
import { MapView, type Basemap } from "./MapView";
import { SIDE_COLOUR } from "./overlays";
import { clock, type Side } from "./protocol";
import { ClockPanel, EndCard, Feed, ForcesPanel, GamePanel, HqConsole, Inspector, MapKey, NewGameCard, Questions, ReportCard, Spectrum, ViewPanel } from "./Panels";
import { useServer } from "./useServer";

function readPref<T extends string>(key: string, fallback: T): T {
  try {
    return (localStorage.getItem(`bgws.${key}`) as T) ?? fallback;
  } catch {
    return fallback;
  }
}
function writePref(key: string, value: string) {
  try {
    localStorage.setItem(`bgws.${key}`, value);
  } catch {
    /* private window: not remembered */
  }
}

export function App() {
  const server = useServer();
  const { state: s, send } = server;
  const [selected, setSelected] = useState<string | null>(null);
  const [layers, setLayers] = useState<Set<LayerKey>>(() => {
    const saved = readPref<string>("layers", "");
    return new Set((saved ? saved.split(",") : DEFAULT_LAYERS) as LayerKey[]);
  });
  const [basemap, setBasemap] = useState<Basemap>(() => readPref<Basemap>("basemap", "terrain"));
  const [hqOpen, setHqOpen] = useState(false);
  const [objective, setObjective] = useState<{ lat: number; lon: number } | null>(null);
  const [picking, setPicking] = useState(false);
  const [newGame, setNewGame] = useState(false);
  const [endClosed, setEndClosed] = useState(false);
  const [focus, setFocus] = useState<{ lon: number; lat: number; key: number } | null>(null);
  const pickedAt = useRef(0);
  const [view, setView] = useState<[number, number, number, number] | null>(null);
  const hasPackMap = !!s.game?.pack?.basemap;
  // A game on a theatre pack shows the pack's own map. "terrain" works
  // without one too: the world overview, served (and cached) by bgws-server.
  useEffect(() => {
    if (hasPackMap) setBasemap("terrain");
  }, [hasPackMap, s.game?.name]);

  useEffect(() => writePref("layers", [...layers].join(",")), [layers]);
  useEffect(() => writePref("basemap", basemap), [basemap]);
  // 3D terrain, when the game is on a pack with elevation and the terrain basemap is up.
  const [threeD, setThreeD] = useState(() => readPref<string>("3d", "off") === "on");
  useEffect(() => writePref("3d", threeD ? "on" : "off"), [threeD]);
  const can3D = basemap === "terrain" && !!s.game?.pack?.dem;
  const no3D =
    basemap !== "terrain"
      ? "3D uses the terrain basemap: pick terrain first"
      : !s.game?.pack
        ? "3D needs a theatre pack (New game… → Ground): generated ground has no real elevation to show"
        : "This pack has no elevation map (built before packs carried one): build it again with bgws terrain";
  const [liftTick, setLiftTick] = useState(0);
  const onLift = useCallback(() => setLiftTick((t) => t + 1), []);

  // The selected unit's details, now and every second and a half while selected.
  useEffect(() => {
    if (!selected) return;
    send({ kind: "inspect", unit: selected });
    const timer = window.setInterval(() => send({ kind: "inspect", unit: selected }), 1500);
    return () => window.clearInterval(timer);
  }, [selected, send, s.viewer]);

  // What the selected unit can see: shown as soon as it is picked, and kept
  // up to date as it moves (worked out again once it has gone 100 m).
  const [sightOn, setSightOn] = useState(() => readPref<string>("sight", "on") === "on");
  useEffect(() => writePref("sight", sightOn ? "on" : "off"), [sightOn]);
  const sightAsked = useRef<{ unit: string; lon: number; lat: number; at: number } | null>(null);
  const viewerSide = s.viewer === "umpire" ? null : s.viewer;
  const selectedEntity = selected ? s.frame?.entities.find((x) => x.name === selected) : undefined;
  const mayLook = !!selectedEntity && (viewerSide === null || selectedEntity.side === viewerSide);
  useEffect(() => {
    if (!sightOn || !selectedEntity || !mayLook) {
      if (s.viewshed) server.clearViewshed();
      sightAsked.current = null;
      return;
    }
    const last = sightAsked.current;
    const now = performance.now();
    const moved = last ? Math.hypot((selectedEntity.lon - last.lon) * 111_320 * Math.cos((last.lat * Math.PI) / 180), (selectedEntity.lat - last.lat) * 111_320) : Infinity;
    if (!last || last.unit !== selectedEntity.name || (moved > 100 && now - last.at > 1000)) {
      sightAsked.current = { unit: selectedEntity.name, lon: selectedEntity.lon, lat: selectedEntity.lat, at: now };
      send({ kind: "viewshed", unit: selectedEntity.name });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sightOn, selected, mayLook, s.version]);

  // A new game or a branch: forget what belonged to the old one.
  const gameName = s.game?.name;
  useEffect(() => {
    setSelected(null);
    setObjective(null);
    setEndClosed(false);
  }, [gameName]);
  useEffect(() => {
    if (!s.frame?.over) setEndClosed(false);
  }, [s.frame?.over]);

  const select = useCallback(
    (name: string) => {
      pickedAt.current = performance.now();
      setSelected(name);
      const e = s.frame?.entities.find((x) => x.name === name);
      if (e) setFocus({ lon: e.lon, lat: e.lat, key: performance.now() });
    },
    [s.frame],
  );
  const pickOnMap = useCallback((name: string) => {
    pickedAt.current = performance.now();
    setSelected(name);
  }, []);

  const onMapClick = useCallback(
    (lon: number, lat: number) => {
      if (picking) {
        setObjective({ lon, lat });
        setPicking(false);
        return;
      }
      // deck.gl's click on a unit comes with the map's own: only an empty click deselects.
      window.setTimeout(() => {
        if (performance.now() - pickedAt.current > 120) setSelected(null);
      }, 30);
    },
    [picking],
  );

  const deckLayers = useMemo(
    () =>
      s.frame
        ? buildLayers({ frame: s.frame, events: s.events, game: s.game, on: layers, selected, unit: s.unit, viewshed: s.viewshed, objective, towns: s.game?.pack?.towns ?? [], onPick: pickOnMap })
        : [],
    // The frame object is mutated in place; `version` says when.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [s.version, s.frame, s.events, s.game, layers, selected, s.unit, s.viewshed, objective, pickOnMap, liftTick],
  );

  const fit = useMemo<[[number, number], [number, number]] | null>(() => {
    const es = s.frame?.entities.filter((e) => e.domain !== "drone") ?? [];
    if (!es.length) return null;
    const lons = es.map((e) => e.lon);
    const lats = es.map((e) => e.lat);
    return [
      [Math.min(...lons) - 0.01, Math.min(...lats) - 0.01],
      [Math.max(...lons) + 0.01, Math.max(...lats) + 0.01],
    ];
    // Only the first frame of each game decides where to look.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gameName, !!s.frame]);

  const strength = (side: Side) => {
    const units = (s.frame?.entities ?? []).filter((e) => e.side === side && e.domain !== "drone");
    const total = units.reduce((n, e) => n + e.total, 0);
    if (!total) return null;
    const fit = units.reduce((n, e) => n + (e.status === "destroyed" || e.status === "broken" ? 0 : e.fit), 0);
    return Math.round((fit / total) * 100);
  };
  const breakAt = s.game?.victory.find((v) => v.kind === "breakpoint")?.pct ?? null;
  const over = !!s.frame?.over;
  const missingKeys = s.game ? (["blue", "red"] as const).some((x) => s.game!.deciders[x].keyMissing) : false;

  return (
    <>
      <MapView layers={deckLayers} basemap={basemap} pack={s.game?.pack ?? null} threeD={threeD && can3D} onLift={onLift} onView={setView} fit={fit} fitKey={`${gameName}`} focus={focus} onMapClick={onMapClick} tooltip={tooltip} />

      <div className="header">
        <span className="brand">BGWS</span>
        <span className="mode">RUST ENGINE</span>
        <span className="subtle" style={{ color: "#c7ccdb" }}>{s.game?.name ?? "…"}</span>
        <span className="subtle">
          {clock(s.frame?.t ?? 0)}
          {(["blue", "red"] as const).map((x) => {
            const v = strength(x);
            return v == null ? null : (
              <span key={x} style={{ color: SIDE_COLOUR[x] }}>
                {" "}
                · {x} {v}%
              </span>
            );
          })}
        </span>
        {s.questions.length > 0 && <span className="pill warn">waiting for you</span>}
        {over && (
          <span className="pill warn">
            {s.frame!.over!.winner ?? "drawn"} — {s.frame!.over!.reason}
          </span>
        )}
        {missingKeys && <span className="pill warn" title="See Game, on the left">API key missing: rules decide</span>}
        <span className="spacer" />
        <span className={`subtle${s.connected ? "" : " bad"}`} style={{ color: s.connected ? undefined : "#e07a5f", maxWidth: 420, overflow: "hidden", textOverflow: "ellipsis" }} title={s.status}>
          {s.status}
        </span>
      </div>

      <div className="pane left">
        <GamePanel
          game={s.game}
          onNew={() => {
            send({ kind: "scenarios" });
            setNewGame(true);
          }}
          onSave={() => send({ kind: "save" })}
          onReview={() => send({ kind: "review" })}
        />
        <ClockPanel
          frame={s.frame}
          live={s.live}
          scrub={s.scrub}
          playing={s.playing}
          speed={s.speed}
          waiting={s.questions.length}
          over={over}
          limitS={s.game?.limitS ?? 0}
          onPlay={server.setPlaying}
          onSpeed={server.setSpeed}
          onSeek={(t) => send({ kind: "seek", t })}
          onLive={() => send({ kind: "live" })}
          onBranch={(t) => send({ kind: "branch", t })}
        />
        <ViewPanel
          viewer={s.viewer}
          onViewer={server.setViewer}
          layers={layers}
          onLayer={(k) =>
            setLayers((l) => {
              const n = new Set(l);
              if (n.has(k)) n.delete(k);
              else n.add(k);
              return n;
            })
          }
          basemap={basemap}
          onBasemap={setBasemap}
          hasPackMap={hasPackMap}
          threeD={threeD}
          can3D={can3D}
          no3D={no3D}
          onThreeD={setThreeD}
          hqOpen={hqOpen}
          onHq={() => setHqOpen((o) => !o)}
        />
        <ForcesPanel frame={s.frame} selected={selected} onSelect={select} breakAt={breakAt} />
        <MapKey />
      </div>

      <div className="pane right">
        <div className="scroll">
          <Questions questions={s.questions} onAnswer={server.answer} />
          <Feed events={s.events} onSelect={select} />
        </div>
        <div className="section">
          <Inspector
            unit={s.unit}
            selected={selected}
            viewer={s.viewer}
            frame={s.frame}
            viewshedOn={sightOn}
            onClose={() => setSelected(null)}
            onWhy={(unit) => send({ kind: "why", unit })}
            onMessages={(unit) => send({ kind: "messages", unit })}
            onViewshed={(unit) => setSightOn(!!unit)}
            onDetect={(observer, target) => send({ kind: "detect", observer, target })}
          />
        </div>
      </div>

      {hqOpen && (
        <HqConsole
          viewer={s.viewer}
          frame={s.frame}
          objective={objective}
          picking={picking}
          onPick={() => setPicking((p) => !p)}
          onSeat={(side, seat) => send({ kind: "seat", side, seat })}
          onIssue={(side, plan) => send({ kind: "command", side, plan })}
          onViewer={server.setViewer}
          onClose={() => {
            setHqOpen(false);
            setPicking(false);
          }}
        />
      )}
      {layers.has("spectrum") && s.frame && <Spectrum frame={s.frame} />}
      {picking && <div className="toast">Click the map to set the objective</div>}
      {s.report && <ReportCard title={s.report.title} text={s.report.text} onClose={server.closeReport} />}
      {newGame && (
        <NewGameCard
          scenarios={s.scenarios}
          packs={s.packs}
          terrain={s.terrain}
          view={view}
          current={s.game?.name}
          onOpen={(name, pack) => {
            send({ kind: "open", name, ...(pack ? { pack } : {}) });
            setNewGame(false);
          }}
          onBuild={(name, bbox, cellM) => send({ kind: "terrain", name, bbox, cell_m: cellM })}
          onClose={() => setNewGame(false)}
        />
      )}
      {over && !endClosed && !s.report && !newGame && s.frame && (
        <EndCard
          frame={s.frame}
          events={s.events}
          onReview={() => send({ kind: "review" })}
          onSave={() => send({ kind: "save" })}
          onReplay={() => send({ kind: "branch", t: 0 })}
          onNew={() => {
            send({ kind: "scenarios" });
            setNewGame(true);
          }}
          onClose={() => setEndClosed(true)}
        />
      )}
    </>
  );
}
