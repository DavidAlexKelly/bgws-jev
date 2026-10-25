// ── realtime/world/hooks.ts ──────────────────────────────────────────────
// The registries one domain uses to hear about another's events: impacts,
// hit modifiers, detections. Kept in a module that imports nothing at run
// time, so every domain can register at load, whatever order modules load in.

import type { WeaponSpec } from "../catalogue/schema";
import type { WorldCtx } from "./context";
import type { Entity, WorldTrack } from "./types";
import type { LatLng } from "../../lib/board";
import type { Observer, Target } from "./sensing";

/** Modifies a hit's chance for a target: decoys, a defending aircraft, cages. 1 is no change. */
export type HitModifier = (c: WorldCtx, munition: Entity, weapon: WeaponSpec, targetId: string) => number;
export const HIT_MODIFIERS: HitModifier[] = [];
export function registerHitModifier(m: HitModifier): void {
  HIT_MODIFIERS.push(m);
}

/** Called when a munition reaches or misses its target, for the domains that care (BDA, intercept stats). */
export type ImpactListener = (c: WorldCtx, munition: Entity, outcome: { hit: boolean; killed: boolean; targetId?: string; at: LatLng }) => void;
export const IMPACT_LISTENERS: ImpactListener[] = [];
export function onImpact(listener: ImpactListener): void {
  IMPACT_LISTENERS.push(listener);
}

export type DetectionListener = (c: WorldCtx, o: Observer, t: Target, track: WorldTrack, isNew: boolean) => void;
export const DETECTION_LISTENERS: DetectionListener[] = [];
/** Domains that act on detections (a drone team's targetFound, a destroyer's vampire inbound). */
export function onDetection(listener: DetectionListener): void {
  DETECTION_LISTENERS.push(listener);
}
