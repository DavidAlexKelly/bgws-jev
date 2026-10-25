// ── realtime/world/imagery.ts ────────────────────────────────────────────
// Satellite imagery a side holds (W9), for its HQs' pictures. Kept apart from
// the space domain so the fusion can read it without loading the domain.

import type { Side } from "../../lib/state";
import { wfig } from "./figures";
import type { WorldState, WorldTrack } from "./types";

/** Detections from imagery that has reached the side and is not yet too old to use. */
export function deliveredImagery(c: { world: Pick<WorldState, "imagery">; time: number }, side: Side): WorldTrack[] {
  const keep = wfig("world/domains/space.IMAGERY_KEEP_S");
  return (c.world.imagery ?? []).filter((b) => b.side === side && b.deliveredAt <= c.time && c.time - b.takenAt < keep).flatMap((b) => b.detections);
}
