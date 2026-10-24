// ── realtime/world/sensing.ts ────────────────────────────────────────────
// World sensors (MULTI_DOMAIN_PLAN.md §4.6, W2): every sensor turns target
// signatures into a chance of detection, through a model of its type, and
// each detection updates the observing side's track on the target.
//
//   eye, optic, thermal, eo  a hazard per second from the visual or IR
//                            signature, light and weather; line of sight
//   radar, cbr               a detection range from the radar cross-section
//                            (R ∝ σ^¼), the radar horizon and terrain; one
//                            look per scan period; the radar is an emitter
//   esm, rwr                 emitters on the air, in band and in range
//   sonar                    passive: SL − TL − NL + DI ≥ DT; noise rises with speed
//   missileWarning           ballistic launches anywhere it covers
//
// The land engine's own spotting (detection.ts) is untouched: land units
// still find each other its way. This adds what it cannot see — aircraft,
// drones, ships, submarines, rounds in flight — and what world platforms
// (drones, aircraft, satellites) see of land units. A sighting a land unit's
// drone or radar makes reaches its side's land picture as a contact report
// from that unit, over the radio net, like any other.
//
// Sensors are staggered: each looks on its own scan period, offset by its
// owner's id, so a thousand radars do not all look in the same second.

import { hasRecord, recordFor } from "../catalogue";
import type { Classification, SensorSpec } from "../catalogue/schema";
import type { Side } from "../../lib/state";
import type { LatLng } from "../../lib/board";
import { opposing } from "../../lib/state";
import type { WorldCtx } from "./context";
import { sensorConditions } from "./environment";
import { wfig } from "./figures";
import { distanceM, groundAt, horizonM, seesBetween, bearingDeg } from "./geo";
import { registerSystem } from "./step";
import { DETECTION_LISTENERS } from "./hooks";
import type { Entity, WorldTrack } from "./types";

/** Something a sensor could see: a land unit or an entity, flattened for the sensor models. */
export interface Target {
  id: string;
  side: Side | "neutral";
  domain: Entity["domain"] | "land";
  at: LatLng;
  altM: number;
  speedMs: number;
  /** Signatures: visual and IR multipliers, RCS m², acoustic dB. */
  visual: number;
  ir: number;
  rcsM2: number;
  acousticDb: number;
  label: string;
  /** What a "type" classification would call it. */
  typeName: string;
  decoy: boolean;
}

/** Someone with sensors: a land unit's or an entity's. */
export interface Observer {
  id: string;
  side: Side;
  at: LatLng;
  /** Height of the platform above sea level, m (the sensor's own mast is added). */
  altM: number;
  sensors: SensorSpec[];
  /** The land unit that reports what it sees on the land net, if any: itself, or a drone's team. */
  reportsVia?: string;
  silent: boolean;
  sensorKilled: boolean;
}

const CLASS_RANK: Record<Classification, number> = { track: 0, class: 1, type: 2, identity: 3 };
const CLASS_BY_RANK: Classification[] = ["track", "class", "type", "identity"];

function stagger(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h;
}

/** Due to look this second? */
function looks(c: WorldCtx, ownerId: string, sensor: SensorSpec, index: number): boolean {
  const period = Math.max(1, Math.round(sensor.scanS));
  return period <= 1 || (Math.round(c.time) + stagger(ownerId) + index) % period === 0;
}

export function targetsOf(c: WorldCtx): Target[] {
  const out: Target[] = [];
  for (const id of Object.keys(c.game.forceElements).sort()) {
    const fe = c.game.forceElements[id];
    if (!c.landAlive(id)) continue;
    const comps = c.comps(id);
    if (comps?.carrier && !c.comps(comps.carrier)?.dismounts.out.includes(id)) continue;
    const rec = comps && hasRecord(comps.archetype) ? recordFor(comps.archetype) : undefined;
    const sig = rec?.signatures ?? { visual: 1, ir: 1, rcsM2: 10, acousticDb: 0 };
    const unit = c.land.units[id];
    out.push({
      id,
      side: fe.side,
      domain: "land",
      at: fe.position,
      altM: groundAt(c.config.terrain, fe.position) + 2,
      speedMs: unit?.lastMovedAt === c.time ? 5 : 0,
      visual: sig.visual * (unit?.lastMovedAt === c.time ? 1.5 : 1) * (unit && c.time - unit.lastShotAt < 10 ? 2 : 1),
      ir: sig.ir,
      rcsM2: sig.rcsM2,
      acousticDb: 0,
      label: fe.label,
      typeName: rec?.name ?? fe.label,
      decoy: fe.isDummy,
    });
  }
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.status !== "active" || e.domain === "space") continue;
    let sig = { visual: 0.3, ir: 1.5, rcsM2: 0.1, acousticDb: 0 };
    let typeName = e.label;
    if (e.domain !== "munition" && hasRecord(e.archetype)) {
      const rec = recordFor(e.archetype);
      sig = rec.signatures;
      typeName = rec.name;
    }
    // A decoy is built to look like what it decoys, on radar and to the eye.
    const decoy = e.munition?.decoy === true || e.archetype === "generic/owaDecoy";
    const noisy = e.domain === "subsurface" ? sig.acousticDb + 20 * Math.log10(1 + e.speedMs / 3) : sig.acousticDb;
    out.push({
      id,
      side: e.owner,
      domain: e.domain,
      at: e.pos,
      altM: e.altM,
      speedMs: e.speedMs,
      visual: sig.visual,
      ir: sig.ir * (e.domain === "munition" ? 2 : 1),
      rcsM2: sig.rcsM2,
      acousticDb: noisy,
      label: e.label,
      typeName: decoy && e.archetype === "generic/owaDecoy" ? recordFor("generic/owaDrone").name : typeName,
      decoy,
    });
  }
  return out;
}

export function observersOf(c: WorldCtx): Observer[] {
  const out: Observer[] = [];
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id) || !hasRecord(comps.archetype)) continue;
    const sensors = recordFor(comps.archetype).sensors.filter((s) => s.kind !== "eye" && s.kind !== "thermal");
    if (!sensors.length) continue;
    const fe = c.fe(id)!;
    out.push({
      id,
      side: fe.side,
      at: fe.position,
      altM: groundAt(c.config.terrain, fe.position),
      sensors,
      reportsVia: id,
      silent: comps.emcon === "silent",
      sensorKilled: comps.damage.sensor,
    });
  }
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.status !== "active" || e.domain === "munition" || e.domain === "space" || !hasRecord(e.archetype)) continue;
    if (e.owner === "neutral") continue;
    const sensors = recordFor(e.archetype).sensors;
    if (!sensors.length) continue;
    const parentLand = e.parent && c.fe(e.parent) ? e.parent : undefined;
    out.push({
      id,
      side: e.owner,
      at: e.pos,
      altM: e.altM,
      sensors,
      ...(parentLand ? { reportsVia: parentLand } : {}),
      silent: e.emcon === "silent",
      sensorKilled: e.damage.sensor,
    });
  }
  return out;
}

/** Chance this sensor detects this target now, or 0. Also what classification it reaches. */
export function detection(c: WorldCtx, o: Observer, s: SensorSpec, t: Target): { p: number; classification: Classification } {
  const none = { p: 0, classification: "track" as Classification };
  const range = distanceM(o.at, t.at);
  if (range > s.rangeM * 2.5) return none;
  const air = t.domain === "air" || t.domain === "munition";
  const sub = t.domain === "subsurface";
  const env = sensorConditions(c.world.env, c.time, s.kind);
  const eyeAlt = o.altM + s.heightM;
  // How far up the ladder: identity only well inside range.
  const cls = (frac: number): Classification => {
    const max = CLASS_RANK[s.classifies];
    const rank = frac < 0.3 ? max : frac < 0.6 ? Math.min(max, 2) : frac < 0.9 ? Math.min(max, 1) : 0;
    return CLASS_BY_RANK[rank];
  };
  switch (s.kind) {
    case "radar":
    case "cbr": {
      if (o.silent || sub) return none;
      if (air && !s.seesAir) return none;
      if (s.kind === "cbr" && t.domain !== "munition") return none;
      if (!air && t.domain === "land" && s.seesAir && s.kind === "radar") {
        // An air-search radar does not pick ground vehicles out of the clutter.
        return none;
      }
      const reach = s.rangeM * Math.pow(Math.max(1e-4, t.rcsM2) / 5, 0.25) * env.factor;
      if (range > reach) return none;
      if (range > horizonM(Math.max(1, eyeAlt - Math.min(0, groundAt(c.config.terrain, o.at))), Math.max(1, t.altM))) return none;
      if (!seesBetween(c.config.terrain, o.at, eyeAlt, t.at, t.altM)) return none;
      const frac = range / reach;
      return { p: 0.95 * (1 - frac ** 4) + 0.03, classification: cls(frac) };
    }
    case "eye":
    case "optic":
    case "thermal":
    case "eo":
    case "irst": {
      if (sub) return none;
      if (t.domain === "surface" && s.kind === "eo" && o.altM < 50) {
        // A small boat's camera sees ships, fine.
      }
      const signature = s.kind === "thermal" || s.kind === "irst" ? t.ir : t.visual;
      const reach = Math.min(env.maxRangeM, s.rangeM * Math.sqrt(Math.max(0.01, signature)));
      if (range > reach) return none;
      if (!seesBetween(c.config.terrain, o.at, eyeAlt, t.at, t.altM)) return none;
      const frac = range / reach;
      // A hazard per second, so looking for longer finds more, at any tick length.
      const perSecond = wfig("world/sensing.OPTIC_RATE") * (1 - frac) * env.factor;
      return { p: 1 - Math.exp(-perSecond * Math.max(1, s.scanS)), classification: cls(frac) };
    }
    case "sonar": {
      if (!(sub || t.domain === "surface")) return none;
      // Passive sonar equation: SL − TL − NL + DI ≥ DT, TL = 20·log10(r).
      const tl = 20 * Math.log10(Math.max(1, range));
      const excess = t.acousticDb - tl - wfig("world/sensing.SONAR_NOISE_DB") + wfig("world/sensing.SONAR_DI_DB") - wfig("world/sensing.SONAR_DT_DB");
      if (excess < -6 || range > s.rangeM) return none;
      return { p: 1 / (1 + Math.exp(-excess / 3)), classification: cls(range / s.rangeM) };
    }
    case "missileWarning": {
      if (t.domain !== "munition") return none;
      const m = c.entity(t.id)?.munition;
      if (!m || !["ballistic", "cruise"].includes(c.entity(t.id)?.label ?? "")) return none;
      return { p: 0.95, classification: "class" };
    }
    default:
      return none;
  }
}

/** Fold a detection into the side's track on the target. */
export function updateTrack(c: WorldCtx, side: Side, t: Target, by: string, classification: Classification, quality: number, errorM: number): WorldTrack {
  const tracks = c.world.tracks[side];
  const had = tracks[t.id];
  const better = !had || CLASS_RANK[classification] >= CLASS_RANK[had.classification];
  const cls: Classification = better ? classification : had.classification;
  const thought =
    cls === "identity" ? t.label : cls === "type" ? t.typeName : cls === "class" ? (t.domain === "land" ? "vehicle" : t.domain === "munition" ? "missile" : t.domain === "air" ? "aircraft" : t.domain === "surface" ? "ship" : t.domain) : "unknown";
  const track: WorldTrack = {
    id: t.id,
    truthId: t.id,
    at: t.at,
    altM: t.altM,
    seenAt: c.time,
    quality: Math.max(quality, had && c.time - had.seenAt < 10 ? had.quality : 0),
    classification: cls,
    thought,
    domain: t.domain === "land" ? "land" : t.domain,
    by,
    ...(t.decoy ? { falseTrack: true } : {}),
    errorM,
  };
  tracks[t.id] = track;
  return track;
}

const QUALITY: Record<string, number> = { radar: 11, cbr: 9, eo: 12, optic: 12, eye: 10, thermal: 12, irst: 9, sonar: 6, missileWarning: 7 };

export function stepSensing(c: WorldCtx): void {
  const targets = targetsOf(c);
  const observers = observersOf(c);
  for (const o of observers) {
    if (o.sensorKilled) continue;
    o.sensors.forEach((s, index) => {
      if (s.kind === "esm" || s.kind === "rwr") return; // world/domains/ew.ts
      if (!looks(c, o.id, s, index)) return;
      if (s.active && !o.silent) c.world.emissions.push({ emitterId: o.id, side: o.side, kind: "radar", at: o.at, time: c.time });
      for (const t of targets) {
        if (t.side === o.side || t.id === o.id) continue;
        // Neutral shipping is seen by everyone, and is what ROE protects.
        const d = detection(c, o, s, t);
        if (d.p <= 0 || !c.chance(d.p)) continue;
        const errorM = s.kind === "radar" ? 50 + distanceM(o.at, t.at) * 0.005 : s.kind === "sonar" ? 500 : 20;
        const isNew = !c.world.tracks[o.side][t.id] || c.time - c.world.tracks[o.side][t.id].seenAt > 60;
        const track = updateTrack(c, o.side, t, o.id, d.classification, QUALITY[s.kind] ?? 8, errorM);
        for (const l of DETECTION_LISTENERS) l(c, o, t, track, isNew);
        // A land enemy seen by a land unit's drone or radar goes on the land net.
        if (t.domain === "land" && t.side === opposing(o.side) && o.reportsVia && c.landAlive(o.reportsVia)) {
          reportToLand(c, o.reportsVia, t, d.classification);
        }
      }
    });
  }
  // Tracks age: quality falls while nothing updates them, and a dead one is dropped.
  for (const side of ["blue", "red"] as const) {
    const tracks = c.world.tracks[side];
    for (const id of Object.keys(tracks)) {
      const t = tracks[id];
      const age = c.time - t.seenAt;
      const truth = t.truthId ? (c.entity(t.truthId) ?? (c.fe(t.truthId) ? { status: c.landAlive(t.truthId) ? "active" : "destroyed" } : undefined)) : undefined;
      if (age > wfig("world/sensing.TRACK_DROP_S") || (truth && truth.status !== "active" && truth.status !== "landed" && age > 5)) {
        delete tracks[id];
      } else if (age > 0 && age % 20 === 0 && t.quality > 0) {
        tracks[id] = { ...t, quality: t.quality - 1 };
      }
    }
  }
}

/** A land unit tells its side, over the land net, what its world sensor found. */
function reportToLand(c: WorldCtx, reporterId: string, t: Target, classification: Classification): void {
  const unit = c.land.units[reporterId];
  if (!unit) return;
  const last = unit.lastReported[t.id] ?? -Infinity;
  if (c.time - last < 30) return;
  c.land.setUnit(reporterId, { lastReported: { ...unit.lastReported, [t.id]: c.time } });
  const level = classification === "identity" || classification === "type" ? "full" : "partial";
  const from = c.fe(reporterId)!.position;
  c.land.radio({
    kind: "contact",
    from: reporterId,
    to: "all",
    text: `contact (from the air): ${level === "full" ? t.label : "unidentified"} (${t.id}), ${Math.round(distanceM(from, t.at))} m ${Math.round(bearingDeg(from, t.at))}° of ${reporterId}`,
    contact: { enemyId: t.id, level, at: t.at, seenAt: c.time, errorM: 30, moving: t.speedMs > 0, ...(level === "full" ? { label: t.label } : {}) },
  });
}

export { onDetection, type DetectionListener } from "./hooks";

registerSystem({
  name: "sensing",
  order: 40,
  when: (c) => Object.keys(c.world.entities).length > 0 || Object.values(c.world.land).some((l) => hasRecord(l.archetype) && recordFor(l.archetype).sensors.some((s) => s.kind !== "eye" && s.kind !== "thermal")),
  run: stepSensing,
});
