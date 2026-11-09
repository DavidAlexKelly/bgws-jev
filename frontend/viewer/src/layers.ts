// Everything the game draws, as deck.gl layers, bottom to top: what units
// can see, the air and the nets, enemy tracks, sightings and fire, kill
// chains, then the units themselves.

import type { Layer } from "@deck.gl/core";
import { PathStyleExtension } from "@deck.gl/extensions";
import { BitmapLayer, IconLayer, LineLayer, PathLayer, PolygonLayer, ScatterplotLayer, TextLayer } from "@deck.gl/layers";
import { eventOverlays, SIDE_RGB, type Mark, type Shot } from "./overlays";
import type { FrameEntity, FrameSensor, FrameTrack, FrameView, GameEvent, GameInfo, UnitDetail, Viewshed } from "./protocol";
import { trackIcon, unitIcon } from "./symbols";

export type LayerKey = "labels" | "fire" | "sightings" | "chains" | "tracks" | "sensors" | "spectrum" | "nets";

export const LAYERS: { key: LayerKey; label: string; title: string }[] = [
  { key: "labels", label: "names", title: "Unit names under their symbols" },
  { key: "fire", label: "fire", title: "Shots by outcome, shell impacts, drone strikes, wrecks" },
  { key: "sightings", label: "sightings", title: "Who has just spotted whom" },
  { key: "chains", label: "kill chains", title: "Fire missions and drone strikes in progress: observer, shooter, aim point" },
  { key: "tracks", label: "tracks", title: "Enemy tracks each side holds, with their uncertainty" },
  { key: "sensors", label: "sensors", title: "Each unit's longest-reaching sensor" },
  { key: "spectrum", label: "spectrum", title: "Jammers and emitters on the air (and the spectrum strip)" },
  { key: "nets", label: "nets", title: "Comms links to each HQ, by traffic" },
];

export const DEFAULT_LAYERS: LayerKey[] = ["labels", "fire", "sightings", "chains", "tracks"];

const M_LAT = 111_320;
const mLon = (lat: number) => 111_320 * Math.cos((lat * Math.PI) / 180);

function footprint(e: FrameEntity, s: FrameSensor): [number, number][] {
  const pts: [number, number][] = [];
  const all = s.for_deg >= 359;
  const half = ((s.for_deg / 2) * Math.PI) / 180;
  const h = (s.heading_deg * Math.PI) / 180;
  if (!all) pts.push([e.lon, e.lat]);
  for (let i = 0; i <= 40; i++) {
    const a = all ? (i / 40) * 2 * Math.PI : h - half + (i / 40) * 2 * half;
    pts.push([e.lon + (Math.sin(a) * s.range_m) / mLon(e.lat), e.lat + (Math.cos(a) * s.range_m) / M_LAT]);
  }
  return pts;
}

function ellipse(t: FrameTrack): [number, number][] {
  const pts: [number, number][] = [];
  const a = (t.angle_deg * Math.PI) / 180;
  for (let i = 0; i <= 28; i++) {
    const th = (i / 28) * 2 * Math.PI;
    const x = Math.max(t.sx, 25) * Math.cos(th);
    const y = Math.max(t.sy, 25) * Math.sin(th);
    const e = x * Math.cos(a) + y * Math.sin(a);
    const n = -x * Math.sin(a) + y * Math.cos(a);
    pts.push([t.lon + e / mLon(t.lat), t.lat + n / M_LAT]);
  }
  return pts;
}

/** In 3D, the terrain's height under a point (MapView sets it from MapLibre's
 *  terrain, exaggeration included); flat, none. */
let terrainZ: ((lon: number, lat: number) => number) | null = null;
/** How much the 3D terrain is stretched upright (the theatres are mostly low and flat). */
export const TERRAIN_EXAGGERATION = 1.5;
export function setTerrainZ(f: ((lon: number, lat: number) => number) | null) {
  terrainZ = f;
}
/** A position on the ground: lifted onto the terrain in 3D, a few metres up so it is not buried. */
const at = (lon: number, lat: number): [number, number] | [number, number, number] => (terrainZ ? [lon, lat, terrainZ(lon, lat) + 4] : [lon, lat]);
/** A line on the ground, each point lifted in 3D. */
const onGround = (path: [number, number][]) => (terrainZ ? path.map(([lon, lat]) => at(lon, lat)) : path);

const viewshedImages = new WeakMap<Viewshed, HTMLCanvasElement>();
/** Seen ground in the side's colour, hidden ground within reach darkened, beyond reach clear. */
function viewshedImage(v: Viewshed): HTMLCanvasElement {
  const hit = viewshedImages.get(v);
  if (hit) return hit;
  const c = document.createElement("canvas");
  c.width = v.cols;
  c.height = v.rows;
  const g = c.getContext("2d")!;
  const img = g.createImageData(v.cols, v.rows);
  const [r, gr, b] = SIDE_RGB[v.side] ?? SIDE_RGB.neutral;
  const mid = (v.cols - 1) / 2, reach = v.rangeM / Math.max(v.cellM, 1);
  const tenthsOf = new Uint8Array(v.cols * v.rows);
  for (let r = 0, at = 0; r + 1 < v.runs.length; r += 2) {
    tenthsOf.fill(v.runs[r], at, at + v.runs[r + 1]);
    at += v.runs[r + 1];
  }
  for (let j = 0; j < v.rows; j++)
    for (let i = 0; i < v.cols; i++) {
      // The cells run from the south-west; images from the top-left.
      const k = ((v.rows - 1 - j) * v.cols + i) * 4;
      if (Math.hypot(i - mid, j - mid) > reach) continue;
      // The chance of a clear line, in tenths: none dark, some faint, most bright.
      const tenths = tenthsOf[j * v.cols + i];
      if (tenths >= 1) {
        img.data[k] = r;
        img.data[k + 1] = gr;
        img.data[k + 2] = b;
        img.data[k + 3] = 30 + Math.round((tenths / 9) * 90);
      } else {
        img.data[k + 3] = 125;
      }
    }
  g.putImageData(img, 0, 0);
  viewshedImages.set(v, c);
  return c;
}

const STATUS_RING: Record<string, [number, number, number, number] | undefined> = {
  suppressed: [232, 197, 71, 220],
  pinned: [255, 159, 67, 240],
  broken: [255, 77, 77, 240],
};

export function buildLayers(args: {
  frame: FrameView;
  events: GameEvent[];
  game: GameInfo | null;
  on: Set<LayerKey>;
  selected: string | null;
  unit: UnitDetail | null;
  viewshed: Viewshed | null;
  objective: { lon: number; lat: number } | null;
  /** The pack's towns, named on the map (its basemap carries no fonts). */
  towns: { name: string; lat: number; lon: number; radiusM: number }[];
  onPick: (name: string) => void;
}): Layer[] {
  const { frame: f, on } = args;
  const dash = [new PathStyleExtension({ dash: true })];
  const byId = new Map(f.entities.map((e) => [e.id, e]));
  const out: Layer[] = [];
  const lines = (id: string, data: Shot[], dashed: boolean) =>
    new PathLayer({
      id,
      data: data.filter((s) => s.dashed === dashed),
      getPath: (s: Shot) => onGround([s.from, s.to]),
      getColor: (s: Shot) => s.colour,
      getWidth: (s: Shot) => s.width,
      widthUnits: "pixels",
      ...(dashed && dash.length ? { extensions: dash, getDashArray: [4, 3] } : {}),
    });

  if (args.towns.length) {
    out.push(
      new TextLayer({
        id: "places",
        data: args.towns,
        getPosition: (t: { lon: number; lat: number }) => at(t.lon, t.lat),
        getText: (t: { name: string }) => t.name,
        getSize: (t: { radiusM: number }) => (t.radiusM >= 4000 ? 15 : t.radiusM >= 1500 ? 13 : 11),
        getColor: [199, 204, 219, 200],
        fontFamily: "system-ui, sans-serif",
        fontSettings: { sdf: true },
        outlineWidth: 3,
        outlineColor: [6, 13, 24, 220],
        characterSet: "auto",
      }),
    );
  }
  if (args.viewshed) {
    const v = args.viewshed;
    // Cells of a few metres, blended at their edges: the sight is swept at the finest detail the ground has.
    out.push(new BitmapLayer({ id: "viewshed", image: viewshedImage(v), bounds: [v.west, v.south, v.east, v.north], textureParameters: { minFilter: "linear", magFilter: "linear" } }));
    // Rings round where it stands now: how far it looks, and how far its weapons reach.
    const now = f.entities.find((x) => x.name === v.unit);
    const centre = now ? { lon: now.lon, lat: now.lat } : { lon: v.lon, lat: v.lat };
    const ring = (m: number) => Array.from({ length: 97 }, (_, q) => {
      const a = (q / 96) * 2 * Math.PI;
      return [centre.lon + (m * Math.sin(a)) / mLon(centre.lat), centre.lat + (m * Math.cos(a)) / M_LAT] as [number, number];
    });
    const [r, gr, b] = SIDE_RGB[v.side] ?? SIDE_RGB.neutral;
    type Ring = { path: [number, number][]; color: number[]; dash: number[]; label: string };
    const rings: Ring[] = [{ path: ring(v.rangeM), color: [r, gr, b, 230], dash: [1, 0], label: `sees ${(v.rangeM / 1000).toFixed(1)} km` }];
    if (v.weaponM > 0) rings.push({ path: ring(v.weaponM), color: [255, 214, 102, 230], dash: [6, 4], label: `weapons ${(v.weaponM / 1000).toFixed(1)} km` });
    out.push(
      new PathLayer({
        id: "viewshed-rings",
        data: rings,
        getPath: (d: Ring) => onGround(d.path),
        getColor: (d: Ring) => d.color as [number, number, number, number],
        getWidth: 2,
        widthUnits: "pixels",
        getDashArray: (d: Ring) => d.dash as [number, number],
        dashJustified: true,
        extensions: [new PathStyleExtension({ dash: true })],
      }),
      new TextLayer({
        id: "viewshed-ring-labels",
        data: rings,
        getPosition: (d: Ring) => at(...d.path[0]),
        getText: (d: Ring) => d.label,
        getColor: (d: Ring) => d.color as [number, number, number, number],
        getSize: 12,
        getPixelOffset: [0, -10],
        fontFamily: "ui-monospace, monospace",
        fontSettings: { sdf: true },
        outlineWidth: 3,
        outlineColor: [6, 13, 24, 220],
      }),
    );
  }
  if (on.has("sensors")) {
    const data = f.sensors.filter((s) => byId.has(s.entity)).map((s) => ({ ...s, poly: footprint(byId.get(s.entity)!, s) }));
    out.push(
      new PolygonLayer({
        id: "sensors",
        data,
        getPolygon: (s: { poly: [number, number][] }) => s.poly,
        filled: true,
        stroked: true,
        getFillColor: (s: FrameSensor) => [...SIDE_RGB[s.side], s.active ? 22 : 12],
        getLineColor: (s: FrameSensor) => [...SIDE_RGB[s.side], 110],
        lineWidthMinPixels: 1,
      }),
    );
  }
  if (on.has("spectrum")) {
    out.push(
      new ScatterplotLayer({
        id: "jammers",
        data: f.emitters.filter((x) => x.kind === "jammer"),
        getPosition: (j: { lon: number; lat: number }) => at(j.lon, j.lat),
        getRadius: (j: { radius_m: number }) => j.radius_m,
        radiusUnits: "meters",
        filled: true,
        stroked: true,
        getFillColor: [232, 121, 249, 9],
        getLineColor: [232, 121, 249, 110],
        lineWidthMinPixels: 1,
      }),
      new ScatterplotLayer({
        id: "emitters",
        data: f.emitters.filter((x) => x.kind !== "jammer"),
        getPosition: (x: { lon: number; lat: number }) => at(x.lon, x.lat),
        getRadius: 13,
        radiusUnits: "pixels",
        filled: false,
        stroked: true,
        getLineColor: [232, 121, 249, 200],
        lineWidthMinPixels: 1.5,
      }),
    );
  }
  if (on.has("nets")) {
    const data = f.links.filter((l) => byId.has(l.from) && byId.has(l.to));
    out.push(
      new LineLayer({
        id: "nets",
        data,
        getSourcePosition: (l: { from: number }) => at(byId.get(l.from)!.lon, byId.get(l.from)!.lat),
        getTargetPosition: (l: { to: number }) => at(byId.get(l.to)!.lon, byId.get(l.to)!.lat),
        getColor: (l: { up: boolean; side: "blue" | "red" }) => (l.up ? [...SIDE_RGB[l.side], 130] : [120, 120, 110, 110]),
        getWidth: (l: { traffic: number }) => 1 + Math.min(4, l.traffic),
        widthUnits: "pixels",
      }),
    );
  }
  if (on.has("tracks") && f.tracks.length) {
    out.push(
      new PolygonLayer({
        id: "track-areas",
        data: f.tracks,
        getPolygon: ellipse,
        filled: true,
        stroked: true,
        getFillColor: (t: FrameTrack) => [...SIDE_RGB[t.owner === "blue" ? "red" : "blue"], 26],
        getLineColor: (t: FrameTrack) => [...SIDE_RGB[t.owner === "blue" ? "red" : "blue"], 150],
        lineWidthMinPixels: 1,
        pickable: true,
      }),
      new IconLayer({
        id: "track-icons",
        data: f.tracks,
        getPosition: (t: FrameTrack) => at(t.lon, t.lat),
        getIcon: (t: FrameTrack) => trackIcon(t.owner, t.class),
        getSize: 28,
        pickable: true,
      }),
    );
  }
  const ov = eventOverlays(f, args.events, args.game?.origin ?? null);
  if (on.has("sightings")) out.push(lines("spots", ov.spots, true));
  if (on.has("fire")) {
    out.push(
      new ScatterplotLayer({
        id: "wrecks",
        data: ov.wrecks,
        getPosition: (m: Mark) => at(m.at[0], m.at[1]),
        getRadius: (m: Mark) => m.radius,
        radiusUnits: "pixels",
        getFillColor: (m: Mark) => m.colour,
        filled: true,
        stroked: false,
        opacity: 0.35,
      }),
      lines("shots-miss", ov.shots, true),
      lines("shots", ov.shots, false),
      new ScatterplotLayer({
        id: "marks",
        data: ov.marks,
        getPosition: (m: Mark) => at(m.at[0], m.at[1]),
        getRadius: (m: Mark) => m.radius,
        radiusUnits: "pixels",
        filled: true,
        stroked: true,
        getFillColor: (m: Mark) => (m.filled ? [m.colour[0], m.colour[1], m.colour[2], Math.round(m.colour[3] * 0.35)] : [0, 0, 0, 0]),
        getLineColor: (m: Mark) => m.colour,
        lineWidthMinPixels: 2,
      }),
    );
  }
  if (on.has("chains") && f.chains.length) {
    const chains = f.chains.filter((c) => byId.has(c.shooter));
    out.push(
      new LineLayer({
        id: "chain-cue",
        data: chains.filter((c) => c.sensor != null && byId.has(c.sensor)),
        getSourcePosition: (c: { sensor: number }) => at(byId.get(c.sensor)!.lon, byId.get(c.sensor)!.lat),
        getTargetPosition: (c: { shooter: number }) => at(byId.get(c.shooter)!.lon, byId.get(c.shooter)!.lat),
        getColor: [196, 181, 253, 150],
        getWidth: 1,
        widthUnits: "pixels",
      }),
      new LineLayer({
        id: "chain-fire",
        data: chains,
        pickable: true,
        getSourcePosition: (c: { shooter: number }) => at(byId.get(c.shooter)!.lon, byId.get(c.shooter)!.lat),
        getTargetPosition: (c: { aim_lon: number; aim_lat: number }) => at(c.aim_lon, c.aim_lat),
        getColor: (c: { side: "blue" | "red" }) => [...SIDE_RGB[c.side], 220],
        getWidth: 2.5,
        widthUnits: "pixels",
      }),
      new ScatterplotLayer({
        id: "chain-aim",
        data: chains,
        getPosition: (c: { aim_lon: number; aim_lat: number }) => at(c.aim_lon, c.aim_lat),
        getRadius: 9,
        radiusUnits: "pixels",
        filled: false,
        stroked: true,
        getLineColor: (c: { side: "blue" | "red" }) => [...SIDE_RGB[c.side], 230],
        lineWidthMinPixels: 2,
      }),
    );
  }
  // The selected unit: where it is going, and a ring.
  const sel = args.selected ? f.entities.find((e) => e.name === args.selected) : undefined;
  const route = args.unit?.name === args.selected ? args.unit?.activity?.route ?? [] : [];
  if (sel && route.length) {
    out.push(
      new PathLayer({
        id: "route",
        data: [{ path: [[sel.lon, sel.lat], ...route] }],
        getPath: (r: { path: [number, number][] }) => onGround(r.path),
        getColor: [255, 255, 255, 200],
        getWidth: 2,
        widthUnits: "pixels",
        ...(dash.length ? { extensions: dash, getDashArray: [6, 4] } : {}),
      }),
      new ScatterplotLayer({ id: "route-end", data: [route[route.length - 1]], getPosition: (p: [number, number]) => p, getRadius: 5, radiusUnits: "pixels", getFillColor: [255, 255, 255, 220] }),
    );
  }
  if (args.objective) {
    out.push(
      new ScatterplotLayer({
        id: "objective",
        data: [args.objective],
        getPosition: (o: { lon: number; lat: number }) => at(o.lon, o.lat),
        getRadius: 12,
        radiusUnits: "pixels",
        filled: true,
        stroked: true,
        getFillColor: [232, 197, 71, 40],
        getLineColor: [232, 197, 71, 240],
        lineWidthMinPixels: 3,
      }),
    );
  }
  const ground = f.entities.filter((e) => e.domain !== "drone");
  const drones = f.entities.filter((e) => e.domain === "drone");
  // Each unit's elements where they stand (its formation: a column on the
  // road, a wedge across country, a line when halted), under its symbol.
  const elements: { at: [number, number]; side: string }[] = [];
  for (const e of ground) {
    if (e.status === "destroyed" || !e.elements) continue;
    for (const [de, dn] of e.elements) {
      if (de === 0 && dn === 0) continue;
      elements.push({ at: [e.lon + de / mLon(e.lat), e.lat + dn / M_LAT], side: e.side });
    }
  }
  out.push(
    new ScatterplotLayer({
      id: "elements",
      data: elements,
      getPosition: (x: { at: [number, number] }) => at(x.at[0], x.at[1]),
      getRadius: 3.5,
      radiusUnits: "pixels",
      stroked: true,
      getFillColor: (x: { side: string }) => [...(SIDE_RGB[x.side as keyof typeof SIDE_RGB] ?? SIDE_RGB.neutral), 230] as [number, number, number, number],
      getLineColor: [6, 13, 24, 220],
      lineWidthMinPixels: 1,
    }),
  );
  out.push(
    new ScatterplotLayer({
      id: "status",
      data: ground.filter((e) => STATUS_RING[e.status]),
      getPosition: (e: FrameEntity) => at(e.lon, e.lat),
      getRadius: 21,
      radiusUnits: "pixels",
      filled: false,
      stroked: true,
      getLineColor: (e: FrameEntity) => STATUS_RING[e.status]!,
      lineWidthMinPixels: 2,
    }),
    new ScatterplotLayer({
      id: "selected",
      data: sel ? [sel] : [],
      getPosition: (e: FrameEntity) => at(e.lon, e.lat),
      getRadius: 25,
      radiusUnits: "pixels",
      filled: true,
      stroked: true,
      getFillColor: [255, 255, 255, 25],
      getLineColor: [255, 255, 255, 230],
      lineWidthMinPixels: 2,
    }),
    new IconLayer({
      id: "drones",
      data: drones,
      // Flying: at its own height (exaggerated like the terrain), never below the ground.
      getPosition: (e: FrameEntity) => (terrainZ ? [e.lon, e.lat, Math.max(terrainZ(e.lon, e.lat) + 4, e.alt * TERRAIN_EXAGGERATION)] : [e.lon, e.lat]),
      getIcon: unitIcon,
      getSize: 20,
      pickable: true,
    }),
    new IconLayer({
      id: "units",
      data: ground,
      getPosition: (e: FrameEntity) => at(e.lon, e.lat),
      getIcon: unitIcon,
      getSize: 34,
      pickable: true,
      onClick: (info: { object?: FrameEntity }) => {
        if (info.object) args.onPick(info.object.name);
        return true;
      },
      updateTriggers: { getIcon: ground.map((e) => e.status).join() },
    }),
  );
  if (on.has("labels")) {
    out.push(
      new TextLayer({
        id: "labels",
        data: ground,
        getPosition: (e: FrameEntity) => at(e.lon, e.lat),
        getText: (e: FrameEntity) => e.label || e.name,
        getSize: 11,
        getPixelOffset: [0, 30],
        getColor: (e: FrameEntity) => (e.status === "destroyed" ? [140, 143, 153, 200] : [...SIDE_RGB[e.side], 255]),
        fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
        fontSettings: { sdf: true },
        outlineWidth: 3,
        outlineColor: [6, 13, 24, 230],
        characterSet: "auto",
      }),
    );
  }
  return out;
}

/** Hover text for anything on the map. */
export function tooltip(info: { object?: unknown; layer?: { id: string } | null }): string | null {
  const o = info.object as Record<string, unknown> | undefined;
  if (!o || !info.layer) return null;
  switch (info.layer.id) {
    case "units":
    case "drones": {
      const e = o as unknown as FrameEntity;
      return `${e.label || e.name}\n${e.platform} · ${e.activity}\n${e.fit}/${e.total} fit · ${e.status}${e.speed > 0.5 ? ` · ${(e.speed * 3.6).toFixed(0)} km/h` : ""}`;
    }
    case "track-areas":
    case "track-icons": {
      const t = o as unknown as FrameTrack;
      return `${t.owner} track ${t.id}: ${t.class} (${Math.round(t.p * 100)}%)\n±${Math.round(t.sx)} m · ${Math.round(t.age_s)} s old`;
    }
    case "chain-fire":
      return `${o.kind}: ${o.stage}`;
  }
  return null;
}
