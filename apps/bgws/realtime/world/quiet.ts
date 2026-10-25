// ── realtime/world/quiet.ts ──────────────────────────────────────────────
// Quiet-period skipping, the scope's time dimension (MULTI_DOMAIN_PLAN.md
// §5.1): a game of days is mostly waiting — convoys on the road, units in
// position. While nothing is happening the screen may run the clock faster,
// up to the scope's maxCompression; the moment anything happens it drops
// back. Only the clock's speed changes, never the simulation, so a skipped
// game and an unskipped one with the same seed end the same.

import type { RtState } from "../engine/types";

/** A shot within this long, or a sighting this fresh, means it is not quiet. */
export const QUIET_AFTER_S = 120;

/** Is nothing happening: no fighting, no fresh contacts, nothing in flight, no question waiting? */
export function quietPeriod(state: RtState): boolean {
  const now = state.time;
  for (const [id, u] of Object.entries(state.units)) {
    if ((state.game.forceElements[id]?.combatStrength ?? 0) <= 0) continue;
    if (u.engagement || now - u.lastShotAt < QUIET_AFTER_S) return false;
    for (const seen of Object.values(u.ownSeen)) if (now - seen.time < QUIET_AFTER_S) return false;
  }
  const w = state.world;
  if (w) {
    if (w.questions.some((q) => !q.answer)) return false;
    for (const e of Object.values(w.entities)) if (e.status === "active" && e.domain === "munition") return false;
  }
  return true;
}

/** How fast the clock runs now: the chosen speed, or the scope's fastest while quiet and skipping is on. */
export function clockSpeed(state: RtState | null | undefined, chosen: number, time: { maxCompression: number; skipQuiet: boolean }): number {
  const capped = Math.min(chosen, time.maxCompression);
  return time.skipQuiet && state && quietPeriod(state) ? Math.max(capped, time.maxCompression) : capped;
}
