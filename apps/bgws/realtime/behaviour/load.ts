// ── behaviour/load.ts ────────────────────────────────────────────────────
// Turn the JSON files under ./templates into a validated registry.
//
// Files live at templates/<kind>/<name>.json (kind = core | custom) and a
// template's "id" must equal "<kind>/<name>", so the id in a trace always
// tells you which file to open.

import { validateRegistry, validateTemplate, type Vocabulary } from "./schema";
import type { Template } from "./types";

/** path → parsed JSON, the shape import.meta.glob(..., { eager, import: "default" }) returns. */
export type TemplateFiles = Record<string, unknown>;

/** "./templates/core/tank.json" → "core/tank". */
export function templateIdFromPath(path: string): string | null {
  const m = /(?:^|\/)templates\/(.+)\.json$/.exec(path);
  return m ? m[1] : null;
}

/**
 * Validate every file and build the registry. Throws once, listing every
 * problem in every file, rather than stopping at the first.
 */
export function buildRegistry(files: TemplateFiles, vocab: Vocabulary = {}): Map<string, Template> {
  const errors: string[] = [];
  const registry = new Map<string, Template>();
  for (const [path, raw] of Object.entries(files).sort(([a], [b]) => a.localeCompare(b))) {
    const expected = templateIdFromPath(path);
    const problems = validateTemplate(raw, vocab);
    if (problems.length) {
      errors.push(...problems.map((p) => `${path}: ${p}`));
      continue;
    }
    const t = raw as Template;
    if (expected && t.id !== expected)
      errors.push(`${path}: "id" is "${t.id}" but the file's path says "${expected}"`);
    else if (registry.has(t.id)) errors.push(`${path}: duplicate template id "${t.id}"`);
    else registry.set(t.id, t);
  }
  if (!errors.length) errors.push(...validateRegistry(registry));
  if (errors.length)
    throw new Error(`Behaviour templates are invalid:\n  ${errors.join("\n  ")}`);
  return registry;
}

/** Every template file in this folder, as bundled by Vite. */
export function bundledTemplateFiles(): TemplateFiles {
  return import.meta.glob("./templates/**/*.json", { eager: true, import: "default" });
}
