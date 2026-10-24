// ── realtime/world/pnt.ts ────────────────────────────────────────────────
// Position, navigation and timing (W6, W9): each side's GNSS service, as a
// quality at a place. Counterspace lowers it everywhere; a GNSS jammer
// lowers it in its footprint — for BOTH sides, because jamming is
// indiscriminate [ref R2]. GNSS-guided munitions lose accuracy with it
// (Excalibur's >50% falling below 10% [ref R3, P15] is a calibration target),
// and units' own position error grows, which spoils their calls for fire.

import { distanceM, type LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import { wfig } from "./figures";
import type { WorldState } from "./types";

/** 0–1: how good GNSS is here for this side. */
export function pntQualityAt(world: Pick<WorldState, "pnt" | "jammers">, side: Side, at: LatLng): number {
  let q = world.pnt[side]?.global ?? 1;
  for (const j of world.jammers) {
    if (!j.on || !j.bands.includes("gnss")) continue;
    const d = distanceM(j.at, at);
    if (d >= j.radiusM) continue;
    // Deepest at the jammer, shallowest at the edge of its footprint.
    const depth = 1 - d / j.radiusM;
    q *= 1 - wfig("world/pnt.JAM_DEPTH") * Math.sqrt(depth) - (1 - wfig("world/pnt.JAM_DEPTH")) * 0.5 * depth;
  }
  return Math.max(0, Math.min(1, q));
}

/**
 * A GNSS-guided round's error at this PNT quality: its CEP grows from the
 * guided figure towards an inertial-only drift as quality falls.
 */
export function guidedCepM(cepM: number, quality: number, inertialCepM = wfig("world/pnt.INERTIAL_CEP_M")): number {
  return cepM + (inertialCepM - cepM) * (1 - quality) ** 1.5;
}
