// ── realtime/scenario.ts ─────────────────────────────────────────────────
// A real-time game's setup as one versioned JSON file (docs/plan/
// MULTI_DOMAIN_PLAN.md W0 step 3): forces and where they stand, the ground,
// the rules, who commands, and each unit's behaviour — with copies of any
// user templates those behaviours use, so a scenario opened in another
// browser plays as its author set it up.
//
// Not included: display preferences (event-log streaming, speed, viewpoint).
//
// Version 2 adds the SCOPE (scope.ts; plan §5): the scenario type, how long it
// runs, the area, command levels and which domains are simulated. Its
// `durationMin` is the time limit, which version 1 kept in `rules`. A
// version 1 file is read as a Skirmish with its time limit as the duration.

import type { PlacedElement } from "../lib/forceBuilder";
import { vocabulary } from "./behaviour/vocabulary";
import { buildRegistry, bundledTemplateFiles, type TemplateFiles } from "./behaviour/load";
import type { Template } from "./behaviour/types";
import { effectiveScope, scopeErrors, scopeFor, type Scope } from "./scope";

export const SCENARIO_FORMAT = "bgws.realtime.scenario";
export const SCENARIO_VERSION = 2;
export const GROUND_CHOICES = ["generated", "raster+relief", "raster"] as const;
export type ScenarioGround = (typeof GROUND_CHOICES)[number];

export interface RealtimeScenario {
  format: typeof SCENARIO_FORMAT;
  version: typeof SCENARIO_VERSION;
  name: string;
  savedAt: string;
  /** Seeds the dice: the same scenario and seed play out the same way. */
  seed: string;
  ground: { choice: ScenarioGround; seed: string };
  /** What the game covers (scope.ts). `scope.durationMin` is the time limit. */
  scope: Scope;
  rules: { comms: "radio" | "perfect"; breakpointPct: number };
  command: { useCommander: boolean; model: string; useJev: boolean; directives: { blue: string; red: string } };
  forces: PlacedElement[];
  /** Unit id → behaviour template. Units not listed use their type's default. */
  behaviours: Record<string, string>;
  /** The user templates `behaviours` uses, with their parents. */
  behaviourTemplates: Template[];
}

/** The user templates these behaviours need: each one named, and its custom ancestors. */
export function referencedTemplates(behaviours: Record<string, string>, available: readonly Template[]): Template[] {
  const byId = new Map(available.map((t) => [t.id, t]));
  const needed = new Map<string, Template>();
  const visit = (id: string | undefined) => {
    for (let next = id; next && !needed.has(next); ) {
      const t = byId.get(next);
      if (!t) return;
      needed.set(next, t);
      next = t.parent;
    }
  };
  for (const id of Object.values(behaviours)) visit(id);
  return [...needed.values()].sort((a, b) => a.id.localeCompare(b.id));
}

export function scenarioFileName(name: string): string {
  const slug = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "") || "scenario";
  return `${slug}.bgws-scenario.json`;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isNum = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const isStr = (v: unknown): v is string => typeof v === "string";

/** A version 1 file as version 2: a Skirmish, its time limit as the duration. */
function fromVersion1(raw: Record<string, unknown>): Record<string, unknown> {
  const rules = isObj(raw.rules) ? raw.rules : {};
  const { timeLimitMin, ...rest } = rules;
  const scope = scopeFor("skirmish");
  if (isNum(timeLimitMin)) scope.durationMin = timeLimitMin;
  return { ...raw, version: SCENARIO_VERSION, scope, rules: rest };
}

/** Read and check a scenario file. Every problem is listed; `scenario` is set only when there are none. */
export function parseScenario(raw: unknown): { scenario?: RealtimeScenario; errors: string[] } {
  const errors: string[] = [];
  const need = (ok: boolean, msg: string) => void (ok || errors.push(msg));
  if (!isObj(raw)) return { errors: ["Not a scenario file (expected a JSON object)."] };
  if (raw.format !== SCENARIO_FORMAT) return { errors: [`Not a real-time scenario file (format is "${String(raw.format)}").`] };
  if (raw.version === 1) raw = fromVersion1(raw);
  else if (raw.version !== SCENARIO_VERSION)
    return { errors: [`Scenario version ${String(raw.version)} is not supported (this app reads versions 1 and ${SCENARIO_VERSION}).`] };
  if (!isObj(raw)) return { errors: ["Not a scenario file (expected a JSON object)."] };

  need(isStr(raw.name), '"name" must be text');
  need(isStr(raw.seed), '"seed" must be text');
  const ground = isObj(raw.ground) ? raw.ground : {};
  need(GROUND_CHOICES.includes(ground.choice as ScenarioGround), `"ground.choice" must be one of ${GROUND_CHOICES.join(", ")}`);
  need(isStr(ground.seed), '"ground.seed" must be text');
  const rules = isObj(raw.rules) ? raw.rules : {};
  need(rules.comms === "radio" || rules.comms === "perfect", '"rules.comms" must be "radio" or "perfect"');
  need(isNum(rules.breakpointPct) && rules.breakpointPct >= 10 && rules.breakpointPct <= 100, '"rules.breakpointPct" must be 10–100');
  const scopeProblems = scopeErrors(raw.scope);
  errors.push(...scopeProblems);
  if (!scopeProblems.length) {
    // A scope the engine cannot run yet is refused with the reason, not downgraded.
    for (const reason of effectiveScope(raw.scope as Scope).errors) errors.push(`scope: ${reason}`);
  }
  const command = isObj(raw.command) ? raw.command : {};
  const directives = isObj(command.directives) ? command.directives : {};
  need(typeof command.useCommander === "boolean" && typeof command.useJev === "boolean", '"command.useCommander" and "command.useJev" must be true/false');
  need(isStr(command.model), '"command.model" must be text');
  need(isStr(directives.blue) && isStr(directives.red), '"command.directives.blue/red" must be text');

  const forces = Array.isArray(raw.forces) ? raw.forces : [];
  need(Array.isArray(raw.forces), '"forces" must be a list');
  const ids = new Set<string>();
  forces.forEach((f, i) => {
    const at = `forces[${i}]`;
    if (!isObj(f)) return void errors.push(`${at}: must be an object`);
    need(isStr(f.id) && f.id.length > 0, `${at}: "id" is required`);
    if (isStr(f.id)) {
      need(!ids.has(f.id), `${at}: duplicate unit id "${f.id}"`);
      ids.add(f.id);
    }
    need(f.side === "blue" || f.side === "red", `${at}: "side" must be "blue" or "red"`);
    need(isStr(f.platform) && f.platform.length > 0, `${at}: "platform" is required`);
    need(isStr(f.label), `${at}: "label" must be text`);
    need(isNum(f.platformCount) && f.platformCount > 0, `${at}: "platformCount" must be a positive number`);
    need(isStr(f.troopQuality), `${at}: "troopQuality" must be text`);
    const p = isObj(f.position) ? f.position : {};
    need(isNum(p.lat) && isNum(p.lng) && Math.abs(p.lat) <= 90 && Math.abs(p.lng) <= 180, `${at}: "position" must be { lat, lng }`);
  });

  const behaviours = isObj(raw.behaviours) ? (raw.behaviours as Record<string, unknown>) : {};
  need(isObj(raw.behaviours), '"behaviours" must be an object of unit id → template id');
  const templates = Array.isArray(raw.behaviourTemplates) ? (raw.behaviourTemplates as Template[]) : [];
  need(Array.isArray(raw.behaviourTemplates), '"behaviourTemplates" must be a list');
  const files: TemplateFiles = { ...bundledTemplateFiles() };
  for (const t of templates) if (isObj(t) && isStr(t.id)) files[`./templates/${t.id}.json`] = t;
  try {
    buildRegistry(files, vocabulary());
  } catch (e) {
    errors.push(`Behaviour templates in the file are invalid: ${e instanceof Error ? e.message.split("\n").slice(1, 4).join(" ").trim() : String(e)}`);
  }
  const known = new Set(Object.values(files).map((t) => (t as Template).id));
  for (const [unit, tid] of Object.entries(behaviours)) {
    need(ids.has(unit), `behaviours: "${unit}" is not one of the forces`);
    need(isStr(tid) && known.has(tid), `behaviours.${unit}: unknown template "${String(tid)}"`);
  }

  return errors.length ? { errors } : { scenario: raw as unknown as RealtimeScenario, errors };
}
