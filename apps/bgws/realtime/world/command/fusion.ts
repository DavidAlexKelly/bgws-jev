// ── world/command/fusion.ts ──────────────────────────────────────────────
// An HQ's fused picture, its COP (MULTI_DOMAIN_PLAN.md §4.3, W4): built from
// what has REACHED the HQ, never from the truth.
//
//   - its command post's own land picture: what the post saw, and the contact
//     reports the land radio net has carried to it (comms.ts, with its relays
//     and delays);
//   - world tracks reported by its own subordinates' sensors (drones, radars,
//     aircraft, ships) over a data link, as old as the link makes them.
//
// Track quality falls with age (Link-16's 0–15); a decoy is held as real.
// Each time a contact first reaches the HQ, or a newer sighting of it does,
// the latency from sighting to HQ is recorded (stats.reportLatencyS): a
// destroyed relay or HQ shows up there.

import type { Side } from "../../../lib/state";
import { trackedBy } from "../../engine/picture";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { hasRecord, recordFor } from "../../catalogue";
import { deliveredImagery } from "../imagery";
import { outaged } from "../net";
import type { Headquarters, WorldTrack } from "../types";

const LEVEL_CLASS = { veryPartial: "track", partial: "class", full: "identity" } as const;

/** Every unit and entity under this HQ, all the way down. */
export function subtree(c: Pick<WorldCtx, "world">, hqId: string, seen = new Set<string>()): string[] {
  const hq = c.world.hqs[hqId];
  if (!hq || seen.has(hqId)) return [];
  seen.add(hqId);
  const out: string[] = [hq.postId];
  for (const sub of hq.subordinates) {
    if (c.world.hqs[sub]) out.push(...subtree(c, sub, seen));
    else out.push(sub);
  }
  return out;
}

/** Is this sensor's only way home a SATCOM bearer that is cut? */
function feedCut(c: WorldCtx, sensorId: string): boolean {
  const e = c.entity(sensorId);
  if (!e || !hasRecord(e.archetype)) return false;
  const bearers = recordFor(e.archetype).bearers;
  return bearers.length > 0 && bearers.every((b) => b.startsWith("satcom")) && bearers.every((b) => outaged(c, b, sensorId));
}

/** The HQ's COP as of now. */
export function fuse(c: WorldCtx, hq: Headquarters): Record<string, WorldTrack> {
  const cop: Record<string, WorldTrack> = {};
  const post = hq.postId;
  const unit = c.land.units[post];
  const offer = (t: WorldTrack) => {
    const had = cop[t.id];
    if (!had || t.seenAt > had.seenAt) cop[t.id] = t;
  };
  const age = (seenAt: number) => c.time - seenAt;
  const quality = (seenAt: number, base: number) => Math.max(0, Math.round(base - age(seenAt) / wfig("world/command/fusion.QUALITY_DECAY_S")));
  if (unit && c.landAlive(post)) {
    for (const [enemyId, seen] of Object.entries(unit.ownSeen)) {
      if (!c.landAlive(enemyId)) continue;
      offer({ id: enemyId, truthId: enemyId, at: seen.at ?? c.fe(enemyId)!.position, altM: 0, seenAt: seen.time, quality: quality(seen.time, 14), classification: LEVEL_CLASS[seen.level], thought: seen.level === "full" ? c.fe(enemyId)!.label : "vehicle", domain: "land", by: post, errorM: 25 });
    }
    trackedBy({ game: c.game, units: c.land.units, reports: c.land.prev.reports }, post, (enemyId, track) => {
      if (!c.landAlive(enemyId)) return;
      offer({ id: enemyId, truthId: enemyId, at: track.at, altM: 0, seenAt: track.seenAt, quality: quality(track.seenAt, 12), classification: LEVEL_CLASS[track.level], thought: track.label ?? (track.level === "full" ? "identified" : "vehicle"), domain: "land", by: track.from, errorM: track.errorM });
    });
  }
  // Tracks from the HQ's own sensors and subordinates', over the data link.
  const mine = new Set(subtree(c, hq.id));
  const linkS = wfig("world/net.DATALINK_S");
  for (const t of Object.values(c.world.tracks[hq.side])) {
    if (!mine.has(t.by) && !(c.entity(t.by)?.parent && mine.has(c.entity(t.by)!.parent!))) continue;
    // A sensor that reports only over SATCOM goes dark when its SATCOM is cut (W9).
    if (feedCut(c, t.by)) continue;
    if (c.time - t.seenAt < linkS) continue;
    offer({ ...t, quality: quality(t.seenAt, t.quality) });
  }
  // Spot reports from HQs below, as they arrived.
  const byId = new Map(c.world.messages.map((m) => [m.id, m]));
  for (const id of hq.inbox) {
    const m = byId.get(id);
    if (m?.kind !== "spotrep" || !Array.isArray(m.body.tracks)) continue;
    for (const t of m.body.tracks as WorldTrack[]) if (c.landAlive(t.id) || c.entity(t.id)?.status === "active") offer({ ...t, quality: quality(t.seenAt, t.quality - 1) });
  }
  // Emitters the side's SIGINT has fixed by bearings (W6): where a radar or a radio was, to the fix's error.
  for (const f of Object.values(c.world.fixes[hq.side])) {
    if (age(f.time) > wfig("world/sensing.TRACK_DROP_S") * 3 || !(c.landAlive(f.emitterId) || c.entity(f.emitterId)?.status === "active")) continue;
    offer({ id: f.emitterId, truthId: f.emitterId, at: f.at, altM: 0, seenAt: f.time, quality: quality(f.time, 8), classification: "class", thought: `${f.kind} emitter`, domain: "land", by: "esm", errorM: f.errorM });
  }
  // Satellite imagery the side has received: old, and only what was there when the image was taken (W9).
  for (const t of deliveredImagery(c, hq.side)) if (c.landAlive(t.id) || c.entity(t.id)?.status === "active") offer({ ...t, quality: quality(t.seenAt, t.quality) });
  // What the HQ above passed down: its COP, older still.
  if (hq.parent && c.world.hqs[hq.parent]) {
    for (const t of Object.values(c.world.hqs[hq.parent].cop)) if (age(t.seenAt) < wfig("world/sensing.TRACK_DROP_S")) offer({ ...t, quality: Math.max(0, t.quality - 2) });
  }
  return cop;
}

/** Update every HQ's COP, recording how late new reports arrive. */
export function fuseAll(c: WorldCtx): void {
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    if (hq.down) continue;
    const cop = fuse(c, hq);
    const latencies: number[] = [];
    for (const [tid, t] of Object.entries(cop)) {
      const had = hq.cop[tid];
      if (t.domain === "land" && (!had || t.seenAt > had.seenAt)) latencies.push(c.time - t.seenAt);
    }
    c.world.hqs[id] = { ...hq, cop };
    if (latencies.length) recordLatency(c, hq.side, id, latencies);
  }
}

function recordLatency(c: WorldCtx, side: Side, hqId: string, latencies: number[]): void {
  c.stat((s) => ({
    ...s,
    reportLatencyS: { ...s.reportLatencyS, [side]: [...s.reportLatencyS[side], ...latencies].slice(-2000) },
    reportLatencyByHq: { ...s.reportLatencyByHq, [hqId]: [...(s.reportLatencyByHq[hqId] ?? []), ...latencies].slice(-2000) },
  }));
}

/** Spot reports up the tree: each HQ passes what is new in its picture to the HQ above, over its nets. */
export function reportUp(c: WorldCtx, sendNetFn: (c: WorldCtx, spec: { side: Side; kind: "spotrep"; from: string; to: string; text: string; body: Record<string, unknown> }) => void): void {
  const every = wfig("world/command/fusion.SPOTREP_S");
  if (c.time % every !== 0) return;
  for (const id of Object.keys(c.world.hqs).sort()) {
    const hq = c.world.hqs[id];
    const parent = hq.parent ? c.world.hqs[hq.parent] : undefined;
    if (hq.down || !parent || parent.down) continue;
    const since = c.world.memo[`spotrep:${id}`] ?? -Infinity;
    const fresh = Object.values(hq.cop).filter((t) => t.seenAt > since);
    if (!fresh.length) continue;
    c.mark(`spotrep:${id}`);
    sendNetFn(c, { side: hq.side, kind: "spotrep", from: hq.postId, to: parent.postId, text: `${id}: ${fresh.length} contact${fresh.length === 1 ? "" : "s"}`, body: { tracks: fresh } });
  }
}
