/**
 * BehaviourBench — try behaviour templates in a calibration scene, side by side.
 *
 * Pick a scene (realtime/engine/situations.ts: one decision a unit faces,
 * set up by hand) and one or two templates. Each column shows the path the
 * template takes, every option and why it is or is not on offer, and what the
 * rules would choose. It runs the game's own code (realtime/behaviour/bench.ts).
 */

import { useState } from "react";
import { BENCH_SCENES, benchScene, type BenchResult, type BenchStatus } from "./realtime/behaviour/bench";
import { allTemplates, customTemplates } from "./realtime/behaviour/store";
import { useBehaviours } from "./realtime/behaviour/useBehaviours";
import type { Bias } from "./realtime/behaviour/types";
import { DECISION_POINTS } from "./realtime/engine/decisions";

const STATUS_WORDS: Record<BenchStatus, string> = {
  offered: "offered",
  removed: "switched off by the template",
  condition: "its condition is not met here",
  "nothing here": "allowed, but nothing to offer here",
};
const LEAN: Record<Bias, string> = { [-2]: "strongly avoid", [-1]: "avoid", 0: "", 1: "prefer", 2: "strongly prefer" };

/** "land.pullBackToFriends" → "Pull back to friends". */
function actionLabel(action: string): string {
  const words = action.replace(/^land\./, "").replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function run(scene: string, templateId: string): { result?: BenchResult; error?: string } {
  try {
    return { result: benchScene(scene, templateId, customTemplates()) };
  } catch (e) {
    return { error: e instanceof Error ? e.message : String(e) };
  }
}

export default function BehaviourBench({ initialTemplate }: { initialTemplate: string }) {
  useBehaviours(); // re-run when templates are saved
  const templates = allTemplates();
  const [scene, setScene] = useState(BENCH_SCENES[0]?.id ?? "");
  const [a, setA] = useState(initialTemplate);
  const [b, setB] = useState<string>("");
  // One scene is a few milliseconds of the game's own code: run it on every render.
  const left = run(scene, a);
  const right = b ? run(scene, b) : null;
  const current = BENCH_SCENES.find((s) => s.id === scene);

  const picker = (label: string, value: string, onChange: (v: string) => void, optional = false) => (
    <label className="bm-field" style={{ flex: 1, minWidth: 200 }}>
      <span>{label}</span>
      <select className="bm-select" value={value} onChange={(e) => onChange(e.target.value)}>
        {optional && <option value="">— none —</option>}
        {templates.map((t) => (
          <option key={t.id} value={t.id}>
            {t.name ?? t.id}
          </option>
        ))}
      </select>
    </label>
  );

  return (
    <div>
      <div className="bm-head">
        <div className="bm-head-text">
          <h2 className="bm-name">Test bench</h2>
          <p className="bm-sub">
            Put a template in a scene and see what it offers there, what it leaves out and why, and what the rules
            would pick. Choose a second template to compare.
          </p>
        </div>
      </div>
      <div className="bm-card">
        <label className="bm-field">
          <span>Scene</span>
          <select className="bm-select" value={scene} onChange={(e) => setScene(e.target.value)}>
            {BENCH_SCENES.map((s) => (
              <option key={s.id} value={s.id}>
                {s.point} · {s.story}
              </option>
            ))}
          </select>
        </label>
        {current && (
          <p className="bm-card-sub" style={{ margin: 0 }}>
            <strong>{DECISION_POINTS[current.point].title}.</strong> {current.story}
          </p>
        )}
        <div className="bm-assign-row" style={{ marginTop: 12 }}>
          {picker("Template", a, setA)}
          {picker("Compare with", b, setB, true)}
        </div>
      </div>
      <div className="bm-bench">
        <BenchColumn outcome={left} />
        {right && <BenchColumn outcome={right} />}
      </div>
    </div>
  );
}

function BenchColumn({ outcome }: { outcome: { result?: BenchResult; error?: string } }) {
  if (outcome.error || !outcome.result) {
    return <div className="bm-card bm-callout">This template cannot run here: {outcome.error}</div>;
  }
  const r = outcome.result;
  const choice = r.offered.find((o) => o.id === r.rulesChoice);
  return (
    <div className="bm-card bm-bench-col">
      <h3 className="bm-card-title">{r.lineage.join(" ← ")}</h3>
      <p className="bm-card-sub">{r.question}</p>
      <div className="bm-bench-path" title="The graph nodes walked, trigger first">
        Path: {r.path.join(" → ") || "—"}
      </div>
      {r.doctrine.length > 0 && (
        <p className="bm-option-note" style={{ margin: "8px 0" }}>
          Doctrine: {r.doctrine.join(" ")}
        </p>
      )}
      <ul className="bm-options bm-bench-rows">
        {r.rows.map((row, i) => (
          <li key={`${row.action}-${i}`} className="bm-option" data-status={row.status}>
            <span className="bm-option-name">
              {actionLabel(row.action)}
              {LEAN[row.bias] && <span className="bm-badge">{LEAN[row.bias]}</span>}
            </span>
            <span className="bm-option-note">{STATUS_WORDS[row.status]}</span>
            {row.options.map((o) => (
              <span key={o.id} className="bm-option-note">
                {o.id === r.rulesChoice ? "▶ " : "• "}
                {o.summary}
              </span>
            ))}
          </li>
        ))}
      </ul>
      <div className="bm-bench-choice">
        The rules would choose: <strong>{choice?.summary ?? r.rulesChoice}</strong>
        <span className="bm-badge" data-tone={r.sensibleChoice ? "ok" : "warn"}>
          {r.sensibleChoice ? "a sensible choice" : "not one judged sensible here"}
        </span>
        <div className="bm-option-note">Sensible here (judged by hand): {r.situation.sensible.join(", ")}. Jev chooses among the offered options in play.</div>
      </div>
    </div>
  );
}
