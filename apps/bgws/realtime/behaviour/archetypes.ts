// ── behaviour/archetypes.ts ──────────────────────────────────────────────
// Which built-in template a unit uses when nothing else is assigned, by what
// it is (its target class). Shared by the engine (land.ts) and the asset
// page (store.ts), so both always agree on a unit's default.

export const DEFAULT_TEMPLATE_ID = "core/default";

const BY_TARGET_CLASS: Record<string, string> = {
  armoured_vehicle: "core/armour",
  foot: "core/infantry",
};

/** The default template for a unit or asset of this target class. */
export function defaultTemplateForClass(targetClass: string | null | undefined): string {
  return (targetClass && BY_TARGET_CLASS[targetClass]) || DEFAULT_TEMPLATE_ID;
}
