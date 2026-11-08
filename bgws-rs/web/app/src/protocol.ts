// What bgws-server sends and takes over /ws (apps/server/src/lib.rs). The
// frame types are generated from Rust (web/types, ts-rs); the rest are the
// server's own JSON messages.

import type { FrameEntity } from "../../types/FrameEntity";
import type { FrameView } from "../../types/FrameView";
import type { Question } from "../../types/Question";
import type { Side } from "../../types/Side";
import type { Viewer } from "../../types/Viewer";

export type { FrameEntity, FrameView, Question, Side, Viewer };
export type { FrameChain } from "../../types/FrameChain";
export type { FrameEmitter } from "../../types/FrameEmitter";
export type { FrameLink } from "../../types/FrameLink";
export type { FrameSensor } from "../../types/FrameSensor";
export type { FrameTrack } from "../../types/FrameTrack";

/** One line of the game's event log, with names for ids. */
export interface GameEvent {
  clock: string;
  t: number;
  kind: string;
  text: string;
  side: Side | null;
  entity: string | null;
  target: string | null;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  data: any;
}

export interface SideDeciders {
  units: string;
  hqs: string;
  keyMissing: boolean;
  person: boolean;
  directive: string;
}

/** The game as the server describes it on `hello` and after a new one is opened. */
/** A theatre pack (`bgws terrain`), as the server describes it. */
export interface PackInfo {
  name: string;
  bbox: [number, number, number, number] | null;
  cellM: number;
  /** URLs under /packs/, when the pack lives in the packs folder. */
  basemap: string | null;
  dem: string | null;
  preview: string | null;
  roadsKm: number | null;
  sizeKm: [number, number];
  towns: { name: string; lat: number; lon: number; radiusM: number }[];
}

/** A terrain build's progress. */
export interface TerrainProgress {
  name: string;
  state: "running" | "done" | "failed" | "busy";
  stage?: string;
  fraction?: number;
  text?: string;
}

export interface GameInfo {
  name: string;
  description: string;
  notes: string[];
  scope: { type: string; durationMin: number; area?: { widthKm: number; depthKm: number } };
  startUtc: string;
  victory: { kind: string; pct?: number }[];
  origin: { lat: number; lon: number };
  forces: { blue: number; red: number };
  deciders: Record<"blue" | "red", SideDeciders>;
  flat: boolean;
  limitS: number;
  pack: PackInfo | null;
}

export interface ScenarioBrief {
  file: string;
  name: string;
  description: string;
  type: string;
  durationMin: number;
  units: number;
}

/** One unit in detail (`inspect`): truth for the umpire and own units. */
export interface UnitDetail {
  id: number;
  name: string;
  side: Side;
  own: boolean;
  gone?: boolean;
  // Another side's unit: only what the viewer's side has seen.
  known?: string;
  lastSeenS?: number | null;
  label?: string | null;
  platform?: string | null;
  platformId?: string;
  role?: string;
  domain?: string;
  template?: string;
  quality?: number;
  hq?: boolean;
  dummy?: boolean;
  speed?: number;
  health?: { fit: number; start: number; mobilityKills: number; firepowerKills: number; destroyed: boolean } | null;
  activity?: {
    word: string;
    detail: string;
    /** Waypoints still to go, as [lon, lat]. */
    route: [number, number][];
    posture: string;
    roe: string;
    deciding: boolean;
    mission: { purpose: string; phase: number; urgent: boolean; phases: string[] };
  } | null;
  people?: {
    suppression: number;
    pinned: boolean;
    morale: string;
    stress: number;
    cohesion: number;
    fatigue: number;
    sleepDebtH: number;
    skill: number;
    lastFiredOnS: number | null;
    lastFiredOnBy: string | null;
  } | null;
  mounts?: { weapon: string; munition: string; rounds: number }[];
  knows?: { id?: number; name?: string; level?: string; rangeM?: number | null; agoS: number; moving?: boolean; suspect?: boolean; bearingDeg?: number }[];
  decisions?: { t: number; text: string; data: GameEvent["data"] }[];
  recent?: { t: number; kind: string; text: string }[];
}

export interface Viewshed {
  unit: string;
  side: Side;
  cols: number;
  rows: number;
  /** Grid cell, m; how far it looks (its best sensor), m; its weapons' reach, m. */
  cellM: number;
  rangeM: number;
  weaponM: number;
  /** Where it stood when this was worked out. */
  lon: number;
  lat: number;
  west: number;
  south: number;
  east: number;
  north: number;
  mask: string;
}

/** What the browser sends. */
export type Outbound =
  | { kind: "hello" }
  | { kind: "play" }
  | { kind: "pause" }
  | { kind: "speed"; speed: number }
  | { kind: "viewer"; viewer: Viewer }
  | { kind: "answer"; question: number; choice: string }
  | { kind: "command"; side: Side; plan: unknown }
  | { kind: "seat"; side: Side; seat: "planner" | "person" | "rules" }
  | { kind: "seek"; t: number }
  | { kind: "live" }
  | { kind: "branch"; t: number }
  | { kind: "save" }
  | { kind: "why"; unit: string }
  | { kind: "review" }
  | { kind: "detect"; observer: string; target: string }
  | { kind: "messages"; unit: string }
  | { kind: "viewshed"; unit: string }
  | { kind: "inspect"; unit: string }
  | { kind: "scenarios" }
  | { kind: "open"; name: string; pack?: string }
  | { kind: "terrain"; name: string; bbox: [number, number, number, number]; cell_m?: number };

/** Positions between full frames: u8 1, f64 t, u32 n; then u32 id, f64 lat, f64 lon, f32 alt, f32 heading, u8 status. */
const STATUS = ["ok", "suppressed", "pinned", "broken", "destroyed"];

export function applyPositions(frame: FrameView, buf: ArrayBuffer): number | null {
  const v = new DataView(buf);
  if (v.byteLength < 13 || v.getUint8(0) !== 1) return null;
  const t = v.getFloat64(1, true);
  const n = v.getUint32(9, true);
  const byId = new Map<number, FrameEntity>(frame.entities.map((e) => [e.id, e]));
  for (let i = 0, o = 13; i < n && o + 29 <= v.byteLength; i++, o += 29) {
    const e = byId.get(v.getUint32(o, true));
    if (!e) continue;
    e.lat = v.getFloat64(o + 4, true);
    e.lon = v.getFloat64(o + 12, true);
    e.alt = v.getFloat32(o + 20, true);
    e.heading_deg = v.getFloat32(o + 24, true);
    e.status = STATUS[v.getUint8(o + 28)] ?? e.status;
  }
  frame.t = t;
  frame.clock = clock(t);
  return t;
}

/** Simulated seconds as hh:mm:ss. */
export function clock(t: number): string {
  const s = Math.max(0, Math.floor(t));
  return [Math.floor(s / 3600), Math.floor(s / 60) % 60, s % 60].map((x) => String(x).padStart(2, "0")).join(":");
}

/** A span of seconds in words: "40 s", "3 min", "2 h 10 min". */
export function span(s: number): string {
  if (s < 90) return `${Math.round(s)} s`;
  if (s < 5400) return `${Math.round(s / 60)} min`;
  const h = Math.floor(s / 3600);
  return `${h} h ${Math.round((s - h * 3600) / 60)} min`;
}
