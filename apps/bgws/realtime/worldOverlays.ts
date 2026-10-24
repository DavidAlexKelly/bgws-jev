// ── bgws/realtime/worldOverlays.ts ─────────────────────────────────────────
// The world layer on the map (MULTI_DOMAIN_PLAN.md W13 steps 1 and 3), as
// GeoJSON: pure, like overlays.ts, so the screen only hands it to the map.
//
//   entities   aircraft, drones, ships, submarines, satellites' ground
//              tracks, missiles in flight and fixed sites, by domain, each
//              toggleable; in a side's picture, only what that side knows:
//              its own, and the tracks it holds (with their age)
//   areas      jammer footprints, air-defence engagement rings, obstacles,
//              the sea
//   lines      a satellite's coming ground track; each kill chain under way,
//              from shooter to target; aircraft and missiles' paths ahead

import type { LatLng } from "../lib/board";
import type { Side } from "../lib/state";
import { hasRecord, recordFor, weaponSpec } from "./catalogue";
import type { RtState } from "./engine/types";
import { circle, type Collection, type Feature } from "./overlays";
import { satellitePos } from "./world/domains/space";
import type { Entity } from "./world/types";

export type Picture = "truth" | "blue" | "red";
export type WorldLayer = "land" | "air" | "sea" | "space" | "spectrum" | "missiles";
export const WORLD_LAYERS: WorldLayer[] = ["land", "air", "sea", "space", "spectrum", "missiles"];

export interface WorldOverlayView {
  seenBy: Picture;
  layers: ReadonlySet<WorldLayer>;
}

export interface WorldOverlay {
  entities: Collection;
  areas: Collection;
  lines: Collection;
}

const collection = (features: Feature[]): Collection => ({ type: "FeatureCollection", features });
const point = (at: LatLng, properties: Record<string, unknown>): Feature => ({ type: "Feature", properties, geometry: { type: "Point", coordinates: [at.lng, at.lat] } });
const line = (points: LatLng[], properties: Record<string, unknown>): Feature => ({ type: "Feature", properties, geometry: { type: "LineString", coordinates: points.map((p) => [p.lng, p.lat]) } });

function layerOf(e: Pick<Entity, "domain" | "archetype">): WorldLayer {
  if (e.domain === "munition") return "missiles";
  if (e.domain === "space") return "space";
  if (e.domain === "surface" || e.domain === "subsurface") return "sea";
  if (e.domain === "air") return "air";
  return "land";
}

/** A short glyph for the map by kind: what the eye needs to tell apart at a glance. */
function glyph(e: Entity): string {
  if (e.domain === "munition") return "•";
  if (e.domain === "space") return "◇";
  if (e.domain === "subsurface") return "▾";
  if (e.domain === "surface") return "▬";
  if (e.domain === "node") return "■";
  if (e.archetype.includes("fpv") || e.archetype.includes("quad") || e.archetype.includes("loiter") || e.archetype.includes("owa")) return "✕";
  return "✈";
}

export function worldOverlay(state: RtState, view: WorldOverlayView): WorldOverlay {
  const w = state.world;
  if (!w) return { entities: collection([]), areas: collection([]), lines: collection([]) };
  const on = (l: WorldLayer) => view.layers.has(l);
  const entities: Feature[] = [];
  const areas: Feature[] = [];
  const lines: Feature[] = [];
  const side = view.seenBy === "truth" ? null : (view.seenBy as Side);

  // What is drawn: the truth, or a side's own and what it holds tracks of.
  for (const e of Object.values(w.entities)) {
    if (e.status !== "active" && e.status !== "landed") continue;
    const layer = layerOf(e);
    if (!on(layer) || (e.status === "landed" && layer === "air")) continue;
    if (side && e.owner !== side) continue;
    const pos = e.domain === "space" ? satellitePos(e, state.time) : e.pos;
    entities.push(point(pos, { id: e.id, side: e.owner, layer, glyph: glyph(e), label: e.label, altM: Math.round(e.altM), known: "own", age: 0 }));
  }
  if (side) {
    for (const t of Object.values(w.tracks[side])) {
      const layer: WorldLayer = t.domain === "munition" ? "missiles" : t.domain === "air" ? "air" : t.domain === "surface" || t.domain === "subsurface" ? "sea" : "land";
      if (!on(layer) || t.domain === "land") continue;
      entities.push(point(t.at, { id: t.id, side: side === "blue" ? "red" : "blue", layer, glyph: t.domain === "munition" ? "•" : t.domain === "air" ? "✈" : "▬", label: t.thought, altM: Math.round(t.altM), known: t.classification, age: state.time - t.seenAt }));
    }
  }

  // Spectrum: jammer footprints, by band.
  if (on("spectrum")) {
    for (const j of w.jammers) {
      if (!j.on || (side && j.side !== side)) continue;
      for (const band of j.bands) areas.push(circle(j.at, j.radii?.[band as keyof typeof j.radii] ?? j.radiusM, { kind: "jammer", side: j.side, band }));
    }
  }
  // Air defence: each launcher's reach, to its longest-ranged interceptor.
  if (on("air")) {
    for (const [id, comps] of Object.entries(w.land)) {
      const fe = state.game.forceElements[id];
      if (!fe || fe.combatStrength <= 0 || (side && fe.side !== side)) continue;
      const reach = Math.max(0, ...comps.mounts.filter((m) => ["samShort", "samMedium", "samLong"].includes(weaponSpec(m.weaponId).kind)).map((m) => weaponSpec(m.weaponId).maxRangeM));
      if (reach > 0) areas.push(circle(fe.position, reach, { kind: "adRing", side: fe.side, silent: comps.emcon === "silent" }));
    }
  }
  // Obstacles, and the sea.
  if (on("land")) {
    for (const o of w.obstacles) {
      if (side && o.owner !== side && !o.knownTo.includes(side)) continue;
      areas.push(circle(o.at, Math.max(o.widthM, o.depthM) / 2, { kind: o.kind, side: o.owner, lanes: o.lanes.length }));
    }
  }
  if (on("sea") && w.sea) {
    for (const water of w.sea.waters) areas.push({ type: "Feature", properties: { kind: "sea" }, geometry: { type: "Polygon", coordinates: [[...water, water[0]].map((p) => [p.lng, p.lat])] } });
  }

  // Satellites' coming ground tracks: the next twenty minutes.
  if (on("space")) {
    for (const e of Object.values(w.entities)) {
      if (e.domain !== "space" || e.status !== "active" || !hasRecord(e.archetype) || (recordFor(e.archetype).mover.orbitAltKm ?? 0) > 30_000) continue;
      if (side && e.owner !== side) continue;
      lines.push(line(Array.from({ length: 21 }, (_, i) => satellitePos(e, state.time + i * 60)), { kind: "groundTrack", side: e.owner }));
    }
  }
  // Kill chains under way: shooter to target.
  for (const k of w.chains) {
    if (k.engaged == null || k.result != null || !k.shooterId || !k.truthId) continue;
    if (side && k.side !== side) continue;
    const from = state.game.forceElements[k.shooterId]?.position ?? w.entities[k.shooterId]?.pos;
    const to = state.game.forceElements[k.truthId]?.position ?? w.entities[k.truthId]?.pos;
    if (from && to) lines.push(line([from, to], { kind: "killChain", side: k.side, means: k.means ?? "" }));
  }
  // Where aircraft and missiles are heading.
  if (on("air") || on("missiles")) {
    for (const e of Object.values(w.entities)) {
      if (e.status !== "active" || (side && e.owner !== side)) continue;
      const layer = layerOf(e);
      if ((layer !== "air" && layer !== "missiles") || !on(layer)) continue;
      const goal = e.task.kind === "strike" ? (e.task.targetId ? (state.game.forceElements[e.task.targetId]?.position ?? w.entities[e.task.targetId]?.pos) : undefined) ?? e.task.aimAt : e.task.kind === "route" ? e.task.waypoints[0] : undefined;
      if (goal) lines.push(line([e.pos, goal], { kind: "heading", side: e.owner, layer }));
    }
  }
  return { entities: collection(entities), areas: collection(areas), lines: collection(lines) };
}

export const WORLD_SOURCE = { entity: "rt-world-entity", area: "rt-world-area", line: "rt-world-line" } as const;

/** How the world overlay looks: MapLibre layers, bottom to top. */
export function worldLayerSpecs(sideColour: { blue: string; red: string }): Record<string, unknown>[] {
  const bySide = ["match", ["get", "side"], "blue", sideColour.blue, "red", sideColour.red, "#c8ccd6"];
  return [
    { id: `${WORLD_SOURCE.area}-sea`, type: "fill", source: WORLD_SOURCE.area, filter: ["==", ["get", "kind"], "sea"], paint: { "fill-color": "#1d4f7a", "fill-opacity": 0.12 } },
    { id: `${WORLD_SOURCE.area}-fill`, type: "fill", source: WORLD_SOURCE.area, filter: ["!=", ["get", "kind"], "sea"], paint: { "fill-color": ["match", ["get", "kind"], "jammer", "#c084fc", "adRing", bySide, "#e8c547"], "fill-opacity": ["match", ["get", "kind"], "jammer", 0.06, "adRing", 0.04, 0.1] } },
    { id: `${WORLD_SOURCE.area}-edge`, type: "line", source: WORLD_SOURCE.area, filter: ["!=", ["get", "kind"], "sea"], paint: { "line-color": ["match", ["get", "kind"], "jammer", "#c084fc", "adRing", bySide, "#e8c547"], "line-width": 1, "line-opacity": ["case", ["boolean", ["get", "silent"], false], 0.25, 0.6], "line-dasharray": [2, 2] } },
    { id: `${WORLD_SOURCE.line}-track`, type: "line", source: WORLD_SOURCE.line, filter: ["==", ["get", "kind"], "groundTrack"], paint: { "line-color": "#9ad1ff", "line-width": 1, "line-opacity": 0.5, "line-dasharray": [1, 2] } },
    { id: `${WORLD_SOURCE.line}-heading`, type: "line", source: WORLD_SOURCE.line, filter: ["==", ["get", "kind"], "heading"], paint: { "line-color": bySide, "line-width": 1, "line-opacity": 0.4 } },
    { id: `${WORLD_SOURCE.line}-chain`, type: "line", source: WORLD_SOURCE.line, filter: ["==", ["get", "kind"], "killChain"], paint: { "line-color": "#ff5c8a", "line-width": 2, "line-opacity": 0.8, "line-dasharray": [4, 2] } },
    {
      id: `${WORLD_SOURCE.entity}-dot`,
      type: "circle",
      source: WORLD_SOURCE.entity,
      paint: {
        "circle-radius": ["match", ["get", "layer"], "missiles", 2.5, "space", 4, 5],
        "circle-color": bySide,
        "circle-opacity": ["case", ["==", ["get", "known"], "own"], 0.95, ["max", 0.25, ["-", 1, ["/", ["get", "age"], 300]]]],
        "circle-stroke-width": 1,
        "circle-stroke-color": "#0d1017",
      },
    },
  ];
}
