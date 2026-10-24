// ── bgws/realtime/WorldPanel.tsx ───────────────────────────────────────────
// The world's inspectors (MULTI_DOMAIN_PLAN.md W13 steps 4–6, W12): what the
// map cannot say. Tabs:
//
//   Feed        the world's own feed, filterable by domain
//   Kill chains each strike as a bar from find to assess (W11)
//   HQs         each HQ: its picture, its plan, its nets, what is waiting
//   Logistics   requests, convoys, evacuations, node stocks (W10)
//   Network     outages, jammers, who is cut off (W4, W6, W9)
//   Figures     the research backlog, and a sensitivity sweep on a vignette
//
// Read-only: it shows the state the runner holds and never changes it.

import { useMemo, useState } from "react";
import { allFigures } from "./world/figures";
import { researchBacklog, sensitivity, sensitivityReport } from "./world/batch";
import { VIGNETTES } from "./world/vignettes";
import type { RtState } from "./engine/types";
import type { KillChain } from "./world/types";

type Tab = "feed" | "chains" | "hqs" | "logistics" | "network" | "figures";
const TABS: { id: Tab; label: string }[] = [
  { id: "feed", label: "Feed" },
  { id: "chains", label: "Kill chains" },
  { id: "hqs", label: "HQs" },
  { id: "logistics", label: "Logistics" },
  { id: "network", label: "Network" },
  { id: "figures", label: "Figures" },
];

const SIDE = { blue: "#8fc2ff", red: "#ff9e8f", neutral: "#c8ccd6" } as const;
const box: React.CSSProperties = { font: "11px/1.45 var(--font-mono, monospace)", color: "#c7ccdb" };
const subtle: React.CSSProperties = { color: "#8a91a8" };
const tabButton = (on: boolean): React.CSSProperties => ({
  font: "inherit",
  background: on ? "#1f2a44" : "transparent",
  color: on ? "#e9ecfb" : "#8a91a8",
  border: "1px solid #2a3350",
  borderRadius: 4,
  padding: "2px 6px",
  cursor: "pointer",
});
const mins = (s: number) => `${Math.round(s / 60)} min`;

export default function WorldPanel(props: { state: RtState; onClose?: () => void }) {
  const { state } = props;
  const [tab, setTab] = useState<Tab>("feed");
  const w = state.world;
  if (!w) return <div style={{ ...box, ...subtle }}>This game has no world layer: land only.</div>;
  return (
    <div style={box}>
      <div style={{ display: "flex", gap: 4, flexWrap: "wrap", marginBottom: 6, alignItems: "center" }}>
        {TABS.map((t) => (
          <button key={t.id} style={tabButton(tab === t.id)} onClick={() => setTab(t.id)}>
            {t.label}
          </button>
        ))}
        {props.onClose && (
          <button onClick={props.onClose} style={{ ...tabButton(false), marginLeft: "auto" }} aria-label="Close">
            ✕
          </button>
        )}
      </div>
      {tab === "feed" && <Feed state={state} />}
      {tab === "chains" && <Chains chains={w.chains} now={state.time} />}
      {tab === "hqs" && <Hqs state={state} />}
      {tab === "logistics" && <Logistics state={state} />}
      {tab === "network" && <Network state={state} />}
      {tab === "figures" && <Figures />}
    </div>
  );
}

function Feed({ state }: { state: RtState }) {
  const [domain, setDomain] = useState("all");
  const log = state.world!.log;
  const domains = useMemo(() => ["all", ...new Set(log.map((l) => l.domain))].sort(), [log]);
  const shown = log.filter((l) => domain === "all" || l.domain === domain).slice(-80).reverse();
  return (
    <>
      <select value={domain} onChange={(e) => setDomain(e.target.value)} style={{ font: "inherit", marginBottom: 4 }}>
        {domains.map((d) => (
          <option key={d}>{d}</option>
        ))}
      </select>
      <div style={{ maxHeight: 320, overflowY: "auto" }}>
        {shown.map((l, i) => (
          <div key={`${l.time}-${i}`}>
            <span style={subtle}>{Math.floor(l.time / 60)}:{String(l.time % 60).padStart(2, "0")} </span>
            <span style={{ color: SIDE[l.side as keyof typeof SIDE] ?? "#c8ccd6" }}>{l.domain}</span> {l.text}
          </div>
        ))}
      </div>
    </>
  );
}

/** Each chain as a bar on a shared time axis: found, fixed, targeted, engaged, assessed. */
function Chains({ chains, now }: { chains: KillChain[]; now: number }) {
  const shown = chains.filter((k) => k.engaged != null || k.result == null).slice(-25);
  if (!shown.length) return <div style={subtle}>No kill chains yet.</div>;
  const t0 = Math.min(...shown.map((k) => k.found));
  const span = Math.max(60, now - t0);
  const x = (t: number) => `${(100 * (t - t0)) / span}%`;
  const STEPS: { key: keyof KillChain; colour: string; label: string }[] = [
    { key: "found", colour: "#6b7a90", label: "found" },
    { key: "fixed", colour: "#9ad1ff", label: "fixed" },
    { key: "targeted", colour: "#e8c547", label: "targeted" },
    { key: "engaged", colour: "#ff9f43", label: "engaged" },
    { key: "assessed", colour: "#5fd4c0", label: "assessed" },
  ];
  return (
    <div>
      <div style={{ ...subtle, marginBottom: 4 }}>
        {STEPS.map((s) => (
          <span key={s.key} style={{ marginRight: 8 }}>
            <span style={{ color: s.colour }}>■</span> {s.label}
          </span>
        ))}
      </div>
      {shown.map((k) => {
        const end = k.assessed ?? now;
        return (
          <div key={k.id} style={{ marginBottom: 4 }} title={`${k.id}: ${k.result ?? "running"} · ${k.means ?? "no shooter yet"} · $${Math.round(k.costUsd / 1000)}k · ${k.network}`}>
            <div>
              <span style={{ color: SIDE[k.side] }}>{k.truthId}</span> <span style={subtle}>{k.result ?? "running"}{k.means ? ` · ${k.means}` : ""} · {mins((k.engaged ?? now) - k.found)} to engage</span>
            </div>
            <div style={{ position: "relative", height: 6, background: "#141a2c", borderRadius: 3 }}>
              <div style={{ position: "absolute", left: x(k.found), width: `calc(${x(end)} - ${x(k.found)})`, top: 2, height: 2, background: "#2f3b5c" }} />
              {STEPS.map((s) => {
                const t = k[s.key] as number | undefined;
                return t == null ? null : <div key={s.key} style={{ position: "absolute", left: x(t), top: 0, width: 6, height: 6, marginLeft: -3, borderRadius: 3, background: s.colour }} />;
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function Hqs({ state }: { state: RtState }) {
  const w = state.world!;
  const hqs = Object.values(w.hqs).sort((a, b) => a.side.localeCompare(b.side) || a.id.localeCompare(b.id));
  if (!hqs.length) return <div style={subtle}>No HQs in this game.</div>;
  return (
    <div style={{ maxHeight: 360, overflowY: "auto" }}>
      {hqs.map((h) => {
        const lastPlan = [...w.log].reverse().find((l) => l.domain === "c2" && l.kind === "plan" && l.text.startsWith(h.id));
        const inbox = h.inbox.length;
        return (
          <div key={h.id} style={{ marginBottom: 8, borderLeft: `2px solid ${SIDE[h.side]}`, paddingLeft: 6 }}>
            <div>
              <b style={{ color: SIDE[h.side] }}>{h.id}</b> <span style={subtle}>{h.echelon} · post {h.postId}{h.down ? " · DOWN" : ""} · {h.seat} seat</span>
            </div>
            <div>picture: {Object.keys(h.cop).length} contacts{Object.values(h.cop).length ? `, freshest ${mins(state.time - Math.max(...Object.values(h.cop).map((t) => t.seenAt)))} old` : ""}</div>
            <div>{h.planning ? `planning (${h.planning.why}), ready in ${mins(h.planning.readyAt - state.time)}` : `next plan in ${mins(Math.max(0, h.nextPlanAt - state.time))}`}</div>
            {lastPlan && <div style={subtle}>last plan: {lastPlan.text.replace(`${h.id} issues its plan `, "")}</div>}
            <div style={subtle}>
              {h.subordinates.length} subordinates · {inbox} messages in
              {h.guidance ? ` · guidance: ${h.guidance.aims[0] ?? ""}` : ""}
            </div>
          </div>
        );
      })}
    </div>
  );
}

function Logistics({ state }: { state: RtState }) {
  const w = state.world!;
  const l = w.logistics;
  const nodes = Object.values(w.entities).filter((e) => e.domain === "node" && (e.archetype === "generic/depot" || e.archetype === "generic/railhead" || e.archetype === "generic/port"));
  return (
    <div style={{ maxHeight: 360, overflowY: "auto" }}>
      {nodes.map((n) => (
        <div key={n.id}>
          <span style={{ color: SIDE[n.owner] }}>{n.label}</span> <span style={subtle}>hp {n.hp.toFixed(1)} · fuel {Math.round(n.stores.fuelL / 1000)} kL · {Object.entries(n.stores.ammo).filter(([, v]) => (v ?? 0) > 0).map(([f, v]) => `${f} ${v}`).join(", ")}</span>
        </div>
      ))}
      {!l && <div style={subtle}>Logistics is not simulated in this game.</div>}
      {l && (
        <>
          <div style={{ marginTop: 6 }}>Requests waiting: {l.requests.length ? l.requests.map((r) => `${r.unitId} (${Math.round(r.need * 100)}%)`).join(", ") : "none"}</div>
          <div style={{ marginTop: 4 }}>Deliveries:</div>
          {l.jobs.slice(-12).map((j) => (
            <div key={j.id} style={subtle}>
              {j.id}: {j.means} {j.vehicleId ?? ""} → {j.unitId} · {j.state}
              {j.deliveredAt ? ` after ${mins(j.deliveredAt - j.startedAt)}` : ""}
            </div>
          ))}
          <div style={{ marginTop: 4 }}>Casualty evacuation:</div>
          {l.evacs.slice(-10).map((e) => (
            <div key={e.id} style={subtle}>
              {e.unitId}: {e.hitAt.length} wounded · {e.means} · {e.state}
              {e.doneAt ? ` in ${mins(e.doneAt - Math.min(...e.hitAt))}` : ""}
            </div>
          ))}
          <div style={{ marginTop: 4 }}>
            Evacuation times: {w.stats.evacuationS.length ? `median ${mins([...w.stats.evacuationS].sort((a, b) => a - b)[Math.floor(w.stats.evacuationS.length / 2)])} over ${w.stats.evacuationS.length}` : "none yet"}
            {w.stats.diedOfWounds ? ` · ${w.stats.diedOfWounds} died of wounds` : ""}
          </div>
        </>
      )}
    </div>
  );
}

function Network({ state }: { state: RtState }) {
  const w = state.world!;
  const outages = Object.entries(w.outages).filter(([, until]) => until > state.time);
  const silent = Object.entries(w.land).filter(([, c]) => c.emcon === "silent").map(([id]) => id);
  return (
    <div>
      <div>Outages: {outages.length ? outages.map(([k, until]) => `${k} (${until > 1e12 ? "for good" : `${mins(until - state.time)} more`})`).join(", ") : "none"}</div>
      <div style={{ marginTop: 4 }}>Jammers on: {w.jammers.filter((j) => j.on).map((j) => `${j.id} (${j.side}: ${j.bands.join("/")})`).join(", ") || "none"}</div>
      <div style={{ marginTop: 4 }}>Emitters silent: {silent.join(", ") || "none"}</div>
      <div style={{ marginTop: 4 }}>GNSS: blue {Math.round((w.pnt.blue?.global ?? 1) * 100)}% · red {Math.round((w.pnt.red?.global ?? 1) * 100)}% (before local jamming)</div>
      <div style={{ marginTop: 4 }}>
        Messages: {w.messages.length} sent, {w.messages.filter((m) => m.lost).length} lost · by bearer:{" "}
        {Object.entries(w.messages.reduce((m, x) => ({ ...m, [x.bearer ?? "none"]: (m[x.bearer ?? "none"] ?? 0) + 1 }), {} as Record<string, number>))
          .map(([b, n]) => `${b} ${n}`)
          .join(", ")}
      </div>
      <div style={{ marginTop: 4 }}>
        Report latency at HQs:{" "}
        {Object.entries(w.stats.reportLatencyByHq)
          .map(([hq, xs]) => `${hq} ${xs.length ? mins([...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]) : "—"}`)
          .join(" · ") || "—"}
      </div>
    </div>
  );
}

/** The research backlog, and a sensitivity sweep run here in the browser on a few seeded games. */
function Figures() {
  const [vignette, setVignette] = useState(VIGNETTES[0].id);
  const [report, setReport] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const backlog = useMemo(() => researchBacklog(15), []);
  const total = useMemo(() => Object.keys(allFigures()).length, []);
  const run = async () => {
    const v = VIGNETTES.find((x) => x.id === vignette)!;
    setBusy(true);
    try {
      setReport(sensitivityReport(v, await sensitivity(v, 2)));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div style={{ maxHeight: 360, overflowY: "auto" }}>
      <div style={subtle}>{total} figures in the registry. The least sourced, widest first:</div>
      {backlog.map((f) => (
        <div key={f.key}>
          {f.key} <span style={subtle}>({f.source}) {f.note}</span>
        </div>
      ))}
      <div style={{ marginTop: 8 }}>
        Sensitivity:{" "}
        <select value={vignette} onChange={(e) => setVignette(e.target.value)} style={{ font: "inherit" }}>
          {VIGNETTES.map((v) => (
            <option key={v.id} value={v.id}>
              {v.name}
            </option>
          ))}
        </select>{" "}
        <button onClick={() => void run()} disabled={busy} style={tabButton(false)}>
          {busy ? "running…" : "Run sweep"}
        </button>
      </div>
      {report && <pre style={{ whiteSpace: "pre-wrap", marginTop: 6, color: "#c7ccdb" }}>{report}</pre>}
    </div>
  );
}
