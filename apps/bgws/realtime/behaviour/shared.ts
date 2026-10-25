// ── behaviour/shared.ts ──────────────────────────────────────────────────
// Templates shared across a team through Foundry (DECISION_FLOW_GRAPHS.md §6,
// step 9): a `BgwsBehaviourTemplate` object per template — id, name,
// version, parent and the parent's version, archetype, the JSON body and its
// author — created and edited through Ontology actions.
//
// This module is the storage logic, with the transport handed in: a
// `TemplateRemote` lists the objects and applies the actions. The Foundry
// remote (data/templateStore.ts) speaks the Ontology REST API; tests use an
// in-memory one. Nothing here touches the network or localStorage.

import type { Template } from "./types";

/** One template as its Foundry object holds it. */
export interface SharedTemplate {
  templateId: string;
  name: string;
  version: number;
  parentId?: string;
  parentVersion?: number;
  archetype?: string;
  body: string;
  author?: string;
  updatedAt?: string;
}

export interface TemplateRemote {
  list(): Promise<SharedTemplate[]>;
  create(t: SharedTemplate): Promise<void>;
  edit(t: SharedTemplate): Promise<void>;
  remove(templateId: string): Promise<void>;
}

/** A user template as its shared object: the template itself is the body. */
export function toShared(t: Template, all: readonly Template[], author?: string): SharedTemplate {
  const parent = t.parent ? all.find((p) => p.id === t.parent) : undefined;
  return {
    templateId: t.id,
    name: t.name ?? t.id,
    version: t.version,
    ...(t.parent ? { parentId: t.parent } : {}),
    ...(parent ? { parentVersion: parent.version } : {}),
    ...(t.archetype ? { archetype: t.archetype } : {}),
    body: JSON.stringify(t),
    ...(author ? { author } : {}),
  };
}

/** Back from the shared object; refuses one whose body is not its template. */
export function fromShared(s: SharedTemplate): { template?: Template; problem?: string } {
  try {
    const t = JSON.parse(s.body) as Template;
    if (t.id !== s.templateId) return { problem: `${s.templateId}: the body is for "${t.id}"` };
    return { template: t };
  } catch {
    return { problem: `${s.templateId}: the body is not valid JSON` };
  }
}

export interface PublishResult {
  created: string[];
  edited: string[];
  /** Someone else's newer version is there: not overwritten. */
  conflicts: { id: string; mine: number; theirs: number; by?: string }[];
}

/**
 * Publish these templates: new ones are created, newer versions of shared
 * ones replace them, and a template whose shared copy is newer than this one
 * is left alone and reported (someone else changed it: pull first).
 */
export async function publish(remote: TemplateRemote, templates: readonly Template[], all: readonly Template[], author?: string): Promise<PublishResult> {
  const shared = new Map((await remote.list()).map((s) => [s.templateId, s]));
  const out: PublishResult = { created: [], edited: [], conflicts: [] };
  for (const t of templates) {
    const there = shared.get(t.id);
    const mine = toShared(t, all, author);
    if (!there) {
      await remote.create(mine);
      out.created.push(t.id);
    } else if (there.version > t.version) {
      out.conflicts.push({ id: t.id, mine: t.version, theirs: there.version, ...(there.author ? { by: there.author } : {}) });
    } else if (there.version < t.version || there.body !== mine.body) {
      await remote.edit({ ...mine, version: Math.max(t.version, there.version + (there.body !== mine.body && there.version === t.version ? 1 : 0)) });
      out.edited.push(t.id);
    }
  }
  return out;
}

export interface PullResult {
  /** The templates to keep locally: the newer of each, by version. */
  templates: Template[];
  added: string[];
  updated: string[];
  problems: string[];
  /** A shared template built on a version of its parent that is no longer the one shared. */
  staleParents: string[];
}

/** Pull the team's templates and merge them with these local ones: the higher version wins. */
export async function pull(remote: TemplateRemote, local: readonly Template[]): Promise<PullResult> {
  const byId = new Map(local.map((t) => [t.id, t]));
  const out: PullResult = { templates: [], added: [], updated: [], problems: [], staleParents: [] };
  const shared = await remote.list();
  const versions = new Map(shared.map((s) => [s.templateId, s.version]));
  for (const s of shared) {
    const { template, problem } = fromShared(s);
    if (!template) {
      out.problems.push(problem!);
      continue;
    }
    if (!template.id.startsWith("custom/")) {
      out.problems.push(`${template.id}: only custom templates are shared`);
      continue;
    }
    if (s.parentId && s.parentVersion != null && versions.has(s.parentId) && versions.get(s.parentId) !== s.parentVersion) out.staleParents.push(template.id);
    const mine = byId.get(template.id);
    if (!mine) {
      byId.set(template.id, template);
      out.added.push(template.id);
    } else if (template.version > mine.version) {
      byId.set(template.id, template);
      out.updated.push(template.id);
    }
  }
  out.templates = [...byId.values()].sort((a, b) => a.id.localeCompare(b.id));
  return out;
}

/** An in-memory remote, for tests and for working offline. */
export function memoryRemote(seed: SharedTemplate[] = []): TemplateRemote & { objects: Map<string, SharedTemplate> } {
  const objects = new Map(seed.map((s) => [s.templateId, s]));
  return {
    objects,
    list: async () => [...objects.values()].map((s) => ({ ...s })),
    create: async (t) => {
      if (objects.has(t.templateId)) throw new Error(`${t.templateId} exists`);
      objects.set(t.templateId, { ...t });
    },
    edit: async (t) => {
      if (!objects.has(t.templateId)) throw new Error(`${t.templateId} does not exist`);
      objects.set(t.templateId, { ...t });
    },
    remove: async (id) => void objects.delete(id),
  };
}
