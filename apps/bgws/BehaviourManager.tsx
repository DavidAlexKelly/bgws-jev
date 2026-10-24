/**
 * BehaviourManager — browse, create, copy, edit and delete behaviour templates.
 *
 * A behaviour template decides what a unit is offered at each decision point
 * (realtime/behaviour, docs/plan/DECISION_FLOW_GRAPHS.md). Built-in templates
 * ship with the app and are read-only; yours are variants: "based on" another
 * template, with options switched off, preferences nudged, and doctrine
 * text. That is exactly the template format's parent / remove / bias /
 * doctrine, so everything saved here passes the same validation CI runs.
 */

import { useMemo, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { resolveTemplate } from "./realtime/behaviour/walk";
import {
  allTemplates,
  assignedCount,
  checkTemplates,
  childrenOf,
  customTemplates,
  deleteCustomTemplate,
  freeCustomId,
  isBuiltIn,
  saveCustomTemplate,
  DEFAULT_TEMPLATE_ID,
  assignSide,
  sideAssignmentFor,
  UNIT_KIND_LABEL,
  UNIT_KINDS,
} from "./realtime/behaviour/store";
import { useBehaviours } from "./realtime/behaviour/useBehaviours";
import type { Bias, Comparison, Condition, FactValue, ResolvedTemplate, Template } from "./realtime/behaviour/types";
import { LAND_FACT_INFO, LAND_FACTS, type FactInfo, type LandFact } from "./realtime/behaviour/facts";
import "./behaviour.css";
import BehaviourBench from "./BehaviourBench";

const BIAS_LABEL: Record<Bias, string> = {
  [-2]: "−2 strongly discouraged",
  [-1]: "−1 discouraged",
  0: "0 neutral",
  1: "+1 preferred",
  2: "+2 strongly preferred",
};
const BIASES: Bias[] = [-2, -1, 0, 1, 2];

/** "land.pullBackToFriends" → "Pull back to friends". */
function actionLabel(action: string): string {
  const words = action.replace(/^land\./, "").replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
  return words.charAt(0).toUpperCase() + words.slice(1);
}
const displayName = (t: Template) => t.name ?? t.id;

const OP_WORDS: Record<Comparison, string> = { "==": "is", "!=": "is not", ">": ">", ">=": "≥", "<": "<", "<=": "≤" };
const factInfo = (fact: string): FactInfo | undefined => (LAND_FACT_INFO as Record<string, FactInfo>)[fact];

/** A condition in words: "Range to the enemy ≤ 1000 m". */
function describeCondition(c: Condition): string {
  if ("all" in c) return c.all.map(describeCondition).join(" and ");
  if ("any" in c) return c.any.map(describeCondition).join(" or ");
  if ("not" in c) return `not (${describeCondition(c.not)})`;
  const info = factInfo(c.fact);
  const value = typeof c.value === "boolean" ? (c.value ? "yes" : "no") : `${c.value}${info?.unit ? ` ${info.unit}` : ""}`;
  return `${info?.label ?? c.fact} ${OP_WORDS[c.op]} ${value}`;
}

/** Edit one "only when…" comparison; anything more complex is shown, not edited. */
function ConditionEditor({ value, onChange }: { value: Condition | undefined; onChange: (c: Condition | undefined) => void }) {
  if (!value) {
    return (
      <button className="bm-cond-add" onClick={() => onChange({ fact: "enemy.rangeM", op: "<=", value: 1000 })}>
        + Only when…
      </button>
    );
  }
  if (!("fact" in value)) {
    return (
      <span className="bm-cond">
        Only when {describeCondition(value)}
        <button className="bm-cond-x" aria-label="Remove condition" onClick={() => onChange(undefined)}>×</button>
      </span>
    );
  }
  const info = factInfo(value.fact) ?? LAND_FACT_INFO["enemy.rangeM"];
  const ops: Comparison[] = info.kind === "number" ? ["<", "<=", ">", ">=", "==", "!="] : ["==", "!="];
  const setFact = (fact: LandFact) => {
    const next = LAND_FACT_INFO[fact] as FactInfo;
    const v: FactValue = next.kind === "number" ? 1000 : next.kind === "boolean" ? true : (next.values?.[0] ?? "");
    onChange({ fact, op: next.kind === "number" ? "<=" : "==", value: v });
  };
  return (
    <span className="bm-cond">
      Only when
      <select className="bm-select bm-cond-field" aria-label="Fact" value={value.fact} onChange={(e) => setFact(e.target.value as LandFact)}>
        {LAND_FACTS.map((f) => <option key={f} value={f}>{LAND_FACT_INFO[f].label}</option>)}
      </select>
      <select className="bm-select bm-cond-field" aria-label="Comparison" value={value.op} onChange={(e) => onChange({ ...value, op: e.target.value as Comparison })}>
        {ops.map((op) => <option key={op} value={op}>{OP_WORDS[op]}</option>)}
      </select>
      {info.kind === "number" ? (
        <input className="bm-input bm-cond-field" type="number" aria-label="Value" value={Number(value.value ?? 0)} onChange={(e) => onChange({ ...value, value: Number(e.target.value) })} />
      ) : info.kind === "boolean" ? (
        <select className="bm-select bm-cond-field" aria-label="Value" value={String(value.value)} onChange={(e) => onChange({ ...value, value: e.target.value === "true" })}>
          <option value="true">yes</option>
          <option value="false">no</option>
        </select>
      ) : (
        <select className="bm-select bm-cond-field" aria-label="Value" value={String(value.value)} onChange={(e) => onChange({ ...value, value: e.target.value })}>
          {(info.values ?? []).map((v) => <option key={v} value={v}>{v}</option>)}
        </select>
      )}
      {info.unit && <span>{info.unit}</span>}
      <button className="bm-cond-x" aria-label="Remove condition" onClick={() => onChange(undefined)}>×</button>
    </span>
  );
}

/** Resolve without throwing (a broken user template must still be viewable). */
function resolveSafe(id: string, templates: Template[]): ResolvedTemplate | null {
  try {
    return resolveTemplate(id, new Map(templates.map((t) => [t.id, t])));
  } catch {
    return null;
  }
}

export default function BehaviourManager() {
  useBehaviours();
  const [params, setParams] = useSearchParams();
  const templates = allTemplates();
  const selectedId = params.get("id") ?? DEFAULT_TEMPLATE_ID;
  const selected = templates.find((t) => t.id === selectedId) ?? templates[0];
  const select = (id: string) => setParams({ id }, { replace: true });
  const bench = params.get("view") === "bench";
  const showBench = (on: boolean) => setParams(on ? { id: selectedId, view: "bench" } : { id: selectedId }, { replace: true });

  const createFrom = (base: Template, copyOf?: Template) => {
    const name = copyOf ? `${displayName(copyOf)} (copy)` : `New behaviour based on ${displayName(base)}`;
    const t: Template = copyOf
      ? { ...copyOf, id: freeCustomId(name), name, version: 1 }
      : { id: freeCustomId(name), version: 1, name, parent: base.id };
    const problems = saveCustomTemplate(t);
    if (problems.length) window.alert(problems.join("\n"));
    else select(t.id);
  };

  const builtIn = templates.filter((t) => isBuiltIn(t.id));
  const mine = templates.filter((t) => !isBuiltIn(t.id));

  return (
    <div className="bm-page">
      <header className="bm-top">
        <Link className="bm-back" to="/bgws/assets">‹ Asset Manager</Link>
        <div>
          <h1 className="bm-title">Behaviour Manager</h1>
          <p className="bm-sub">What units are offered at each decision, and how Jev is nudged to choose.</p>
        </div>
        <div className="bm-tabs" role="tablist" aria-label="View">
          <button className="bm-btn" role="tab" aria-selected={!bench} data-variant={bench ? undefined : "primary"} onClick={() => showBench(false)}>
            Templates
          </button>
          <button className="bm-btn" role="tab" aria-selected={bench} data-variant={bench ? "primary" : undefined} onClick={() => showBench(true)}>
            Test bench
          </button>
        </div>
      </header>
      <div className="bm-body">
        <aside className="bm-list" aria-label="Behaviour templates">
          <button className="bm-btn" data-variant="primary" data-size="block" onClick={() => createFrom(selected ?? builtIn[0])}>
            + New behaviour
          </button>
          <div className="bm-list-head">Built-in</div>
          {builtIn.map((t) => (
            <ListItem key={t.id} t={t} current={t.id === selected?.id} onClick={() => select(t.id)} />
          ))}
          <div className="bm-list-head">Yours</div>
          {mine.length === 0 && <div className="bm-item-meta" style={{ padding: "0 4px" }}>None yet — copy a built-in one to start.</div>}
          {mine.map((t) => (
            <ListItem key={t.id} t={t} current={t.id === selected?.id} onClick={() => select(t.id)} />
          ))}
        </aside>
        <main className="bm-detail">
          {bench ? (
            <BehaviourBench initialTemplate={selected?.id ?? DEFAULT_TEMPLATE_ID} />
          ) : selected ? (
            <TemplateDetail
              key={selected.id}
              template={selected}
              templates={templates}
              onCopy={() => createFrom(selected, isBuiltIn(selected.id) ? undefined : selected)}
              onSelect={select}
            />
          ) : (
            <div className="bm-empty">No templates.</div>
          )}
        </main>
      </div>
    </div>
  );
}

function ListItem({ t, current, onClick }: { t: Template; current: boolean; onClick: () => void }) {
  const used = assignedCount(t.id);
  return (
    <button className="bm-item" aria-current={current} onClick={onClick}>
      <span className="bm-item-name">{displayName(t)}</span>
      <span className="bm-item-meta">
        {t.parent ? `based on ${t.parent}` : t.id}
        {used ? ` · ${used} asset${used === 1 ? "" : "s"}` : ""}
      </span>
    </button>
  );
}

function TemplateDetail({
  template,
  templates,
  onCopy,
  onSelect,
}: {
  template: Template;
  templates: Template[];
  onCopy: () => void;
  onSelect: (id: string) => void;
}) {
  const readOnly = isBuiltIn(template.id);
  const [draft, setDraft] = useState<Template>(template);
  const [saveProblems, setSaveProblems] = useState<string[]>([]);
  const dirty = JSON.stringify(draft) !== JSON.stringify(template);

  // Live validation of the whole set as it would be with this draft saved.
  const problems = useMemo(() => {
    if (readOnly) return checkTemplates();
    const custom = customTemplates().filter((c) => c.id !== template.id);
    return checkTemplates([...custom, { ...draft, id: template.id }]);
  }, [draft, readOnly, template.id]);

  const draftSet = readOnly ? templates : [...templates.filter((t) => t.id !== template.id), draft];
  const resolved = resolveSafe(template.id, draftSet);
  const inherited = draft.parent ? resolveSafe(draft.parent, draftSet) : null;
  const ownRemove = new Set(draft.remove ?? []);
  const used = assignedCount(template.id);
  const kids = readOnly ? [] : childrenOf(template.id);

  const toggle = (optionId: string, offered: boolean) => {
    const remove = new Set(draft.remove ?? []);
    if (offered) remove.delete(optionId);
    else remove.add(optionId);
    setDraft({ ...draft, remove: remove.size ? [...remove].sort() : undefined });
  };
  const setBias = (optionId: string, value: string) => {
    const bias = { ...(draft.bias ?? {}) };
    if (value === "") delete bias[optionId];
    else bias[optionId] = Number(value) as Bias;
    setDraft({ ...draft, bias: Object.keys(bias).length ? bias : undefined });
  };
  const setWhen = (optionId: string, cond: Condition | undefined) => {
    const when = { ...(draft.when ?? {}) };
    if (cond) when[optionId] = cond;
    else delete when[optionId];
    setDraft({ ...draft, when: Object.keys(when).length ? when : undefined });
  };
  const save = () => {
    const name = (draft.name ?? "").trim() || displayName(template);
    const next = { ...draft, name, version: (template.version ?? 1) + 1 };
    const p = saveCustomTemplate(next);
    setSaveProblems(p);
  };
  const remove = () => {
    if (!window.confirm(`Delete "${displayName(template)}"?${used ? ` ${used} asset(s) using it go back to their default.` : ""}`)) return;
    const p = deleteCustomTemplate(template.id);
    if (p.length) setSaveProblems(p);
    else onSelect(DEFAULT_TEMPLATE_ID);
  };

  const triggers = resolved ? Object.entries(resolved.triggers) : [];

  return (
    <>
      <div className="bm-head">
        <div className="bm-head-text">
          <h2 className="bm-name">{displayName(readOnly ? template : draft)}</h2>
          <div className="bm-id">{template.id}</div>
          <div className="bm-badges">
            <span className="bm-badge" data-tone={readOnly ? "builtin" : "custom"}>{readOnly ? "Built-in · read-only" : "Yours"}</span>
            {template.parent && <span className="bm-badge">Based on {template.parent}</span>}
            <span className="bm-badge">{used ? `Assigned to ${used} asset${used === 1 ? "" : "s"}` : "Not assigned to any asset"}</span>
          </div>
        </div>
        <div className="bm-actions">
          <button className="bm-btn" onClick={onCopy}>{readOnly ? "Make a variant" : "Duplicate"}</button>
          {!readOnly && (
            <>
              <button className="bm-btn" disabled={!dirty} onClick={() => { setDraft(template); setSaveProblems([]); }}>Revert</button>
              <button className="bm-btn" data-variant="primary" disabled={!dirty || problems.length > 0} onClick={save}>Save</button>
              <button className="bm-btn" data-variant="danger" disabled={kids.length > 0} title={kids.length ? "Other behaviours build on this one" : undefined} onClick={remove}>Delete</button>
            </>
          )}
        </div>
      </div>

      {!readOnly && problems.length > 0 && (
        <div className="bm-callout" data-tone="danger" role="alert">
          <strong>Can&apos;t save yet:</strong>
          <ul>{problems.map((p) => <li key={p}>{p}</li>)}</ul>
        </div>
      )}
      {saveProblems.length > 0 && (
        <div className="bm-callout" data-tone="danger" role="alert"><ul>{saveProblems.map((p) => <li key={p}>{p}</li>)}</ul></div>
      )}
      {dirty && problems.length === 0 && <div className="bm-callout" data-tone="warn">Unsaved changes.</div>}

      {!readOnly && (
        <section className="bm-card">
          <h3 className="bm-card-title">About</h3>
          <div className="bm-field">
            <label htmlFor="bm-name">Name</label>
            <input id="bm-name" className="bm-input" value={draft.name ?? ""} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
          </div>
          <div className="bm-field">
            <label htmlFor="bm-parent">Based on</label>
            <select id="bm-parent" className="bm-select" value={draft.parent ?? ""} onChange={(e) => setDraft({ ...draft, parent: e.target.value || undefined })}>
              {templates.filter((t) => t.id !== template.id).map((t) => (
                <option key={t.id} value={t.id}>{displayName(t)} ({t.id})</option>
              ))}
            </select>
          </div>
          <div className="bm-field">
            <label htmlFor="bm-doctrine">Doctrine (told to Jev at every decision)</label>
            <textarea id="bm-doctrine" className="bm-textarea" placeholder="e.g. This crew holds its ground: it never gives up a position it was ordered to hold." value={draft.doctrine ?? ""} onChange={(e) => setDraft({ ...draft, doctrine: e.target.value || undefined })} />
          </div>
        </section>
      )}
      {readOnly && template.doctrine && (
        <section className="bm-card">
          <h3 className="bm-card-title">Doctrine</h3>
          <div>{template.doctrine}</div>
        </section>
      )}

      <SideAssignment template={template} templates={templates} disabled={dirty} />

      {!resolved && <div className="bm-callout" data-tone="danger">This template can&apos;t be resolved — check what it&apos;s based on.</div>}
      {!readOnly && (
        <div className="bm-callout">
          Options are switched off <strong>by name</strong>: unticking <code>pullBack</code> stops this unit pulling back at every
          decision that offers it. Each decision&apos;s fallback stays on, so a unit always has something to do.
        </div>
      )}
      {triggers.map(([trigger, nodeId]) => {
        const node = resolved?.nodes[nodeId];
        if (!node || node.kind !== "choose") return null;
        return (
          <section className="bm-card" key={trigger}>
            <h3 className="bm-card-title">{trigger} · {node.title}</h3>
            <p className="bm-card-sub">{node.question}</p>
            <div className="bm-options">
              {node.options.map((o) => {
                const inheritedOff = inherited?.remove.has(o.id) ?? false;
                const isFallback = o.id === node.fallback;
                const offered = readOnly ? !resolved?.remove.has(o.id) : !inheritedOff && !ownRemove.has(o.id);
                const lock = readOnly || inheritedOff || isFallback;
                const ownBias = draft.bias?.[o.id];
                const effectiveBias = (resolved?.bias[o.id] ?? o.bias ?? 0) as Bias;
                return (
                  <div className="bm-option" key={o.id} data-off={!offered}>
                    <input
                      type="checkbox"
                      aria-label={`Offer ${actionLabel(o.action)}`}
                      checked={offered}
                      disabled={lock}
                      onChange={(e) => toggle(o.id, e.target.checked)}
                    />
                    <span>
                      <span className="bm-option-name">{actionLabel(o.action)}</span>
                      <span className="bm-option-note">
                        {o.id}
                        {isFallback ? " · fallback when Jev can't answer — always offered" : ""}
                        {inheritedOff ? ` · switched off by ${draft.parent}` : ""}
                      </span>
                      {inherited?.when[o.id] && !draft.when?.[o.id] && (
                        <span className="bm-option-note">Only when {describeCondition(inherited.when[o.id])} (from {draft.parent})</span>
                      )}
                      {readOnly && resolved?.when[o.id] && (
                        <span className="bm-option-note">Only when {describeCondition(resolved.when[o.id])}</span>
                      )}
                      {!readOnly && offered && !isFallback && (
                        <ConditionEditor value={draft.when?.[o.id]} onChange={(c) => setWhen(o.id, c)} />
                      )}
                    </span>
                    {readOnly ? (
                      <span className="bm-option-note">{BIAS_LABEL[effectiveBias]}</span>
                    ) : (
                      <select
                        className="bm-select bm-bias"
                        aria-label={`Preference for ${actionLabel(o.action)}`}
                        value={ownBias === undefined ? "" : String(ownBias)}
                        disabled={!offered}
                        onChange={(e) => setBias(o.id, e.target.value)}
                      >
                        <option value="">Inherit ({BIAS_LABEL[(inherited?.bias[o.id] ?? o.bias ?? 0) as Bias].split(" ")[0]})</option>
                        {BIASES.map((b) => <option key={b} value={b}>{BIAS_LABEL[b]}</option>)}
                      </select>
                    )}
                  </div>
                );
              })}
            </div>
          </section>
        );
      })}
    </>
  );
}

/**
 * "All Red armour uses this": side assignments, which beat a unit type's
 * assignment and lose only to a scenario file's own (DECISION_FLOW_GRAPHS.md §4.4).
 */
function SideAssignment({ template, templates, disabled }: { template: Template; templates: Template[]; disabled: boolean }) {
  const nameOf = (id: string) => templates.find((t) => t.id === id)?.name ?? id;
  return (
    <section className="bm-card">
      <h3 className="bm-card-title">Use for a whole side</h3>
      <p className="bm-card-sub">
        Every unit of that kind on that side uses this behaviour in new games, whatever its type is set to. A scenario
        file&apos;s own choices still win.
      </p>
      <div className="bm-side-grid">
        {(["blue", "red"] as const).map((side) =>
          UNIT_KINDS.map((kind) => {
            const current = sideAssignmentFor(side, kind);
            const mine = current === template.id;
            return (
              <label key={`${side}-${kind}`} className="bm-side-cell" data-side={side}>
                <input
                  type="checkbox"
                  checked={mine}
                  disabled={disabled}
                  onChange={(e) => assignSide(side, kind, e.target.checked ? template.id : null)}
                />
                <span>
                  {side === "blue" ? "Blue" : "Red"} {UNIT_KIND_LABEL[kind]}
                  {current && !mine && <span className="bm-option-note">now: {nameOf(current)}</span>}
                </span>
              </label>
            );
          }),
        )}
      </div>
      {disabled && <p className="bm-option-note">Save your changes first.</p>}
    </section>
  );
}
