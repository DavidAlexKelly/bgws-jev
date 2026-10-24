// ── behaviour/store.ts ───────────────────────────────────────────────────
// The user's behaviour templates and which asset uses which, kept in this
// browser's localStorage (docs/plan/DECISION_FLOW_GRAPHS.md §6; sharing
// through Foundry comes later).
//
// Built-in templates (templates/core/*.json) ship with the app and are
// read-only. User templates are always ids "custom/<slug>" and are checked
// with the same validation CI runs on the shipped files.

import { LAND_ACTIONS } from "./actions/land";
import { DEFAULT_TEMPLATE_ID, defaultTemplateForClass } from "./archetypes";
import { LAND_FACTS } from "./facts";
import { buildRegistry, bundledTemplateFiles, type TemplateFiles } from "./load";
import type { Template } from "./types";

export { DEFAULT_TEMPLATE_ID };
const CUSTOM_KEY = "bgws.behaviour.custom.v1";
const ASSIGN_KEY = "bgws.behaviour.assignments.v1";
const SIDE_KEY = "bgws.behaviour.sideAssignments.v1";

/** The kinds of unit a side assignment can name ("Red armour"), by target class. */
export const UNIT_KINDS = ["armoured_vehicle", "foot", "soft_skin"] as const;
export type UnitKind = (typeof UNIT_KINDS)[number];
export const UNIT_KIND_LABEL: Record<UnitKind, string> = {
  armoured_vehicle: "armour",
  foot: "infantry",
  soft_skin: "soft-skinned vehicles",
};
type SideName = "blue" | "red";

function read<T>(key: string, fallback: T): T {
  try {
    const raw = globalThis.localStorage?.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}
function write(key: string, value: unknown): void {
  globalThis.localStorage?.setItem(key, JSON.stringify(value));
}

// ── Change notification (for useSyncExternalStore) ──────────────────────
let version = 0;
const listeners = new Set<() => void>();
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
export const getVersion = () => version;
function changed(): void {
  version++;
  for (const l of listeners) l();
}

// ── Templates ────────────────────────────────────────────────────────────
export const isBuiltIn = (id: string) => !id.startsWith("custom/");

export function customTemplates(): Template[] {
  return read<Template[]>(CUSTOM_KEY, []);
}

/** Every template file: the shipped ones plus the user's, keyed as if on disk. */
export function allTemplateFiles(custom: Template[] = customTemplates()): TemplateFiles {
  const files: TemplateFiles = { ...bundledTemplateFiles() };
  for (const t of custom) files[`./templates/${t.id}.json`] = t;
  return files;
}

/** Validate the whole set as it would be with `custom` saved. Empty = valid. */
export function checkTemplates(custom: Template[] = customTemplates()): string[] {
  try {
    buildRegistry(allTemplateFiles(custom), { actions: new Set(LAND_ACTIONS), facts: new Set<string>(LAND_FACTS) });
    return [];
  } catch (e) {
    const text = e instanceof Error ? e.message : String(e);
    return text
      .split("\n")
      .slice(1)
      .map((l) => l.trim())
      .filter(Boolean);
  }
}

/** All templates by id, shipped first. Invalid user templates are still listed, so they can be fixed. */
export function allTemplates(): Template[] {
  return Object.values(allTemplateFiles()) as Template[];
}

export function slugify(name: string): string {
  return (
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "behaviour"
  );
}

/** A free "custom/<slug>" id for this name. */
export function freeCustomId(name: string, except?: string): string {
  const taken = new Set(allTemplates().map((t) => t.id));
  if (except) taken.delete(except);
  const base = `custom/${slugify(name)}`;
  if (!taken.has(base)) return base;
  for (let i = 2; ; i++) if (!taken.has(`${base}-${i}`)) return `${base}-${i}`;
}

/**
 * Save a user template. Refuses (returning the problems) if the set would
 * not validate, so what is stored is always loadable. `previousId` handles
 * renames: assignments and children follow the new id.
 */
export function saveCustomTemplate(t: Template, previousId?: string): string[] {
  if (isBuiltIn(t.id)) return [`"${t.id}" is a built-in template and cannot be changed`];
  let custom = customTemplates().filter((c) => c.id !== t.id && c.id !== previousId);
  if (previousId && previousId !== t.id)
    custom = custom.map((c) => (c.parent === previousId ? { ...c, parent: t.id } : c));
  custom.push(t);
  custom.sort((a, b) => a.id.localeCompare(b.id));
  const problems = checkTemplates(custom);
  if (problems.length) return problems;
  write(CUSTOM_KEY, custom);
  if (previousId && previousId !== t.id) {
    const a = assignments();
    for (const [asset, tid] of Object.entries(a)) if (tid === previousId) a[asset] = t.id;
    write(ASSIGN_KEY, a);
  }
  changed();
  return [];
}

/** Templates that inherit from this one (directly). */
export const childrenOf = (id: string) => customTemplates().filter((t) => t.parent === id);

/** Delete a user template; assets using it go back to their default. Refuses if others inherit from it. */
export function deleteCustomTemplate(id: string): string[] {
  if (isBuiltIn(id)) return [`"${id}" is a built-in template and cannot be deleted`];
  const kids = childrenOf(id);
  if (kids.length) return [`${kids.map((k) => k.name ?? k.id).join(", ")} build on it — delete or re-base those first`];
  write(CUSTOM_KEY, customTemplates().filter((t) => t.id !== id));
  const a = assignments();
  for (const [asset, tid] of Object.entries(a)) if (tid === id) delete a[asset];
  write(ASSIGN_KEY, a);
  changed();
  return [];
}

// ── Assignments ──────────────────────────────────────────────────────────
function assignments(): Record<string, string> {
  return read<Record<string, string>>(ASSIGN_KEY, {});
}

/** The template an asset uses by default: its unit type's (Armour, Infantry), else core/default. */
export const defaultTemplateFor = (targetClass?: string | null) => defaultTemplateForClass(targetClass);

/** The template explicitly assigned to an asset, if any (and if it still exists). */
export function assignmentFor(assetId: string): string | undefined {
  const assigned = assignments()[assetId];
  return assigned && allTemplates().some((t) => t.id === assigned) ? assigned : undefined;
}

/** The template an asset uses: its assignment, if that template still exists, else its default. */
export function templateFor(assetId: string, targetClass?: string | null): string {
  return assignmentFor(assetId) ?? defaultTemplateFor(targetClass);
}
export const isAssigned = (assetId: string, targetClass?: string | null) =>
  templateFor(assetId, targetClass) !== defaultTemplateFor(targetClass);

/** Assign a template to an asset; null (or its default) clears the assignment. */
export function assignTemplate(assetId: string, templateId: string | null, targetClass?: string | null): void {
  const a = assignments();
  if (!templateId || templateId === defaultTemplateFor(targetClass)) delete a[assetId];
  else a[assetId] = templateId;
  write(ASSIGN_KEY, a);
  changed();
}

// ── Side assignments: "all Red armour uses Stand fast" ───────────────────
function sideAssignments(): Record<SideName, Partial<Record<UnitKind, string>>> {
  const raw = read<Partial<Record<SideName, Partial<Record<UnitKind, string>>>>>(SIDE_KEY, {});
  return { blue: { ...raw.blue }, red: { ...raw.red } };
}

/** The template a side's units of this kind use, if one is assigned (and still exists). */
export function sideAssignmentFor(side: SideName, kind: string | null | undefined): string | undefined {
  if (!kind) return undefined;
  const assigned = sideAssignments()[side][kind as UnitKind];
  return assigned && allTemplates().some((t) => t.id === assigned) ? assigned : undefined;
}

/** Make a side's units of this kind use a template; null clears it. */
export function assignSide(side: SideName, kind: UnitKind, templateId: string | null): void {
  const all = sideAssignments();
  if (templateId) all[side][kind] = templateId;
  else delete all[side][kind];
  write(SIDE_KEY, all);
  changed();
}

/** Which side assignments name this template: "Red armour", … */
export function sideUsesOf(templateId: string): { side: SideName; kind: UnitKind }[] {
  const all = sideAssignments();
  const out: { side: SideName; kind: UnitKind }[] = [];
  for (const side of ["blue", "red"] as const) for (const kind of UNIT_KINDS) if (all[side][kind] === templateId) out.push({ side, kind });
  return out;
}

/**
 * The template a unit uses and why, most specific first
 * (docs/plan/DECISION_FLOW_GRAPHS.md §4.4):
 *   1. the unit's own (a scenario file's — the caller layers that on top),
 *   2. its side's assignment for its kind ("Red armour"),
 *   3. its type's assignment (set on the asset's page),
 *   4. its kind's built-in default.
 */
export function resolveBehaviour(unit: { platform: string; side?: SideName; targetClass?: string | null }): {
  templateId: string;
  why: "side" | "asset" | "default";
} {
  const bySide = unit.side ? sideAssignmentFor(unit.side, unit.targetClass) : undefined;
  if (bySide) return { templateId: bySide, why: "side" };
  const byAsset = assignmentFor(unit.platform);
  if (byAsset) return { templateId: byAsset, why: "asset" };
  return { templateId: defaultTemplateFor(unit.targetClass), why: "default" };
}

/**
 * What a game should run, fixed at its start: each placed unit's template
 * where it is not its kind's default (side assignment, else its type's), why,
 * and a copy of the user templates, so later edits never change a game in
 * progress.
 */
export function behaviourSnapshot(units: readonly { id: string; platform: string; side?: SideName; targetClass?: string | null }[]): {
  behaviours: Record<string, string>;
  behaviourTemplates: Template[];
  /** Why each listed unit has its template. */
  why: Record<string, "side" | "asset">;
} {
  const behaviours: Record<string, string> = {};
  const why: Record<string, "side" | "asset"> = {};
  for (const u of units) {
    const r = resolveBehaviour(u);
    if (r.why === "default") continue;
    behaviours[u.id] = r.templateId;
    why[u.id] = r.why;
  }
  return { behaviours, behaviourTemplates: customTemplates(), why };
}

/** How many assets have this template assigned (beyond defaults). */
export const assignedCount = (templateId: string) =>
  Object.values(assignments()).filter((t) => t === templateId).length;
