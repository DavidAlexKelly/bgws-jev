// ── realtime/world/tickHook.ts ───────────────────────────────────────────
// The one call `tick` makes into the world layer. Loading this loads every
// domain (world/index.ts), so their systems and handlers are registered
// before the first second runs.

import "./index";
import { WorldCtx, type LandView } from "./context";
import { finishWorld, runSystems } from "./step";
import type { WorldLogItem, WorldState } from "./types";

export function stepWorld(prev: WorldState, view: LandView): { world: WorldState; log: WorldLogItem[] } {
  const c = new WorldCtx(prev, view);
  runSystems(c);
  return { world: finishWorld(c), log: c.newLog };
}
