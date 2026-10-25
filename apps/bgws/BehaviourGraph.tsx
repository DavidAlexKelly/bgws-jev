// ── bgws/BehaviourGraph.tsx ────────────────────────────────────────────────
// The behaviour editor's graph view (DECISION_FLOW_GRAPHS.md §5.1, step 10):
// the template drawn as a node canvas — triggers on the left, branches, and
// choose nodes with their options as ports — over its merged result, with
// every patch op reachable from the canvas:
//
//   an option   remove or restore it; set its bias; add or clear an
//               "only when" condition over the fact vocabulary
//   a node      override its title, question and fallback (the override is
//               stored in this template's own `nodes`, the parent untouched)
//   the whole   its doctrine
//
// Every change is saved as a patch on this template (store.ts validates the
// whole set before anything is kept). Built-in templates are read-only: copy
// one to edit it. Drawn in plain SVG: no canvas library is a dependency.

import { useMemo, useState } from "react";
import { layoutTemplate, ROW_H, type LaidNode } from "./realtime/behaviour/graphLayout";
import { isBuiltIn, saveCustomTemplate } from "./realtime/behaviour/store";
import { COMPARISONS, type Bias, type ChooseNode, type Comparison, type Condition, type FactValue, type Template } from "./realtime/behaviour/types";
import { ALL_FACTS } from "./realtime/behaviour/vocabulary";
import { resolveTemplate } from "./realtime/behaviour/walk";

type Selection = { kind: "node"; id: string } | { kind: "option"; node: string; option: string } | null;
const KIND_FILL: Record<LaidNode["kind"], string> = { trigger: "#20304f", choose: "#1c2438", branch: "#2a2238", do: "#1f3326" };

export default function BehaviourGraph({ template, templates }: { template: Template; templates: Template[] }) {
  const [sel, setSel] = useState<Selection>(null);
  const [problems, setProblems] = useState<string[]>([]);
  const readOnly = isBuiltIn(template.id);
  const resolved = useMemo(() => resolveTemplate(template.id, new Map(templates.map((t) => [t.id, t]))), [template, templates]);
  const graph = useMemo(() => layoutTemplate(resolved, new Set(Object.keys(template.nodes ?? {}))), [resolved, template]);
  const byId = new Map(graph.nodes.map((n) => [n.id, n]));

  const save = (patch: Partial<Template>) => {
    const next: Template = { ...template, ...patch, version: template.version + 1 };
    setProblems(saveCustomTemplate(next));
  };

  const edgePath = (from: LaidNode, to: LaidNode, row?: number) => {
    const y1 = from.y + (row != null ? ROW_H * (1.5 + row) : from.h / 2);
    const x1 = from.x + from.w;
    const x2 = to.x;
    const y2 = to.y + Math.min(to.h / 2, ROW_H);
    const mid = (x1 + x2) / 2;
    return `M${x1},${y1} C${mid},${y1} ${mid},${y2} ${x2},${y2}`;
  };

  return (
    <div style={{ display: "flex", gap: 12, alignItems: "flex-start", minHeight: 0 }}>
      <div style={{ flex: 1, overflow: "auto", border: "1px solid #2a3350", borderRadius: 6, maxHeight: "78vh", background: "#0d1017" }}>
        <svg width={graph.width} height={graph.height} style={{ font: "11px var(--font-mono, monospace)" }} role="img" aria-label={`Decision graph of ${template.name ?? template.id}`}>
          {graph.edges.map((e, i) => {
            const from = byId.get(e.from);
            const to = byId.get(e.to);
            if (!from || !to) return null;
            return <path key={i} d={edgePath(from, to, e.fromRow)} fill="none" stroke={e.label === "else" ? "#6b7a90" : "#8fa8d8"} strokeWidth={1.2} strokeDasharray={e.label === "else" ? "4 3" : undefined} />;
          })}
          {graph.nodes.map((n) => (
            <g key={n.id} transform={`translate(${n.x},${n.y})`}>
              <rect
                width={n.w}
                height={n.h}
                rx={n.kind === "choose" ? 8 : n.kind === "branch" ? 2 : 4}
                fill={KIND_FILL[n.kind]}
                stroke={sel?.kind === "node" && sel.id === n.id ? "#e8c547" : n.own ? "#c4b5fd" : "#3a4466"}
                strokeWidth={sel?.kind === "node" && sel.id === n.id ? 2 : 1}
                onClick={() => n.kind !== "trigger" && setSel({ kind: "node", id: n.id })}
                style={{ cursor: n.kind === "trigger" ? "default" : "pointer" }}
              />
              <text x={8} y={ROW_H} fill="#e9ecfb" style={{ pointerEvents: "none" }}>
                {n.kind === "trigger" ? `⚡ ${n.label}` : n.label.slice(0, 34)}
              </text>
              {n.options?.map((o, i) => {
                const on = sel?.kind === "option" && sel.node === n.id && sel.option === o.id;
                return (
                  <g key={o.id} transform={`translate(0,${ROW_H * (1.5 + i)})`} onClick={() => setSel({ kind: "option", node: n.id, option: o.id })} style={{ cursor: "pointer" }}>
                    <rect x={4} y={2} width={n.w - 8} height={ROW_H - 2} rx={3} fill={on ? "#33405f" : "transparent"} />
                    <text x={10} y={ROW_H - 4} fill={o.removed ? "#5b6278" : "#c7ccdb"} textDecoration={o.removed ? "line-through" : undefined}>
                      {o.id}
                      {o.bias ? ` ${o.bias > 0 ? "+" : ""}${o.bias}` : ""}
                      {o.conditional ? " ?" : ""}
                    </text>
                  </g>
                );
              })}
            </g>
          ))}
        </svg>
      </div>
      <aside style={{ width: 300, font: "12px/1.5 var(--font-mono, monospace)", color: "#c7ccdb" }}>
        {readOnly && <p style={{ color: "#e8c547" }}>Built-in: read-only. Copy it from the Templates tab to edit it here.</p>}
        <Doctrine template={template} readOnly={readOnly} onSave={(doctrine) => save({ doctrine })} />
        {sel?.kind === "option" && <OptionInspector template={template} node={resolved.nodes[sel.node] as ChooseNode} option={sel.option} readOnly={readOnly} onSave={save} resolvedRemoved={resolved.remove.has(sel.option)} />}
        {sel?.kind === "node" && resolved.nodes[sel.id]?.kind === "choose" && <NodeInspector template={template} id={sel.id} node={resolved.nodes[sel.id] as ChooseNode} readOnly={readOnly} onSave={save} />}
        {sel?.kind === "node" && resolved.nodes[sel.id]?.kind !== "choose" && <p>{sel.id}: {resolved.nodes[sel.id]?.kind} node. Branches and drills are edited in the template file.</p>}
        {!sel && <p style={{ color: "#8a91a8" }}>Click a node or an option to edit it. Lilac outline: set by this template; the rest is inherited.</p>}
        {problems.length > 0 && (
          <div style={{ color: "#e07a5f" }}>
            Not saved:
            {problems.map((p) => (
              <div key={p}>{p}</div>
            ))}
          </div>
        )}
      </aside>
    </div>
  );
}

function Doctrine({ template, readOnly, onSave }: { template: Template; readOnly: boolean; onSave: (d: string) => void }) {
  const [text, setText] = useState(template.doctrine ?? "");
  return (
    <div style={{ marginBottom: 10 }}>
      <div style={{ color: "#8a91a8" }}>Doctrine (told to Jev)</div>
      <textarea value={text} onChange={(e) => setText(e.target.value)} disabled={readOnly} rows={3} style={{ width: "100%", font: "inherit" }} />
      {!readOnly && text !== (template.doctrine ?? "") && <button onClick={() => onSave(text)}>Save doctrine</button>}
    </div>
  );
}

function OptionInspector(props: { template: Template; node: ChooseNode; option: string; readOnly: boolean; resolvedRemoved: boolean; onSave: (p: Partial<Template>) => void }) {
  const { template, option, readOnly, onSave } = props;
  const spec = props.node.options.find((o) => o.id === option);
  const removed = props.resolvedRemoved;
  const bias = template.bias?.[option] ?? spec?.bias ?? 0;
  const when = template.when?.[option];
  const [fact, setFact] = useState(ALL_FACTS[0]);
  const [op, setOp] = useState<Comparison>("==");
  const [value, setValue] = useState("true");
  if (!spec) return null;
  const parsed = (v: string): FactValue => (v === "true" ? true : v === "false" ? false : v === "null" ? null : Number.isFinite(Number(v)) ? Number(v) : v);
  return (
    <div>
      <b>{option}</b> <span style={{ color: "#8a91a8" }}>→ {spec.action}</span>
      <div style={{ marginTop: 6 }}>
        <label>
          <input
            type="checkbox"
            checked={!removed}
            disabled={readOnly}
            onChange={() => onSave({ remove: removed ? (template.remove ?? []).filter((r) => r !== option) : [...(template.remove ?? []), option] })}
          />{" "}
          offered
        </label>
      </div>
      <div>
        bias{" "}
        <select value={bias} disabled={readOnly} onChange={(e) => onSave({ bias: { ...(template.bias ?? {}), [option]: Number(e.target.value) as Bias } })}>
          {[-2, -1, 0, 1, 2].map((b) => (
            <option key={b} value={b}>
              {b > 0 ? `+${b}` : b}
            </option>
          ))}
        </select>
      </div>
      <div style={{ marginTop: 6 }}>
        only when: {when ? <code>{JSON.stringify(when)}</code> : <span style={{ color: "#8a91a8" }}>always</span>}
        {!readOnly && when && (
          <button
            onClick={() => {
              const next = { ...(template.when ?? {}) };
              delete next[option];
              onSave({ when: next });
            }}
          >
            clear
          </button>
        )}
      </div>
      {!readOnly && (
        <div style={{ display: "flex", gap: 4, flexWrap: "wrap", marginTop: 4 }}>
          <select value={fact} onChange={(e) => setFact(e.target.value)} style={{ maxWidth: 150 }}>
            {ALL_FACTS.map((f) => (
              <option key={f}>{f}</option>
            ))}
          </select>
          <select value={op} onChange={(e) => setOp(e.target.value as Comparison)}>
            {COMPARISONS.map((c) => (
              <option key={c}>{c}</option>
            ))}
          </select>
          <input value={value} onChange={(e) => setValue(e.target.value)} style={{ width: 60 }} />
          <button onClick={() => onSave({ when: { ...(template.when ?? {}), [option]: { fact, op, value: parsed(value) } as Condition } })}>set</button>
        </div>
      )}
    </div>
  );
}

function NodeInspector(props: { template: Template; id: string; node: ChooseNode; readOnly: boolean; onSave: (p: Partial<Template>) => void }) {
  const { template, id, node, readOnly, onSave } = props;
  const [title, setTitle] = useState(node.title);
  const [question, setQuestion] = useState(node.question);
  const [fallback, setFallback] = useState(node.fallback);
  const dirty = title !== node.title || question !== node.question || fallback !== node.fallback;
  return (
    <div>
      <b>{id}</b> <span style={{ color: "#8a91a8" }}>choose node{template.nodes?.[id] ? " (overridden here)" : " (inherited)"}</span>
      <div>title</div>
      <input value={title} disabled={readOnly} onChange={(e) => setTitle(e.target.value)} style={{ width: "100%" }} />
      <div>question</div>
      <textarea value={question} disabled={readOnly} onChange={(e) => setQuestion(e.target.value)} rows={2} style={{ width: "100%", font: "inherit" }} />
      <div>fallback</div>
      <select value={fallback} disabled={readOnly} onChange={(e) => setFallback(e.target.value)}>
        {node.options.map((o) => (
          <option key={o.id}>{o.id}</option>
        ))}
      </select>
      {!readOnly && dirty && (
        <div>
          <button onClick={() => onSave({ nodes: { ...(template.nodes ?? {}), [id]: { ...node, title, question, fallback } } })}>Save node</button>
        </div>
      )}
      {!readOnly && template.nodes?.[id] && (
        <button
          onClick={() => {
            const next = { ...(template.nodes ?? {}) };
            delete next[id];
            onSave({ nodes: next });
          }}
        >
          back to the inherited node
        </button>
      )}
    </div>
  );
}
