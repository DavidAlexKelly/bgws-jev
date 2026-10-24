// ── realtime/scope.ts ────────────────────────────────────────────────────
// A scenario's SCOPE (docs/plan/MULTI_DOMAIN_PLAN.md §5, W0 step 6): how long
// it runs, how big it is, which command levels exist, and — the part the
// engine wires — which domains are simulated, abstracted or off.
//
// A scope is chosen first, from a scenario type (§5.3), then adjusted. It is
// saved in the scenario file and carried into the engine's config.
//
// HONEST ABOUT WHAT EXISTS. The plan lists eight scenario types and eleven
// domains; today the engine simulates land combat and nothing else.
// `ENGINE_CAN` says exactly what is built. `effectiveScope` turns any scope
// into one the engine can run — an abstracted domain that is not built yet
// is switched off, with a note saying so — and a scope that needs a domain
// SIMULATED that is not built is refused rather than quietly downgraded.
// As workstreams land (M2 drones, …), `ENGINE_CAN` grows and more types
// become playable without touching this logic.

export const DOMAINS = [
  "land",
  "drones",
  "fires",
  "ew",
  "air",
  "airDefence",
  "naval",
  "space",
  "cyber",
  "logistics",
  "medical",
] as const;
export type Domain = (typeof DOMAINS)[number];

export const DOMAIN_LABEL: Record<Domain, string> = {
  land: "Land",
  drones: "Drones",
  fires: "Fires (artillery)",
  ew: "Electronic warfare",
  air: "Air",
  airDefence: "Air defence",
  naval: "Naval",
  space: "Space",
  cyber: "Cyber",
  logistics: "Logistics",
  medical: "Medical",
};

export const DOMAIN_MODES = ["simulated", "abstracted", "off"] as const;
export type DomainMode = (typeof DOMAIN_MODES)[number];

export const ECHELONS = ["platoon", "company", "battalion", "brigade", "division", "corps", "component", "joint"] as const;
export type Echelon = (typeof ECHELONS)[number];

export const PLANNERS = ["overall", "tactical", "land", "air", "maritime", "spaceCyber", "sustainment"] as const;
export type Planner = (typeof PLANNERS)[number];

export const SCENARIO_TYPES = [
  "skirmish",
  "combinedArms",
  "droneLine",
  "airCampaign",
  "sustainedOffensive",
  "maritime",
  "strike",
  "joint",
  "custom",
] as const;
export type ScenarioType = (typeof SCENARIO_TYPES)[number];

export interface Scope {
  /** The preset it started from; "custom" once it is edited beyond what the type allows. */
  type: ScenarioType;
  /** How long it may run, in game minutes: the time limit. */
  durationMin: number;
  /** The operating area. */
  area: { widthKm: number; depthKm: number };
  /** The highest and lowest command levels that exist. */
  echelons: { highest: Echelon; lowest: Echelon };
  domains: Record<Domain, DomainMode>;
  /** Which Claude planners exist, per side. */
  planners: Planner[];
  /** Aggregation away from contact, and which units Jev leads. */
  detail: { aggregateAwayFromContact: boolean; jev: "all" | "nearContact" };
}

const domains = (simulated: Domain[], abstracted: Domain[] = []): Record<Domain, DomainMode> =>
  Object.fromEntries(DOMAINS.map((d) => [d, simulated.includes(d) ? "simulated" : abstracted.includes(d) ? "abstracted" : "off"])) as Record<
    Domain,
    DomainMode
  >;

export interface ScenarioTypeInfo {
  name: string;
  /** One line for the chooser. */
  blurb: string;
  /** When it becomes playable in the plan (§7). */
  milestone: string;
  /** How the plan says it is won by default (§5.3). */
  victory: string;
  /** Its default scope, as the plan sets it (§5.3). */
  defaults: Omit<Scope, "type">;
}

/** The plan's eight presets (§5.3), plus Custom. Durations are each type's shortest default. */
export const SCENARIO_TYPE_INFO: Record<ScenarioType, ScenarioTypeInfo> = {
  skirmish: {
    name: "Skirmish",
    blurb: "Platoons and companies, a few kilometres, up to two hours.",
    milestone: "M1",
    victory: "Breakpoint or objective",
    defaults: {
      durationMin: 90,
      area: { widthKm: 10, depthKm: 10 },
      echelons: { highest: "company", lowest: "platoon" },
      domains: domains(["land"], ["drones", "fires"]),
      planners: ["tactical"],
      detail: { aggregateAwayFromContact: false, jev: "all" },
    },
  },
  combinedArms: {
    name: "Combined-arms battle",
    blurb: "Company to brigade, with drones, EW and fires simulated.",
    milestone: "M3",
    victory: "Objectives by time",
    defaults: {
      durationMin: 120,
      area: { widthKm: 30, depthKm: 30 },
      echelons: { highest: "brigade", lowest: "company" },
      domains: domains(["land", "drones", "ew", "fires", "logistics"], ["air", "space"]),
      planners: ["overall", "land"],
      detail: { aggregateAwayFromContact: false, jev: "all" },
    },
  },
  droneLine: {
    name: "Drone line",
    blurb: "A positional front: drones, EW, artillery and infantry over days.",
    milestone: "M4",
    victory: "Objectives by time",
    defaults: {
      durationMin: 24 * 60,
      area: { widthKm: 30, depthKm: 15 },
      echelons: { highest: "brigade", lowest: "platoon" },
      domains: domains(["land", "drones", "ew", "fires", "logistics", "medical"], ["air", "space"]),
      planners: ["overall", "land"],
      detail: { aggregateAwayFromContact: false, jev: "all" },
    },
  },
  airCampaign: {
    name: "Air campaign",
    blurb: "Flights, air defence and missiles over hundreds of kilometres.",
    milestone: "M5",
    victory: "Air superiority, or a target set destroyed",
    defaults: {
      durationMin: 24 * 60,
      area: { widthKm: 300, depthKm: 300 },
      echelons: { highest: "component", lowest: "platoon" },
      domains: domains(["air", "airDefence", "ew", "space"], ["land", "logistics"]),
      planners: ["overall", "air", "land"],
      detail: { aggregateAwayFromContact: false, jev: "nearContact" },
    },
  },
  sustainedOffensive: {
    name: "Sustained offensive",
    blurb: "Battalion to division over days, with logistics and medical.",
    milestone: "M6",
    victory: "Objectives by time",
    defaults: {
      durationMin: 3 * 24 * 60,
      area: { widthKm: 100, depthKm: 100 },
      echelons: { highest: "division", lowest: "battalion" },
      domains: domains(["land", "drones", "ew", "airDefence", "logistics", "medical"], ["air", "space", "cyber"]),
      planners: ["overall", "land", "sustainment"],
      detail: { aggregateAwayFromContact: true, jev: "nearContact" },
    },
  },
  maritime: {
    name: "Maritime / littoral",
    blurb: "Ships, coastal batteries and drones at sea, with neutral shipping.",
    milestone: "M7",
    victory: "Sea control, or ships and cargo through",
    defaults: {
      durationMin: 24 * 60,
      area: { widthKm: 500, depthKm: 500 },
      echelons: { highest: "component", lowest: "platoon" },
      domains: domains(["naval", "air", "airDefence", "drones", "ew", "space", "logistics"], ["land"]),
      planners: ["overall", "maritime", "air"],
      detail: { aggregateAwayFromContact: false, jev: "nearContact" },
    },
  },
  strike: {
    name: "Strike and missile defence",
    blurb: "Missiles, air defence and space across a theatre.",
    milestone: "M8",
    victory: "Operational aims",
    defaults: {
      durationMin: 24 * 60,
      area: { widthKm: 1000, depthKm: 1000 },
      echelons: { highest: "joint", lowest: "platoon" },
      domains: domains(["air", "airDefence", "space", "ew", "cyber", "drones"], ["land", "logistics"]),
      planners: ["overall", "air", "spaceCyber"],
      detail: { aggregateAwayFromContact: true, jev: "nearContact" },
    },
  },
  joint: {
    name: "Joint operation",
    blurb: "Every domain, platoon to joint HQ, over weeks.",
    milestone: "M8",
    victory: "The overall aims",
    defaults: {
      durationMin: 7 * 24 * 60,
      area: { widthKm: 400, depthKm: 300 },
      echelons: { highest: "joint", lowest: "platoon" },
      domains: domains([...DOMAINS]),
      planners: ["overall", "land", "air", "maritime", "spaceCyber", "sustainment"],
      detail: { aggregateAwayFromContact: true, jev: "nearContact" },
    },
  },
  custom: {
    name: "Custom",
    blurb: "Start from the skirmish and set each part yourself.",
    milestone: "—",
    victory: "Breakpoint or objective",
    defaults: {
      durationMin: 90,
      area: { widthKm: 10, depthKm: 10 },
      echelons: { highest: "company", lowest: "platoon" },
      domains: domains(["land"]),
      planners: ["tactical"],
      detail: { aggregateAwayFromContact: false, jev: "all" },
    },
  },
};

/** The scope a scenario type starts with. */
export function scopeFor(type: ScenarioType): Scope {
  const d = SCENARIO_TYPE_INFO[type].defaults;
  return {
    type,
    durationMin: d.durationMin,
    area: { ...d.area },
    echelons: { ...d.echelons },
    domains: { ...d.domains },
    planners: [...d.planners],
    detail: { ...d.detail },
  };
}

/**
 * What the engine can actually do today. Each domain lists the modes that
 * are built; `limits` are what today's board, rules and command allow.
 */
/** Which modes each domain has built, as the workstreams land (§7). */
const BUILT: Record<Domain, readonly DomainMode[]> = {
  land: ["simulated"],
  fires: ["simulated", "abstracted", "off"],
  drones: ["simulated", "abstracted", "off"],
  ew: ["simulated", "abstracted", "off"],
  air: ["simulated", "abstracted", "off"],
  airDefence: ["simulated", "off"],
  naval: ["off"],
  space: ["abstracted", "off"],
  cyber: ["off"],
  logistics: ["abstracted", "off"],
  medical: ["off"],
};

export const ENGINE_CAN = {
  domains: BUILT,
  /** The milestone that first builds each domain (§7), for the notes. */
  builtIn: {
    land: "now",
    drones: "M2",
    fires: "M3",
    ew: "M2",
    air: "M5",
    airDefence: "M5",
    naval: "M7",
    space: "M5",
    cyber: "M8",
    logistics: "M6",
    medical: "M4",
  } satisfies Record<Domain, string>,
  limits: {
    durationMin: { min: 5, max: 24 * 60 },
    /** Ground is generated, so the board can be as large as an air campaign needs. */
    areaKm: { max: 300 },
    echelons: ["platoon", "company", "battalion", "brigade", "component"] as readonly Echelon[],
    planners: ["tactical", "overall", "land", "air"] as readonly Planner[],
  },
};

export interface ScopeCheck {
  /** The scope the engine will run: unbuilt abstracted domains off, the rest within today's limits. */
  scope: Scope;
  /** What was changed to make it runnable, in words, for the setup screen. */
  notes: string[];
  /** Why it cannot run at all. Empty if it can. */
  errors: string[];
}

/** Turn a scope into one the engine can run, saying what changed — or why it cannot. */
export function effectiveScope(scope: Scope): ScopeCheck {
  const notes: string[] = [];
  const errors: string[] = [];
  const out: Scope = { ...scope, domains: { ...scope.domains }, area: { ...scope.area }, planners: [...scope.planners] };
  for (const d of DOMAINS) {
    const want = scope.domains[d];
    if (ENGINE_CAN.domains[d].includes(want)) continue;
    const when = ENGINE_CAN.builtIn[d];
    if (want === "simulated") {
      errors.push(`${DOMAIN_LABEL[d]} cannot be simulated yet (planned for ${when}).`);
    } else if (want === "abstracted") {
      out.domains[d] = "off";
      notes.push(`${DOMAIN_LABEL[d]}: abstracted support comes with ${when}; off for now.`);
    } else {
      errors.push(`${DOMAIN_LABEL[d]} cannot be off: the engine has nothing to play without it.`);
    }
  }
  const { durationMin, areaKm, echelons, planners } = ENGINE_CAN.limits;
  if (scope.durationMin < durationMin.min || scope.durationMin > durationMin.max) {
    errors.push(`A game can run ${durationMin.min}–${durationMin.max} minutes today (asked for ${scope.durationMin}).`);
  }
  if (scope.area.widthKm > areaKm.max || scope.area.depthKm > areaKm.max) {
    out.area = { widthKm: Math.min(scope.area.widthKm, areaKm.max), depthKm: Math.min(scope.area.depthKm, areaKm.max) };
    notes.push(`The board is ${areaKm.max} × ${areaKm.max} km today; the larger area is not used.`);
  }
  if (!echelons.includes(scope.echelons.highest) || !echelons.includes(scope.echelons.lowest)) {
    errors.push(`Only ${echelons.join(" and ")} levels exist today (asked for ${scope.echelons.lowest} → ${scope.echelons.highest}).`);
  }
  const unbuilt = scope.planners.filter((p) => !planners.includes(p));
  if (unbuilt.length) {
    out.planners = out.planners.filter((p) => planners.includes(p));
    notes.push(`Only the tactical planner exists today; ${unbuilt.join(", ")} come later.`);
  }
  if (scope.detail.aggregateAwayFromContact) errors.push("Aggregating units away from contact is not built yet.");
  return { scope: out, notes, errors };
}

/** Can a game of this type be played today? */
export function playable(type: ScenarioType): boolean {
  return effectiveScope(scopeFor(type)).errors.length === 0;
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);

/** Check a scope read from a file. Every problem is listed. */
export function scopeErrors(raw: unknown): string[] {
  const errors: string[] = [];
  if (!isObj(raw)) return ['"scope" must be an object'];
  if (!SCENARIO_TYPES.includes(raw.type as ScenarioType)) errors.push(`"scope.type" must be one of ${SCENARIO_TYPES.join(", ")}`);
  if (typeof raw.durationMin !== "number" || !(raw.durationMin > 0)) errors.push('"scope.durationMin" must be a positive number');
  const area = isObj(raw.area) ? raw.area : {};
  if (!(typeof area.widthKm === "number" && area.widthKm > 0 && typeof area.depthKm === "number" && area.depthKm > 0)) {
    errors.push('"scope.area" must be { widthKm, depthKm }, both positive');
  }
  const ech = isObj(raw.echelons) ? raw.echelons : {};
  if (!ECHELONS.includes(ech.highest as Echelon) || !ECHELONS.includes(ech.lowest as Echelon)) {
    errors.push(`"scope.echelons.highest/lowest" must be one of ${ECHELONS.join(", ")}`);
  }
  const dom = isObj(raw.domains) ? raw.domains : {};
  for (const d of DOMAINS) {
    if (!DOMAIN_MODES.includes(dom[d] as DomainMode)) errors.push(`"scope.domains.${d}" must be one of ${DOMAIN_MODES.join(", ")}`);
  }
  if (!Array.isArray(raw.planners) || raw.planners.some((p) => !PLANNERS.includes(p as Planner))) {
    errors.push(`"scope.planners" must be a list of ${PLANNERS.join(", ")}`);
  }
  const det = isObj(raw.detail) ? raw.detail : {};
  if (typeof det.aggregateAwayFromContact !== "boolean" || (det.jev !== "all" && det.jev !== "nearContact")) {
    errors.push('"scope.detail" must be { aggregateAwayFromContact: true/false, jev: "all" | "nearContact" }');
  }
  return errors;
}
