// ── behaviour/vocabulary.ts ──────────────────────────────────────────────
// Every action and fact a template may name: the land vocabulary (the
// decision points D0–D12) and the world vocabulary (the domains' decision
// points, actions/world.ts). One closed list, so a template naming anything
// else fails validation wherever it is loaded.

import { LAND_ACTIONS } from "./actions/land";
import { WORLD_ACTIONS, WORLD_FACTS } from "./actions/world";
import { LAND_FACTS } from "./facts";
import type { Vocabulary } from "./schema";

export const ALL_ACTIONS: readonly string[] = [...LAND_ACTIONS, ...WORLD_ACTIONS];
export const ALL_FACTS: readonly string[] = [...LAND_FACTS, ...WORLD_FACTS];

export function vocabulary(): Vocabulary {
  return { actions: new Set(ALL_ACTIONS), facts: new Set(ALL_FACTS) };
}
