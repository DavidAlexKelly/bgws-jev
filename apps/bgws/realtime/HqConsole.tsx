// ── bgws/realtime/HqConsole.tsx ────────────────────────────────────────────
// The HQ console (MULTI_DOMAIN_PLAN.md W15): a person in an HQ's seat. It
// shows only what the HQ has (console.ts: the inbox of what has arrived, its
// CCIR alerts, its fused picture with ages and errors, what it can tell of
// its own nets, and the fate of the orders it sent) and sends orders through
// the runner's `command`, so they travel the nets like anyone else's.
//
//   Take the seat / hand it back   the HQ's planner stops (or resumes)
//   FRAGO                          one unit: move to, hold, overwatch, or
//                                  engage a contact in the picture
//   Staff draft                    the heuristic's plan from this HQ's
//                                  picture, to approve as it stands

import { useMemo, useState } from "react";
import type { RtConfig, RtOrder, RtState, UnitOrders } from "./engine/types";
import { commandable, commanderAnswer, consoleView, frago, staffDraft } from "./world/command/console";
import type { PlanAnswer } from "./world/types";

type Kind = "move" | "hold" | "overwatch" | "engage";
const box: React.CSSProperties = { font: "11px/1.45 var(--font-mono, monospace)", color: "#c7ccdb" };
const subtle: React.CSSProperties = { color: "#8a91a8" };
const mins = (s: number) => (s < 90 ? `${Math.round(s)} s` : `${Math.round(s / 60)} min`);

export interface HqConsoleProps {
  state: RtState;
  config: RtConfig;
  side: "blue" | "red";
  onCommand: (hq: string, answer: PlanAnswer) => void;
  onSeat: (hq: string, seat: "human" | "claude" | "heuristic") => void;
}

export default function HqConsole({ state, config, side, onCommand, onSeat }: HqConsoleProps) {
  const hqs = Object.values(state.world?.hqs ?? {}).filter((h) => h.side === side);
  const [hqId, setHqId] = useState(hqs[0]?.id ?? "");
  const [unit, setUnit] = useState("");
  const [kind, setKind] = useState<Kind>("hold");
  const [target, setTarget] = useState("");
  const [task, setTask] = useState("");
  const [draft, setDraft] = useState<PlanAnswer | null>(null);
  const hq = state.world?.hqs[hqId];
  const view = useMemo(() => (hq ? consoleView(state, config, hq.id) : undefined), [state, config, hq]);
  const can = useMemo(() => (hq ? commandable(state, hq.id) : { units: [], hqs: [] }), [state, hq]);
  if (!hqs.length) return <div style={{ ...box, ...subtle }}>No {side} HQ in this game.</div>;
  if (!hq || !view) return null;
  const human = view.seat === "human";
  const track = view.cop.find((t) => t.id === target);

  const send = () => {
    const to = unit || can.units[0];
    if (!to) return;
    let order: RtOrder;
    if (kind === "move" && track) order = { kind: "move", to: track.at, mode: "tactical" };
    else if (kind === "engage" && track) order = { kind: "engage", targetId: track.id };
    else if (kind === "overwatch") order = { kind: "overwatch" };
    else order = { kind: "hold" };
    const words = task.trim() || `${kind}${track ? ` ${track.thought} (${track.id})` : ""}`;
    const orders: Record<string, UnitOrders> = { [to]: frago(state, order, words) };
    onCommand(hq.id, commanderAnswer(`FRAGO to ${to}: ${words}`, orders));
    setTask("");
  };

  return (
    <div style={box}>
      <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
        <select value={hq.id} onChange={(e) => setHqId(e.target.value)} style={{ font: "inherit" }}>
          {hqs.map((h) => (
            <option key={h.id} value={h.id}>
              {h.id} ({h.echelon})
            </option>
          ))}
        </select>
        <span style={subtle}>seat: {view.seat}{view.down ? " · POST DOWN, moving" : ""}</span>
        {human ? (
          <button onClick={() => onSeat(hq.id, "heuristic")}>hand back to the staff</button>
        ) : (
          <button onClick={() => onSeat(hq.id, "human")}>take the seat</button>
        )}
      </div>

      <Section title={`Alerts (${view.alerts.length})`}>
        {view.alerts.map((a, i) => (
          <div key={i}>
            <span style={subtle}>{mins(state.time - a.time)} ago</span> {a.text}
          </div>
        ))}
      </Section>
      <Section title={`Inbox (${view.inbox.length})`}>
        {view.inbox.slice(0, 15).map((m) => (
          <div key={m.id}>
            <span style={subtle}>
              {mins(state.time - m.arrivedAt)} ago · {m.from} by {m.bearer}
              {m.arrivedAt - m.sentAt > 60 ? ` · took ${mins(m.arrivedAt - m.sentAt)}` : ""}
            </span>{" "}
            {m.text}
          </div>
        ))}
      </Section>
      <Section title={`Picture (${view.cop.length})`}>
        {view.cop.slice(0, 20).map((t) => (
          <div key={t.id} onClick={() => setTarget(t.id)} style={{ cursor: "pointer", color: t.id === target ? "#e8c547" : undefined }}>
            {t.id}: {t.thought} <span style={subtle}>({t.domain}, {t.classification}, {mins(t.ageS)} old{t.errorM > 200 ? `, ±${Math.round(t.errorM)} m` : ""})</span>
          </div>
        ))}
      </Section>
      <Section title="Comms">
        {view.comms.map((c) => (
          <div key={c.unit} style={{ color: c.reachable ? undefined : "#e07a5f" }}>
            {c.unit}: {c.reachable ? `reachable${c.delayS ? `, ~${mins(c.delayS)}` : ""}` : "cannot be reached"}
            {c.lastHeardAgoS != null ? <span style={subtle}> · last heard {mins(c.lastHeardAgoS)} ago</span> : null}
          </div>
        ))}
      </Section>

      {human && (
        <>
          <Section title="FRAGO">
            <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
              <select value={unit || can.units[0] || ""} onChange={(e) => setUnit(e.target.value)} style={{ font: "inherit" }}>
                {can.units.map((u) => (
                  <option key={u}>{u}</option>
                ))}
              </select>
              <select value={kind} onChange={(e) => setKind(e.target.value as Kind)} style={{ font: "inherit" }}>
                <option value="hold">hold</option>
                <option value="overwatch">overwatch</option>
                <option value="move">move to contact</option>
                <option value="engage">engage contact</option>
              </select>
              {(kind === "move" || kind === "engage") && <span style={subtle}>{track ? `→ ${track.id}` : "click a contact above"}</span>}
              <input value={task} onChange={(e) => setTask(e.target.value)} placeholder="task, in words" style={{ font: "inherit", flex: 1, minWidth: 120 }} />
              <button disabled={!can.units.length || ((kind === "move" || kind === "engage") && !track)} onClick={send}>
                send
              </button>
            </div>
          </Section>
          <Section title="Staff draft">
            <button onClick={() => setDraft(staffDraft(state, hq.id) ?? null)}>ask the staff for a plan</button>
            {draft && (
              <div>
                <div>{draft.plan}</div>
                <div style={subtle}>{Object.keys(draft.orders).length} unit order(s){draft.guidance ? ", guidance for the HQs below" : ""}</div>
                <button
                  onClick={() => {
                    onCommand(hq.id, commanderAnswer(draft.plan, draft.orders, draft.guidance));
                    setDraft(null);
                  }}
                >
                  approve and send
                </button>{" "}
                <button onClick={() => setDraft(null)}>reject</button>
              </div>
            )}
          </Section>
        </>
      )}
      <Section title={`Sent (${view.sent.length})`}>
        {view.sent.map((m) => (
          <div key={m.id}>
            <span style={{ color: m.status === "lost" ? "#e07a5f" : m.status === "arrived" ? "#7fd18b" : "#e8c547" }}>{m.status}</span> → {m.to}: {m.text}
          </div>
        ))}
      </Section>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginTop: 6 }}>
      <div style={{ color: "#e9ecfb" }}>{title}</div>
      <div style={{ paddingLeft: 6 }}>{children}</div>
    </div>
  );
}
