// What the feed puts on the map, as the TS screen draws it (realtime/
// overlays.ts): shots by outcome, shell impacts, who spotted whom, drone
// strikes and wrecks. Each fades over a span of simulated time.

import type { FrameView, GameEvent, Side } from "./protocol";

export type Rgba = [number, number, number, number];

export interface Shot {
  from: [number, number];
  to: [number, number];
  colour: Rgba;
  width: number;
  dashed: boolean;
}

export interface Mark {
  at: [number, number];
  colour: Rgba;
  radius: number;
  filled: boolean;
  text?: string;
}

const SHOT_S = 40;
const SPOT_S = 25;
const IMPACT_S = 60;

const OUTCOME: Record<string, [number, number, number]> = {
  "fire.kill": [255, 77, 77],
  "fire.hit": [255, 159, 67],
  "fire.miss": [215, 219, 230],
};

/** Metres east and north of the origin to lon/lat (the theatre is small enough to treat as flat). */
export function enuToLonLat(origin: { lat: number; lon: number }, e: number, n: number): [number, number] {
  const mLat = 111_320;
  const mLon = 111_320 * Math.cos((origin.lat * Math.PI) / 180);
  return [origin.lon + e / mLon, origin.lat + n / mLat];
}

const fade = (age: number, life: number) => Math.max(0, 1 - age / life);

export function eventOverlays(frame: FrameView, events: GameEvent[], origin: { lat: number; lon: number } | null) {
  const now = frame.t;
  const at = new Map<string, [number, number]>();
  for (const e of frame.entities) at.set(e.name, [e.lon, e.lat]);
  const shots: Shot[] = [];
  const spots: Shot[] = [];
  const marks: Mark[] = [];
  const wrecks: Mark[] = [];
  const seenWreck = new Set<string>();
  // Newest last; walk back only as far as the longest life.
  for (let i = events.length - 1; i >= 0; i--) {
    const ev = events[i];
    const age = now - ev.t;
    if (age < 0) continue;
    if (ev.kind === "unit.destroyed" && ev.entity && !seenWreck.has(ev.entity)) {
      seenWreck.add(ev.entity);
      const p = at.get(ev.entity);
      if (p) wrecks.push({ at: p, colour: age < 60 ? [140, 143, 153, 200] : [90, 93, 100, 160], radius: age < 60 ? 22 : 10, filled: true });
      continue;
    }
    if (age > IMPACT_S) {
      if (age > 600) break;
      continue;
    }
    const from = ev.entity ? at.get(ev.entity) : undefined;
    const to = ev.target ? at.get(ev.target) : undefined;
    const rgb = OUTCOME[ev.kind];
    if (rgb && from && to && age <= SHOT_S) {
      const a = Math.round(230 * fade(age, SHOT_S));
      shots.push({ from, to, colour: [...rgb, a], width: ev.kind === "fire.miss" ? 1 : 2, dashed: ev.kind === "fire.miss" });
      if (ev.kind !== "fire.miss") marks.push({ at: to, colour: [...rgb, a], radius: ev.kind === "fire.kill" ? 9 : 7, filled: ev.kind === "fire.kill" });
    } else if (ev.kind === "sense.detect" && from && to && age <= SPOT_S) {
      spots.push({ from, to, colour: [232, 197, 71, Math.round(200 * fade(age, SPOT_S))], width: 1, dashed: true });
    } else if (ev.kind === "drones.strike" && to && age <= SHOT_S) {
      const hit = String(ev.data?.stage ?? "") !== "missed";
      marks.push({ at: to, colour: hit ? [255, 77, 77, Math.round(230 * fade(age, SHOT_S))] : [215, 219, 230, Math.round(160 * fade(age, SHOT_S))], radius: 12, filled: false });
    } else if (ev.kind === "fires.impact" && origin && ev.data && ev.data.centreE != null) {
      const p = enuToLonLat(origin, ev.data.centreE, ev.data.centreN);
      const kills = Number(ev.data.kills ?? 0);
      marks.push({ at: p, colour: kills > 0 ? [255, 77, 77, Math.round(200 * fade(age, IMPACT_S))] : [255, 159, 67, Math.round(160 * fade(age, IMPACT_S))], radius: 14, filled: true });
    }
  }
  return { shots, spots, marks, wrecks };
}

/** What kind of line a feed entry is, for its colour and the feed's filters. */
export type Category = "decision" | "fire" | "sense" | "c2" | "drones" | "ew" | "people" | "logistics" | "world";

export function category(kind: string): Category {
  const head = kind.split(".")[0];
  if (head === "decide") return "decision";
  if (head === "fire" || head === "fires" || kind === "unit.destroyed") return "fire";
  if (head === "sense") return "sense";
  if (head === "c2" || head === "comms") return "c2";
  if (head === "drones") return "drones";
  if (head === "ew" || head === "pnt") return "ew";
  if (head === "people" || head === "medical") return "people";
  if (head === "logistics" || head === "land") return "logistics";
  return "world";
}

export const CATEGORY_COLOUR: Record<Category, string> = {
  decision: "#b8bdd0",
  fire: "#ff9f43",
  sense: "#e8c547",
  c2: "#c4b5fd",
  drones: "#5fd4c0",
  ew: "#e879f9",
  people: "#ff9e8f",
  logistics: "#8a91a8",
  world: "#8a91a8",
};

export const CATEGORY_LABEL: Record<Category, string> = {
  decision: "decisions",
  fire: "fire",
  sense: "sightings",
  c2: "command",
  drones: "drones",
  ew: "EW & PNT",
  people: "people",
  logistics: "ground & supply",
  world: "game",
};

export const SIDE_RGB: Record<Side, [number, number, number]> = { blue: [143, 194, 255], red: [255, 158, 143], neutral: [180, 180, 170] };
export const SIDE_COLOUR: Record<Side, string> = { blue: "#8fc2ff", red: "#ff9e8f", neutral: "#b4b4aa" };
