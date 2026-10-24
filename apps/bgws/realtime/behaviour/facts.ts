// ── behaviour/facts.ts ───────────────────────────────────────────────────
// The facts a land template's conditions may test. A closed vocabulary: the
// engine supplies a value for each (actions/land.ts), validation rejects any
// other name, and the Behaviour Manager offers exactly these.

export interface FactInfo {
  label: string;
  kind: "number" | "boolean" | "text";
  /** For "text": the values it can take. */
  values?: readonly string[];
  unit?: string;
}

export const LAND_FACT_INFO = {
  "self.class": { label: "This unit is", kind: "text", values: ["armoured_vehicle", "soft_skin", "foot"] },
  "self.strength": { label: "Its combat strength", kind: "number" },
  "self.cohesion": { label: "Its cohesion", kind: "text", values: ["steady", "shaken", "broken"] },
  "self.inCover": { label: "It is in cover", kind: "boolean" },
  "enemy.known": { label: "The enemy is known", kind: "boolean" },
  "enemy.rangeM": { label: "Range to the enemy", kind: "number", unit: "m" },
  "orders.urgent": { label: "Its new orders are urgent", kind: "boolean" },
} as const satisfies Record<string, FactInfo>;

export type LandFact = keyof typeof LAND_FACT_INFO;
export const LAND_FACTS = Object.keys(LAND_FACT_INFO) as LandFact[];
