// The panes and overlays around the map, after the TS real-time screen:
// the game and clock on the left, what is happening and the selected unit
// on the right, and cards for questions, reports and the end of the game.

import { useMemo, useState, type ReactNode } from "react";
import { category, CATEGORY_COLOUR, CATEGORY_LABEL, SIDE_COLOUR, type Category } from "./overlays";
import { clock, span, type FrameEntity, type FrameView, type GameEvent, type GameInfo, type PackInfo, type Question, type ScenarioBrief, type Side, type TerrainProgress, type UnitDetail, type Viewer } from "./protocol";
import { LAYERS, type LayerKey } from "./layers";
import type { Basemap } from "./MapView";

export const SPEEDS = [1, 5, 10, 30, 60, 300, 600];

export function Line({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="line">
      <span>{label}</span>
      <span>{children}</span>
    </div>
  );
}

const cap = (s: string) => (s ? s[0].toUpperCase() + s.slice(1) : s);
const pct = (x: number) => `${Math.round(x * 100)}%`;
const DECIDER_WORDS: Record<string, string> = { rules: "rules", jev: "Jev (OpenRouter)", claude: "Claude (Anthropic API)", local: "local model", person: "a person" };
const KEY_NAME: Record<string, string> = { claude: "ANTHROPIC_API_KEY", jev: "OPENROUTER_API_KEY" };

// ── Left pane ──────────────────────────────────────────────────────────────

export function GamePanel(props: { game: GameInfo | null; onNew: () => void; onSave: () => void; onReview: () => void }) {
  const g = props.game;
  return (
    <>
      <div className="group">Game</div>
      {g ? (
        <>
          <div style={{ fontWeight: 700, marginBottom: 2 }}>{g.name}</div>
          <div className="subtle" style={{ marginBottom: 6 }}>
            {g.scope?.type} · {g.scope?.durationMin} min · blue {g.forces.blue} · red {g.forces.red}
            {g.flat ? " · flat ground" : ""}
          </div>
          {g.description && <div className="subtle" style={{ lineHeight: 1.5, marginBottom: 6 }}>{g.description}</div>}
          <Line label="ground">
            {g.pack ? (
              <>
                pack <b>{g.pack.name}</b>: {g.pack.sizeKm[0].toFixed(0)} × {g.pack.sizeKm[1].toFixed(0)} km
                {g.pack.roadsKm ? `, ${Math.round(g.pack.roadsKm).toLocaleString()} km of road` : ""}
                {g.pack.preview && (
                  <>
                    {" "}
                    <a className="link" href={g.pack.preview} target="_blank" rel="noopener">
                      preview ↗
                    </a>
                  </>
                )}
              </>
            ) : g.flat ? (
              "flat"
            ) : (
              "generated (no theatre pack)"
            )}
          </Line>
          {(["blue", "red"] as const).map((s) => {
            const d = g.deciders[s];
            const missing = d.keyMissing ? [d.units, d.hqs].filter((x) => KEY_NAME[x]).map((x) => KEY_NAME[x]) : [];
            return (
              <div key={s} style={{ marginBottom: 4 }}>
                <Line label={`${s} decides`}>
                  <span style={{ color: SIDE_COLOUR[s] }}>
                    {d.person ? "you (questions come here)" : `units: ${DECIDER_WORDS[d.units] ?? d.units} · HQ: ${DECIDER_WORDS[d.hqs] ?? d.hqs}`}
                  </span>
                </Line>
                {missing.length > 0 && <div className="callout warn">No {[...new Set(missing)].join(" or ")} set where the server runs: those decisions fall back to the rules.</div>}
              </div>
            );
          })}
        </>
      ) : (
        <div className="subtle">Waiting for the server…</div>
      )}
      <div className="row" style={{ marginTop: 6 }}>
        <button className="btn" onClick={props.onNew} title="Pick another shipped scenario">New game…</button>
        <button className="btn" onClick={props.onReview} title="The after-action review so far">Review</button>
        <button className="btn" onClick={props.onSave} title="Write the game record on the server (--record)">Save</button>
      </div>
      <div className="row">
        <a className="link" href="/editor" target="_blank" rel="noopener">Scenario editor ↗</a>
        <a className="link" href="/behaviour" target="_blank" rel="noopener">Behaviour Manager ↗</a>
        <a className="link muted" href="/classic">classic viewer</a>
      </div>
    </>
  );
}

export function ClockPanel(props: {
  frame: FrameView | null;
  live: number;
  scrub: boolean;
  playing: boolean;
  speed: number;
  waiting: number;
  over: boolean;
  limitS: number;
  onPlay: (p: boolean) => void;
  onSpeed: (s: number) => void;
  onSeek: (t: number) => void;
  onLive: () => void;
  onBranch: (t: number) => void;
}) {
  const t = props.frame?.t ?? 0;
  const [drag, setDrag] = useState<number | null>(null);
  return (
    <>
      <div className="group">Clock</div>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span className={`clock${props.scrub ? " scrub" : ""}`}>{clock(drag ?? t)}</span>
        <span className="subtle">{props.limitS > 0 ? `of ${clock(props.limitS)}` : ""}</span>
      </div>
      <button className="primary" disabled={props.over} onClick={() => props.onPlay(!props.playing)}>
        {props.over ? "Game over" : props.waiting > 0 ? `Waiting for your ${props.waiting} answer${props.waiting === 1 ? "" : "s"}` : props.playing ? "❚❚ Pause" : "▶ Play"}
      </button>
      <div className="chips fill" style={{ marginTop: 6 }}>
        {SPEEDS.map((x) => (
          <button key={x} className={`chip${props.speed === x ? " on" : ""}`} onClick={() => props.onSpeed(x)} title={`${x} simulated seconds each second`}>
            ×{x}
          </button>
        ))}
      </div>
      <input
        className="timeline"
        type="range"
        min={0}
        max={Math.max(1, Math.floor(props.live))}
        step={1}
        value={Math.floor(drag ?? t)}
        title="Drag to look back through the game"
        onChange={(e) => {
          const v = Number(e.target.value);
          setDrag(v);
          props.onSeek(v);
        }}
        onPointerUp={() => setDrag(null)}
        onKeyUp={() => setDrag(null)}
      />
      <div className="row">
        <button className="btn" disabled={!props.scrub} onClick={props.onLive} title="Back to the live game">Live</button>
        <button
          className="btn"
          disabled={!props.scrub}
          title="Restart the game from the moment shown and play on (what if?)"
          onClick={() => confirm(`Restart the game from ${clock(t)}? Everything after it is replaced.`) && props.onBranch(t)}
        >
          Branch here
        </button>
        {props.scrub && <span className="subtle">looking back: {clock(props.live - t)} before live</span>}
      </div>
    </>
  );
}

export function ViewPanel(props: {
  viewer: Viewer;
  onViewer: (v: Viewer) => void;
  layers: Set<LayerKey>;
  onLayer: (k: LayerKey) => void;
  basemap: Basemap;
  onBasemap: (b: Basemap) => void;
  hasPackMap: boolean;
  /** 3D terrain: on, and whether this game has the elevation for it (a pack's). */
  threeD: boolean;
  can3D: boolean;
  /** Why 3D is off, when it is. */
  no3D: string;
  onThreeD: (on: boolean) => void;
  hqOpen: boolean;
  onHq: () => void;
}) {
  return (
    <>
      <div className="group">View</div>
      <div className="chips fill">
        {(["umpire", "blue", "red"] as Viewer[]).map((v) => (
          <button key={v} className={`chip ${v}${props.viewer === v ? " on" : ""}`} onClick={() => props.onViewer(v)} title={v === "umpire" ? "The truth: every unit" : `What ${v} knows: its own units and its tracks of the enemy`}>
            {v === "umpire" ? "umpire" : `${v} eyes`}
          </button>
        ))}
      </div>
      <div className="chips">
        {LAYERS.map((l) => (
          <button key={l.key} className={`chip${props.layers.has(l.key) ? " on" : ""}`} onClick={() => props.onLayer(l.key)} title={l.title}>
            {l.label}
          </button>
        ))}
      </div>
      <div className="chips fill">
        {(["terrain", "dark", "streets", "satellite"] as Basemap[]).map((b) => (
          <button key={b} className={`chip${props.basemap === b ? " on" : ""}`} onClick={() => props.onBasemap(b)} title={b === "terrain" ? "The world overview (cached by the server), and the theatre pack's own map and hillshade over its box" : undefined}>
            {b}
          </button>
        ))}
      </div>
      <div className="chips fill">
        <button
          className={`chip${props.threeD && props.can3D ? " on" : ""}`}
          disabled={!props.can3D}
          onClick={() => props.onThreeD(!props.threeD)}
          title={
            props.can3D
              ? "The pack's elevation as 3D terrain (×1.5), buildings as blocks. Right-drag or Ctrl-drag to tilt and turn."
              : props.no3D
          }
        >
          3D terrain
        </button>
      </div>
      <button className={`chip${props.hqOpen ? " on" : ""}`} style={{ width: "100%" }} onClick={props.onHq} title="Take a side's HQ seat and give its units orders">
        {props.hqOpen ? "hide" : "show"} the HQ console
      </button>
    </>
  );
}

function strengthOf(units: FrameEntity[]) {
  const fit = units.reduce((n, e) => n + (e.status === "destroyed" ? 0 : e.status === "broken" ? 0 : e.fit), 0);
  const total = units.reduce((n, e) => n + e.total, 0);
  return total ? fit / total : 0;
}

export function ForcesPanel(props: { frame: FrameView | null; selected: string | null; onSelect: (name: string) => void; breakAt: number | null }) {
  const [open, setOpen] = useState<Record<string, boolean>>({ blue: true, red: true });
  const units = (props.frame?.entities ?? []).filter((e) => e.domain !== "drone");
  const sides = (["blue", "red"] as Side[]).filter((s) => units.some((e) => e.side === s));
  if (!sides.length) return null;
  return (
    <>
      <div className="group">Forces</div>
      {sides.map((s) => {
        const mine = units.filter((e) => e.side === s);
        const st = strengthOf(mine);
        return (
          <div key={s} style={{ marginBottom: 6 }}>
            <div className="row" style={{ cursor: "pointer", marginBottom: 2 }} onClick={() => setOpen((o) => ({ ...o, [s]: !o[s] }))}>
              <span style={{ color: SIDE_COLOUR[s], fontWeight: 700, textTransform: "uppercase", fontSize: 10 }}>{s}</span>
              <span className="subtle">{pct(st)} fighting{props.breakAt != null ? ` · breaks below ${100 - props.breakAt}%` : ""}</span>
              <span className="spacer" />
              <span className="subtle">{open[s] ? "▾" : "▸"}</span>
            </div>
            <div className="bar" style={{ marginBottom: 4 }}>
              <div style={{ width: pct(st), background: SIDE_COLOUR[s] }} />
            </div>
            {open[s] &&
              mine.map((e) => (
                <div key={e.id} className={`oob-unit${props.selected === e.name ? " sel" : ""}${e.status === "destroyed" ? " dead" : ""}`} onClick={() => props.onSelect(e.name)} title={`${e.platform} · ${e.activity}`}>
                  <span className="name">{e.label || e.name}</span>
                  <span className="bar">
                    <div style={{ width: pct(e.status === "destroyed" ? 0 : e.fit / Math.max(1, e.total)), background: SIDE_COLOUR[s] }} />
                  </span>
                  <span className="st" style={{ color: e.status === "ok" ? undefined : e.status === "suppressed" ? "#e8c547" : "#ff9f43" }}>
                    {e.status === "ok" ? e.activity : e.status}
                  </span>
                </div>
              ))}
          </div>
        );
      })}
    </>
  );
}

export function MapKey() {
  const [open, setOpen] = useState(false);
  const line = (c: string, dashed = false, w = 2) => <i style={{ borderTop: `${w}px ${dashed ? "dashed" : "solid"} ${c}` }} />;
  const ring = (c: string, fill = false) => <i style={{ width: 12, height: 12, marginLeft: 5, marginRight: 5, borderRadius: "50%", border: `2px solid ${c}`, background: fill ? c : "transparent" }} />;
  const rows: [ReactNode, string][] = [
    [line("#ff4d4d"), "shot that knocked something out"],
    [line("#ff9f43"), "shot that hit"],
    [line("#d7dbe6", true, 1), "shot that missed"],
    [ring("#ff9f43", true), "shell impact (red: a kill)"],
    [ring("#ff4d4d"), "drone strike (pale: it missed)"],
    [line("#e8c547", true, 1), "spotted an enemy"],
    [line("#8fc2ff", false, 3), "kill chain: shooter to aim point"],
    [line("#c4b5fd", false, 1), "kill chain: who called it"],
    [ring("#e8c547"), "suppressed"],
    [ring("#ff9f43"), "pinned"],
    [ring("#ff4d4d"), "broken"],
    [line("#ffffff", true), "selected: where it is going"],
    [ring("#e879f9"), "on the air (spectrum layer)"],
  ];
  return (
    <>
      <div className="group" style={{ cursor: "pointer" }} onClick={() => setOpen((o) => !o)}>
        Map key {open ? "▾" : "▸"}
      </div>
      {open &&
        rows.map(([mark, text]) => (
          <div key={text} className="key">
            {mark}
            {text}
          </div>
        ))}
    </>
  );
}

// ── Right pane ─────────────────────────────────────────────────────────────

export function Questions(props: { questions: Question[]; onAnswer: (id: number, choice: string) => void }) {
  const [open, setOpen] = useState<number | null>(null);
  if (!props.questions.length) return null;
  const fallback = (q: Question) => (q.options.some((o) => o.id === q.fallback) ? q.fallback : q.options[0]?.id ?? "keep");
  return (
    <div className="questions">
      <div className="row" style={{ marginBottom: 2 }}>
        <span style={{ color: "#e8c547", fontWeight: 700 }}>
          Paused: {props.questions.length} decision{props.questions.length === 1 ? "" : "s"} for you
        </span>
        <span className="spacer" />
        <button className="link" onClick={() => props.questions.forEach((q) => props.onAnswer(q.id, fallback(q)))} title="Answer every one with its template's default">
          use the defaults
        </button>
      </div>
      {props.questions.map((q) => (
        <div key={q.id} className="question">
          <div>
            <b>{q.agent}</b> <span className="subtle">({q.trigger})</span>: {q.title}
          </div>
          <div className="subtle" style={{ lineHeight: 1.45 }}>{q.question}</div>
          {q.situation && (
            <button className="link muted" onClick={() => setOpen((o) => (o === q.id ? null : q.id))}>
              {open === q.id ? "hide" : "what it knows"}
            </button>
          )}
          {open === q.id && <div className="feed-detail" style={{ whiteSpace: "pre-wrap" }}>{q.situation}</div>}
          <div className="opts">
            {q.options.map((o) => (
              <button key={o.id} className={`opt${o.id === q.fallback ? " default" : ""}`} onClick={() => props.onAnswer(q.id, o.id)} title={`${o.action}${o.id === q.rules_choice ? " · what the rules would pick" : ""}`}>
                {o.summary || o.id}
                {o.id === q.rules_choice ? " ★" : ""}
              </button>
            ))}
            {!q.options.length && (
              <button className="opt" onClick={() => props.onAnswer(q.id, "keep")}>
                Keep current orders
              </button>
            )}
          </div>
        </div>
      ))}
      <div className="subtle" style={{ marginTop: 4 }}>★ what the rules would pick · gold edge: the template's default</div>
    </div>
  );
}

const CATEGORIES: Category[] = ["decision", "fire", "sense", "c2", "drones", "ew", "people", "logistics", "world"];

export function Feed(props: { events: GameEvent[]; onSelect: (name: string) => void }) {
  const [hidden, setHidden] = useState<Set<Category>>(new Set(["world"]));
  const [open, setOpen] = useState<string | null>(null);
  const shown = useMemo(() => {
    const out: GameEvent[] = [];
    for (let i = props.events.length - 1; i >= 0 && out.length < 400; i--) {
      const e = props.events[i];
      if (e.kind === "decide.question" || e.kind === "decide.answer") continue;
      if (!hidden.has(category(e.kind))) out.push(e);
    }
    return out;
  }, [props.events, hidden]);
  return (
    <>
      <div className="group">What is happening</div>
      <div className="chips">
        {CATEGORIES.map((c) => (
          <button
            key={c}
            className={`chip${hidden.has(c) ? "" : " on"}`}
            style={hidden.has(c) ? undefined : { borderColor: CATEGORY_COLOUR[c], color: CATEGORY_COLOUR[c] }}
            onClick={() =>
              setHidden((h) => {
                const n = new Set(h);
                if (n.has(c)) n.delete(c);
                else n.add(c);
                return n;
              })
            }
          >
            {CATEGORY_LABEL[c]}
          </button>
        ))}
      </div>
      {shown.length === 0 && <div className="subtle">Nothing yet: press Play.</div>}
      {shown.map((e, i) => {
        const cat = category(e.kind);
        const key = `${e.t}|${e.kind}|${e.entity}|${e.text}`;
        const byModel = cat === "decision" && e.data?.source?.kind && e.data.source.kind !== "rules";
        const colour = cat === "fire" && e.side ? SIDE_COLOUR[e.side] : byModel ? "#e8c547" : CATEGORY_COLOUR[cat];
        return (
          <div key={i} className={`feed-entry${e.entity || cat === "decision" ? " click" : ""}`} style={{ color: colour }}>
            <span
              onClick={() => {
                if (e.entity) props.onSelect(e.entity);
                if (cat === "decision") setOpen((o) => (o === key ? null : key));
              }}
              title={cat === "decision" ? "Click for the options and who chose" : e.entity ? "Click to select it on the map" : undefined}
            >
              <span className="t">{e.clock.slice(0, 8)}</span>
              {e.text}
            </span>
            {open === key && cat === "decision" && <DecisionDetail e={e} />}
          </div>
        );
      })}
    </>
  );
}

function DecisionDetail({ e }: { e: GameEvent }) {
  const d = e.data ?? {};
  const p: Record<string, number> | null = d.probabilities ?? null;
  return (
    <div className="feed-detail">
      <div>
        {d.title} ({d.trigger}) → <b>{d.summary}</b> <span className="subtle">[{d.action}]</span>
      </div>
      <div>
        offered: {(d.options ?? []).map((o: { id: string; summary: string }) => `${o.id === d.option ? "▶ " : ""}${o.summary}${p?.[o.id] != null ? ` ${Math.round(p[o.id] * 100)}%` : ""}`).join(" · ")}
      </div>
      <div>
        decided by {d.source?.kind ?? "?"}
        {d.rationale ? `: ${d.rationale}` : ""}
      </div>
    </div>
  );
}

export function Inspector(props: {
  unit: UnitDetail | null;
  selected: string | null;
  viewer: Viewer;
  frame: FrameView | null;
  viewshedOn: boolean;
  onClose: () => void;
  onWhy: (label: string) => void;
  onMessages: (name: string) => void;
  onViewshed: (name: string | null) => void;
  onDetect: (observer: string, target: string) => void;
}) {
  const [target, setTarget] = useState("");
  const u = props.unit;
  if (!props.selected) return <div className="subtle" style={{ lineHeight: 1.5 }}>Click a unit on the map or in Forces to see what it is doing, its orders, its people and what it knows.</div>;
  if (!u || u.name !== props.selected) return <div className="subtle">Loading {props.selected}…</div>;
  const header = (
    <div className="row" style={{ justifyContent: "space-between", flexWrap: "nowrap" }}>
      <span style={{ fontWeight: 700, color: SIDE_COLOUR[u.side], overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
        {u.label ?? u.name} <span className="subtle">({u.name})</span>
      </span>
      <button className="link muted" onClick={props.onClose} aria-label="Close">
        ✕
      </button>
    </div>
  );
  if (u.gone) return <>{header}<div className="subtle">Not on the board.</div></>;
  if (!u.own) {
    return (
      <>
        {header}
        <Line label="known as">{u.known}</Line>
        {u.platform && <Line label="type">{u.platform}</Line>}
        {u.lastSeenS != null && <Line label="last seen">{span(u.lastSeenS)} ago</Line>}
        <div className="subtle" style={{ marginTop: 4 }}>Switch to the umpire view to see its orders and state.</div>
      </>
    );
  }
  const a = u.activity;
  const p = u.people;
  const h = u.health;
  const enemies = (props.frame?.entities ?? []).filter((x) => x.side !== u.side && x.domain !== "drone" && x.status !== "destroyed");
  return (
    <>
      {header}
      <div className="row" style={{ marginBottom: 6 }}>
        <button className="btn" onClick={() => props.onWhy(u.label ?? u.name)} title="What happened to this unit, with the figures behind it">Why?</button>
        <button className="btn" onClick={() => props.onMessages(u.name)} title="Its latest messages and why each was slow or lost">Messages</button>
        <button className={`btn${props.viewshedOn ? " on" : ""}`} onClick={() => props.onViewshed(props.viewshedOn ? null : u.name)} title="What the selected unit can see over the ground and through woods and towns, how far it looks and how far its weapons reach (kept for every unit you select)">
          {props.viewshedOn ? "Hide what it sees" : "Show what it sees"}
        </button>
      </div>
      <Line label="type">
        {u.platform} · {u.role}
        {u.hq ? " · HQ" : ""}
        {u.dummy ? " · decoy" : ""}
      </Line>
      {h && (
        <Line label="state">
          {h.destroyed ? "destroyed" : `${h.fit} of ${h.start} fit`}
          {h.mobilityKills ? ` · ${h.mobilityKills} immobilised` : ""}
          {h.firepowerKills ? ` · ${h.firepowerKills} cannot fire` : ""}
        </Line>
      )}
      {a && (
        <>
          <Line label="doing">
            {a.word}
            {a.detail ? ` — ${a.detail}` : ""}
            {a.deciding ? " · deciding…" : ""}
          </Line>
          <Line label="posture">
            {a.posture} · fires at {({ never: "nothing (weapons hold)", ifFiredUpon: "only once fired on", withinShortRange: "anything inside short range", always: "anything in reach" } as Record<string, string>)[a.roe] ?? a.roe}
          </Line>
          <div className="group">Orders</div>
          <Line label="purpose">{a.mission.purpose || "—"}{a.mission.urgent ? " (urgent)" : ""}</Line>
          {a.mission.phases.length > 0 && (
            <Line label="phases">
              {a.mission.phases.map((ph, i) => (
                <span key={i} style={{ color: i === a.mission.phase ? "#e8c547" : i < a.mission.phase ? "#6a7292" : undefined }}>
                  {i ? " → " : ""}
                  {ph}
                </span>
              ))}
            </Line>
          )}
          <Line label="behaviour">{u.template}</Line>
        </>
      )}
      {p && (
        <>
          <div className="group">People</div>
          <Line label="morale">
            {p.morale} · stress {p.stress.toFixed(1)} · cohesion {pct(p.cohesion)}
          </Line>
          <Line label="suppression">
            {p.pinned ? "pinned" : p.suppression > 0.3 ? "suppressed" : p.suppression > 0.02 ? "under some pressure" : "none"} ({pct(p.suppression)})
          </Line>
          <Line label="fatigue">
            {pct(p.fatigue)} effective{p.sleepDebtH > 0.5 ? ` · ${p.sleepDebtH.toFixed(1)} h sleep owed` : ""} · skill {pct(p.skill)}
          </Line>
          {p.lastFiredOnS != null && (
            <Line label="fired on">
              {span(p.lastFiredOnS)} ago{p.lastFiredOnBy ? ` by ${p.lastFiredOnBy}` : " (shooter unseen)"}
            </Line>
          )}
        </>
      )}
      {u.mounts && u.mounts.length > 0 && (
        <>
          <div className="group">Weapons</div>
          {u.mounts.map((m, i) => (
            <Line key={i} label={m.weapon}>
              {m.rounds} × {m.munition}
            </Line>
          ))}
        </>
      )}
      <div className="group">What it knows</div>
      {!u.knows?.length && <div className="subtle">No enemy seen.</div>}
      {u.knows?.slice(0, 8).map((k, i) =>
        k.suspect ? (
          <Line key={i} label="suspects">
            fire from {Math.round(k.bearingDeg ?? 0)}° ({span(k.agoS)} ago)
          </Line>
        ) : (
          <Line key={i} label={k.level ?? "seen"}>
            {k.name}
            {k.rangeM != null ? ` · ${(k.rangeM / 1000).toFixed(1)} km` : ""} · {k.agoS < 2 ? "in sight" : `${span(k.agoS)} ago`}
            {k.moving ? " · moving" : ""}
          </Line>
        ),
      )}
      {props.viewer === "umpire" && enemies.length > 0 && (
        <div className="row" style={{ marginTop: 4, flexWrap: "nowrap" }}>
          <select className="select" value={target} onChange={(e) => setTarget(e.target.value)} aria-label="Enemy unit">
            <option value="">Can it see…?</option>
            {enemies.map((x) => (
              <option key={x.id} value={x.name}>
                {x.label || x.name}
              </option>
            ))}
          </select>
          <button className="btn" disabled={!target} onClick={() => props.onDetect(u.name, target)} title="Every factor in whether it would detect that unit now">
            Check
          </button>
        </div>
      )}
      {u.decisions && u.decisions.length > 0 && (
        <>
          <div className="group">Last decisions</div>
          {u.decisions.map((d, i) => (
            <Line key={i} label={clock(d.t)}>
              {d.data?.title}: {d.data?.summary} <span style={{ color: d.data?.source?.kind === "rules" ? "#6a7292" : "#e8c547" }}>({d.data?.source?.kind})</span>
            </Line>
          ))}
        </>
      )}
      {u.recent && u.recent.length > 0 && (
        <>
          <div className="group">Lately</div>
          {u.recent.map((r, i) => (
            <Line key={i} label={clock(r.t)}>
              {r.text}
            </Line>
          ))}
        </>
      )}
    </>
  );
}

// ── Floating panels and cards ──────────────────────────────────────────────

const TASKS: [string, string][] = [
  ["", "—"],
  ["advance", "advance to the objective"],
  ["assault", "assault the objective"],
  ["hold", "hold"],
  ["observe", "observe"],
  ["overwatch", "overwatch"],
  ["withdraw", "withdraw from the objective"],
];

/** A point `m` metres from `from`, directly away from `away`. */
function awayFrom(from: { lat: number; lon: number }, away: { lat: number; lon: number }, m: number) {
  const mLat = 111320;
  const mLon = 111320 * Math.cos((from.lat * Math.PI) / 180);
  const de = (from.lon - away.lon) * mLon;
  const dn = (from.lat - away.lat) * mLat;
  const d = Math.hypot(de, dn) || 1;
  return { lat: from.lat + ((dn / d) * m) / mLat, lon: from.lon + ((de / d) * m) / mLon };
}

export function HqConsole(props: {
  viewer: Viewer;
  frame: FrameView | null;
  objective: { lat: number; lon: number } | null;
  picking: boolean;
  onPick: () => void;
  onSeat: (side: Side, seat: "planner" | "person" | "rules") => void;
  onIssue: (side: Side, plan: unknown) => void;
  onViewer: (v: Viewer) => void;
  onClose: () => void;
}) {
  const [plan, setPlan] = useState("");
  const [roe, setRoe] = useState("withinShortRange");
  const [tasks, setTasks] = useState<Record<string, string>>({});
  const [note, setNote] = useState("");
  const side = props.viewer === "umpire" ? null : props.viewer;
  const units = (props.frame?.entities ?? []).filter((e) => e.side === side && e.domain !== "drone" && e.status !== "destroyed");
  const issue = () => {
    if (!side) return;
    const orders = [];
    for (const u of units) {
      const task = tasks[u.name];
      if (!task) continue;
      const needsObj = ["advance", "assault", "withdraw"].includes(task);
      if (needsObj && !props.objective) {
        setNote("pick an objective on the map first");
        return;
      }
      const o = props.objective!;
      const phases = {
        advance: [{ label: "advance", do: "move", to: o, mode: "tactical" }, { label: "hold the objective", do: "overwatch" }],
        assault: [{ label: "assault", do: "move", to: o, mode: "assault" }, { label: "hold the objective", do: "overwatch" }],
        withdraw: [{ label: "withdraw", do: "withdraw", to: needsObj ? awayFrom(u, o, 1500) : null }, { label: "hold", do: "hold" }],
        hold: [{ label: "hold", do: "hold" }],
        observe: [{ label: "observe", do: "observe" }],
        overwatch: [{ label: "overwatch", do: "overwatch" }],
      }[task];
      orders.push({ unit: u.name, task, intent: plan || task, roe, phases });
    }
    if (!orders.length) {
      setNote("give at least one unit a task");
      return;
    }
    props.onIssue(side, { plan: plan || `${side} HQ orders`, orders });
    setNote(`${orders.length} order${orders.length === 1 ? "" : "s"} sent over the nets: units act on them when they arrive`);
  };
  return (
    <div className="floating hq">
      <div className="row" style={{ justifyContent: "space-between" }}>
        <b style={{ color: side ? SIDE_COLOUR[side] : undefined }}>{side ? `${cap(side)} HQ` : "HQ console"}</b>
        <button className="link muted" onClick={props.onClose}>✕</button>
      </div>
      {!side ? (
        <>
          <div className="subtle" style={{ lineHeight: 1.5, marginBottom: 6 }}>Orders go out from one side's HQ, and you see what reaches it. Pick a side:</div>
          <div className="chips fill">
            <button className="chip blue" onClick={() => props.onViewer("blue")}>command blue</button>
            <button className="chip red" onClick={() => props.onViewer("red")}>command red</button>
          </div>
        </>
      ) : (
        <>
          <div className="row">
            <span className="subtle" style={{ width: 70 }}>HQ seat</span>
            <select className="select" style={{ flex: 1 }} defaultValue="" onChange={(e) => e.target.value && props.onSeat(side, e.target.value as "planner" | "person" | "rules")}>
              <option value="">as the scenario says</option>
              <option value="person">me: the HQ waits for my orders</option>
              <option value="planner">the planner (model or heuristic)</option>
              <option value="rules">the rules</option>
            </select>
          </div>
          <div className="row">
            <input className="input" value={plan} onChange={(e) => setPlan(e.target.value)} placeholder="The plan in a few words (the intent)" />
          </div>
          <div className="row">
            <button className={`btn${props.picking ? " on" : ""}`} onClick={props.onPick}>{props.picking ? "click the map…" : "Pick objective on map"}</button>
            <span className="subtle">{props.objective ? `${props.objective.lat.toFixed(4)}, ${props.objective.lon.toFixed(4)}` : "no objective yet"}</span>
          </div>
          <div className="row">
            <span className="subtle" style={{ width: 70 }}>Fire at</span>
            <select className="select" style={{ flex: 1 }} value={roe} onChange={(e) => setRoe(e.target.value)}>
              <option value="always">anything in reach</option>
              <option value="withinShortRange">anything inside short range</option>
              <option value="ifFiredUpon">only once fired on</option>
              <option value="never">nothing (weapons hold)</option>
            </select>
          </div>
          <table>
            <tbody>
              {units.map((u) => (
                <tr key={u.id}>
                  <td>{u.label || u.name}</td>
                  <td style={{ width: 180 }}>
                    <select className="select" value={tasks[u.name] ?? ""} onChange={(e) => setTasks((t) => ({ ...t, [u.name]: e.target.value }))}>
                      {TASKS.map(([v, t]) => (
                        <option key={v} value={v}>
                          {t}
                        </option>
                      ))}
                    </select>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="row" style={{ marginTop: 6 }}>
            <button className="primary" onClick={issue}>Issue orders</button>
          </div>
          {note && <div className="subtle" style={{ color: "#e8c547" }}>{note}</div>}
        </>
      )}
    </div>
  );
}

export function Spectrum({ frame }: { frame: FrameView }) {
  const W = 1000;
  const H = 112;
  const x = (mhz: number) => (Math.log10(Math.max(mhz, 1)) / Math.log10(20000)) * W;
  const y = (w: number) => H - 14 - Math.min(1, Math.max(0.08, (Math.log10(Math.max(w, 0.01)) + 2) / 6)) * (H - 20);
  return (
    <div className="floating spectrum">
      <div className="subtle">On the air: frequency (log), height by power; jammers as bands</div>
      <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
        {[1, 10, 100, 1000, 10000].map((m) => (
          <g key={m}>
            <line x1={x(m)} x2={x(m)} y1={0} y2={H - 14} stroke="currentColor" strokeOpacity={0.15} />
            <text x={x(m) + 3} y={H - 2} fontSize={10} fill="currentColor" fillOpacity={0.6}>
              {m >= 1000 ? `${m / 1000} GHz` : `${m} MHz`}
            </text>
          </g>
        ))}
        {frame.emitters.map((e, i) => {
          const c = SIDE_COLOUR[e.side];
          if (e.kind === "jammer") {
            const x0 = x(e.freq_mhz - e.bw_mhz / 2);
            const x1 = x(e.freq_mhz + e.bw_mhz / 2);
            return (
              <rect key={i} x={x0} y={y(e.erp_w)} width={Math.max(2, x1 - x0)} height={H - 14 - y(e.erp_w)} fill={c} fillOpacity={0.25} stroke={c}>
                <title>jammer {e.freq_mhz.toFixed(0)} MHz ±{(e.bw_mhz / 2).toFixed(0)}, {e.erp_w.toFixed(0)} W</title>
              </rect>
            );
          }
          return (
            <line key={i} x1={x(e.freq_mhz)} x2={x(e.freq_mhz)} y1={y(e.erp_w)} y2={H - 14} stroke={c} strokeWidth={2}>
              <title>{e.kind} {e.freq_mhz.toFixed(1)} MHz, {e.erp_w.toFixed(1)} W</title>
            </line>
          );
        })}
      </svg>
    </div>
  );
}

export function ReportCard(props: { title: string; text: string; onClose: () => void }) {
  return (
    <div className="veil" onClick={props.onClose}>
      <div className="card wide" onClick={(e) => e.stopPropagation()}>
        <div className="row" style={{ justifyContent: "space-between" }}>
          <b>{props.title}</b>
          <button className="btn" onClick={props.onClose}>Close</button>
        </div>
        <pre>{props.text}</pre>
      </div>
    </div>
  );
}

export function EndCard(props: { frame: FrameView; events: GameEvent[]; onReview: () => void; onSave: () => void; onReplay: () => void; onNew: () => void; onClose: () => void }) {
  const o = props.frame.over!;
  const colour = o.winner ? SIDE_COLOUR[o.winner] : "#e8c547";
  const decisions = props.events.filter((e) => e.kind === "decide.apply");
  const byModel = decisions.filter((e) => e.data?.source?.kind && e.data.source.kind !== "rules").length;
  const units = props.frame.entities.filter((e) => e.domain !== "drone");
  return (
    <div className="veil">
      <div className="card" role="dialog" aria-modal="true" aria-label="Game ended">
        <div className="group" style={{ borderBottom: "none", marginBottom: 2 }}>Game ended · {clock(props.frame.t)}</div>
        <h2 style={{ color: colour }}>{o.winner ? `${cap(o.winner)} wins` : "Drawn"}</h2>
        <div style={{ color: "#c7ccdb", lineHeight: 1.5, marginBottom: 14 }}>{cap(o.reason)}.</div>
        <div className="row" style={{ gap: 10, flexWrap: "nowrap", marginBottom: 14 }}>
          {(["blue", "red"] as const).map((s) => {
            const mine = units.filter((e) => e.side === s);
            const lost = mine.reduce((n, e) => n + (e.total - (e.status === "destroyed" ? 0 : e.fit)), 0);
            const total = mine.reduce((n, e) => n + e.total, 0);
            const gone = mine.filter((e) => e.status === "destroyed").length;
            const broken = mine.filter((e) => e.status === "broken").length;
            return (
              <div key={s} style={{ flex: 1, padding: 8, border: `1px solid ${SIDE_COLOUR[s]}55`, borderRadius: 3 }}>
                <div style={{ color: SIDE_COLOUR[s], fontWeight: 700, textTransform: "uppercase", fontSize: 10, marginBottom: 4 }}>{s}</div>
                {mine.length ? (
                  <div className="subtle" style={{ color: "#c7ccdb", lineHeight: 1.6 }}>
                    {pct(o.strength[s] ?? 0)} strength left
                    <br />
                    {lost} of {total} platforms lost
                    <br />
                    {gone} of {mine.length} units destroyed{broken ? `, ${broken} broken` : ""}
                  </div>
                ) : (
                  <div className="subtle">{pct(o.strength[s] ?? 0)} strength left</div>
                )}
              </div>
            );
          })}
        </div>
        <div className="subtle" style={{ marginBottom: 14 }}>
          {decisions.length} decisions, {byModel} by a model
        </div>
        <div className="row" style={{ gap: 8, flexWrap: "nowrap" }}>
          <button className="primary" style={{ flex: 1 }} onClick={props.onReview}>After-action review</button>
          <button className="btn" style={{ flex: 1 }} onClick={props.onReplay} title="Replay the game from the start, then play on (same seed)">↻ From the start</button>
          <button className="btn" style={{ flex: 1 }} onClick={props.onNew}>+ New game</button>
        </div>
        <div className="row" style={{ marginTop: 12 }}>
          <button className="link" onClick={props.onSave}>Save the game record</button>
          <span className="spacer" />
          <button className="link muted" onClick={props.onClose}>close and look at the battlefield</button>
        </div>
      </div>
    </div>
  );
}

export function NewGameCard(props: {
  scenarios: ScenarioBrief[] | null;
  packs: PackInfo[];
  terrain: TerrainProgress | null;
  /** The map's view, for "Generate ground" (west, south, east, north). */
  view: [number, number, number, number] | null;
  current: string | undefined;
  onOpen: (file: string, pack?: string) => void;
  onBuild: (name: string, bbox: [number, number, number, number], cellM: number) => void;
  onClose: () => void;
}) {
  const [ground, setGround] = useState("");
  const [name, setName] = useState("");
  const [cell, setCell] = useState(30);
  const v = props.view;
  const km = v ? [((v[2] - v[0]) * 111.32 * Math.cos((((v[1] + v[3]) / 2) * Math.PI) / 180)).toFixed(0), ((v[3] - v[1]) * 111.32).toFixed(0)] : null;
  const t = props.terrain;
  const building = t?.state === "running";
  const nameOk = /^[A-Za-z0-9_-]+$/.test(name);
  return (
    <div className="veil" onClick={props.onClose}>
      <div className="card wide" onClick={(e) => e.stopPropagation()}>
        <div className="row" style={{ justifyContent: "space-between" }}>
          <b>New game</b>
          <button className="btn" onClick={props.onClose}>Close</button>
        </div>

        <div className="group">Ground</div>
        <div className="row" style={{ flexWrap: "nowrap" }}>
          <select className="select" value={ground} onChange={(e) => setGround(e.target.value)} aria-label="Ground">
            <option value="">each scenario's own ground</option>
            {props.packs.map((p) => (
              <option key={p.name} value={p.name}>
                {p.name}: {p.sizeKm[0].toFixed(0)} × {p.sizeKm[1].toFixed(0)} km{p.roadsKm ? `, ${Math.round(p.roadsKm).toLocaleString()} km of road` : ""}
              </option>
            ))}
          </select>
        </div>
        <div className="subtle" style={{ lineHeight: 1.5, marginBottom: 6 }}>
          {ground
            ? `The scenario is moved onto ${ground}: its forces keep their layout, centred on the pack (the box must be big enough for them).`
            : "A theatre pack is real ground: elevation, land cover, towns and roads, built for a box."}
        </div>
        <details style={{ marginBottom: 8 }}>
          <summary className="subtle" style={{ cursor: "pointer" }}>Generate ground from the map's current view…</summary>
          <div className="callout">
            Downloads the view's elevation and OpenStreetMap data (only that area, cached) and builds a pack in <code>bgws-rs/packs/</code>.
            {km && (
              <>
                {" "}
                The view is about {km[0]} × {km[1]} km; zoom the map to frame the area first.
              </>
            )}
          </div>
          <div className="row" style={{ flexWrap: "nowrap" }}>
            <input className="input" value={name} onChange={(e) => setName(e.target.value)} placeholder="pack name, e.g. kaliningrad" aria-label="Pack name" />
            <select className="select" style={{ width: 120 }} value={cell} onChange={(e) => setCell(Number(e.target.value))} aria-label="Cell size">
              <option value={20}>20 m cells</option>
              <option value={30}>30 m cells</option>
              <option value={50}>50 m cells</option>
              <option value={100}>100 m cells</option>
            </select>
            <button className="btn" disabled={!v || !nameOk || building} onClick={() => v && props.onBuild(name, v, cell)}>
              Generate
            </button>
          </div>
          {t && (
            <div className="subtle" style={{ color: t.state === "failed" || t.state === "busy" ? "#e07a5f" : t.state === "done" ? "#8fd18f" : "#e8c547" }}>
              {t.name}: {t.state === "running" ? `${t.stage} ${Math.round((t.fraction ?? 0) * 100)}%` : t.state === "done" ? "built: pick it under Ground" : t.text}
            </div>
          )}
          {building && (
            <div className="bar" style={{ marginTop: 4 }}>
              <div style={{ width: `${Math.round((t?.fraction ?? 0) * 100)}%`, background: "#e8c547" }} />
            </div>
          )}
        </details>

        <div className="group">Scenario</div>
        {!props.scenarios && <div className="subtle">Loading…</div>}
        {props.scenarios?.map((s) => (
          <button key={s.file} className="scenario" onClick={() => props.onOpen(s.file, ground || undefined)} style={s.name === props.current ? { borderColor: "#e8c547" } : undefined}>
            <div>
              <b>{s.name}</b> {s.name === props.current && <span className="subtle">(playing now)</span>}
            </div>
            <div className="meta">
              {s.type} · {s.durationMin} min · {s.units} units · {s.file}
              {ground ? ` · on ${ground}` : ""}
            </div>
            {s.description && <div className="meta" style={{ color: "#c7ccdb" }}>{s.description}</div>}
          </button>
        ))}
      </div>
    </div>
  );
}
