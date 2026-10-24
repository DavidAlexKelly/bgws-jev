// ── world/domains/space.ts ───────────────────────────────────────────────
// Space (MULTI_DOMAIN_PLAN.md W9): an enabling layer the other domains feel.
//
//   ORBITS       satellites move on two-body orbits (geo.ts), evaluated only
//                every so often: where they are matters at access checks.
//   ISR          an EO or SAR satellite passing within reach of somewhere its
//                side wants to see asks the space cell (spaceTasking): image
//                this area on the pass, or hold it. Cloud blocks EO, not SAR.
//                What the image saw reaches the side after a processing
//                latency (30–90 min), as a batch of detections in every HQ's
//                picture: old, but there. A commercial provider can cut its
//                service off by scenario event [ref R5.4].
//   PNT, SATCOM  GNSS quality is the side's (pnt.ts); counterspace jamming
//                lowers it. SATCOM bearers are up while their satellite is
//                and no outage or provider cut-off stands (net.ts).
//   WARNING      a missile-warning satellite sees ballistic launches and puts
//                them in its side's picture at once, so air defence is cued
//                with the time of flight still to run.
//   COUNTERSPACE scenario events: dazzling an EO satellite, jamming GNSS, a
//                kinetic ASAT as the rare escalation [ref R5.3].

import type { LatLng } from "../../../lib/board";
import { opposing, SIDES, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { onCreateWorld } from "../create";
import { eoClearChance } from "../environment";
import { wfig } from "../figures";
import { distanceM, subSatellitePoint } from "../geo";
import { raiseQuestion } from "../questions";
import { onScheduled, registerSystem } from "../step";
import type { Entity, ImageryBatch, WorldTrack } from "../types";

const ISR = new Set(["generic/satEo", "generic/satSar"]);

/** The orbit's figures from the catalogue: altitude and inclination. */
function orbitOf(e: Entity): { altKm: number; incl: number } {
  const m = recordFor(e.archetype).mover;
  return { altKm: m.orbitAltKm ?? 500, incl: m.inclinationDeg ?? 97 };
}

export function satellitePos(e: Entity, t: number): LatLng {
  const { altKm, incl } = orbitOf(e);
  return subSatellitePoint(altKm, incl, e.orbitPhaseDeg ?? 0, t, e.orbitRaanDeg ?? 0);
}

/**
 * Place a satellite's orbit so its first pass over `over` falls about `afterS`
 * from the start: the ascending node set from where the ground track must be.
 */
function firstPassOver(e: Entity, over: LatLng, afterS: number): { phase: number; raan: number } {
  const { altKm, incl } = orbitOf(e);
  // Search the node longitude that puts the sub-satellite point closest to `over` at `afterS`.
  let best = { raan: 0, phase: 0, d: Infinity };
  for (let phase = 0; phase < 360; phase += 5) {
    for (let raan = -180; raan < 180; raan += 5) {
      const p = subSatellitePoint(altKm, incl, phase, afterS, raan);
      const d = distanceM(p, over);
      if (d < best.d) best = { raan, phase, d };
    }
  }
  return { phase: best.phase, raan: best.raan };
}

onCreateWorld(35, (world, state) => {
  // A satellite with no orbit given gets one whose first pass over the board falls early in the game.
  const fes = Object.values(state.game.forceElements);
  if (!fes.length) return;
  const centre = { lat: fes.reduce((s, f) => s + f.position.lat, 0) / fes.length, lng: fes.reduce((s, f) => s + f.position.lng, 0) / fes.length };
  let n = 0;
  for (const e of Object.values(world.entities).sort((a, b) => a.id.localeCompare(b.id))) {
    if (e.domain !== "space" || !ISR.has(e.archetype) || e.orbitRaanDeg != null) continue;
    const o = firstPassOver(e, centre, 900 + n * 1800);
    world.entities[e.id] = { ...e, orbitPhaseDeg: e.orbitPhaseDeg ?? o.phase, orbitRaanDeg: o.raan, pos: subSatellitePoint(orbitOf(e).altKm, orbitOf(e).incl, e.orbitPhaseDeg ?? o.phase, 0, o.raan) };
    n++;
  }
});

/** Where a side wants to look: the enemy as last known, its start line, the side's objective. */
function areasOfInterest(c: WorldCtx, side: Side): { at: LatLng; label: string; priority: number }[] {
  const out: { at: LatLng; label: string; priority: number }[] = (c.world.nais ?? []).filter((n) => n.side === side).map((n) => ({ at: n.at, label: n.label, priority: n.priority }));
  const enemy = opposing(side);
  const hqs = Object.values(c.world.hqs).filter((h) => h.side === side && !h.down);
  // Old contacts: where the enemy was, worth looking again (CCIR: where is the enemy now?).
  for (const h of hqs) for (const t of Object.values(h.cop)) out.push({ at: t.at, label: `last seen ${t.thought}`, priority: 2 + Math.min(3, (c.time - t.seenAt) / 3600) });
  out.push({ at: c.world.startLines[enemy], label: "the enemy's rear", priority: 2 });
  const objective = c.game.objectives?.[side];
  if (objective) out.push({ at: objective, label: "the objective", priority: 1.5 });
  return out.sort((a, b) => b.priority - a.priority);
}

const footprintM = (e: Entity) => (e.archetype === "generic/satSar" ? wfig("world/domains/space.SAR_FOOTPRINT_M") : wfig("world/domains/space.EO_FOOTPRINT_M"));

function passes(c: WorldCtx): void {
  if (c.time % wfig("world/domains/space.ACCESS_CHECK_S") !== 0) return;
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.domain !== "space" || e.status !== "active" || !hasRecord(e.archetype)) continue;
    const pos = satellitePos(e, c.time);
    c.setEntity(id, { pos });
    if (!ISR.has(e.archetype) || e.owner === "neutral") continue;
    const side = e.owner as Side;
    const reach = recordFor(e.archetype).sensors[0]?.rangeM ?? 300_000;
    const pass = `pass:${id}:${Math.floor(c.time / wfig("world/domains/space.PASS_GAP_S"))}`;
    if (c.world.memo[pass] != null) continue;
    const area = areasOfInterest(c, side).find((a) => distanceM(a.at, pos) <= reach);
    if (!area) continue;
    c.world.memo[pass] = c.time;
    raiseQuestion(c, { agentId: id, side, trigger: "spaceTasking", about: { lat: area.at.lat, lng: area.at.lng, label: area.label, cloud: c.world.env.cloudCover, eo: e.archetype === "generic/satEo" }, quality: 6 });
  }
}

/** Take the image: what is in the footprint, to be delivered after processing. */
function image(c: WorldCtx, sat: Entity, at: LatLng): void {
  const side = sat.owner as Side;
  if (sat.provider === "commercial" && (c.world.outages[`imagery:${side}`] ?? -Infinity) > c.time) {
    c.log({ side, domain: "space", kind: "cutOff", text: `${sat.label} passes, but the commercial provider has cut ${side} off`, ids: [sat.id] });
    return;
  }
  if ((c.world.outages[`dazzle:${sat.id}`] ?? -Infinity) > c.time) {
    c.log({ side, domain: "space", kind: "dazzled", text: `${sat.label} is dazzled: no image this pass`, ids: [sat.id] });
    return;
  }
  if (sat.archetype === "generic/satEo" && !c.chance(eoClearChance(c.world.env))) {
    c.log({ side, domain: "space", kind: "clouded", text: `${sat.label}'s image is clouded over`, ids: [sat.id], at });
    return;
  }
  const radius = footprintM(sat);
  const enemy = opposing(side);
  const detections: WorldTrack[] = [];
  const thought = (archetype: string) => (hasRecord(archetype) ? recordFor(archetype).name : "vehicle");
  for (const id of Object.keys(c.game.forceElements).sort()) {
    const fe = c.fe(id)!;
    if (fe.side !== enemy || !c.landAlive(id) || distanceM(fe.position, at) > radius) continue;
    // Small units in cover are often missed; dummies look real.
    if (!c.chance(wfig("world/domains/space.DETECT_P"))) continue;
    detections.push({ id, truthId: id, at: fe.position, altM: 0, seenAt: c.time, quality: 7, classification: "class", thought: thought(c.world.land[id]?.archetype ?? ""), domain: "land", by: sat.id, errorM: wfig("world/domains/space.ERROR_M") });
  }
  for (const e of Object.values(c.world.entities)) {
    if (e.owner !== enemy || e.status !== "active" || !["surface", "node"].includes(e.domain) || distanceM(e.pos, at) > radius) continue;
    detections.push({ id: e.id, truthId: e.id, at: e.pos, altM: 0, seenAt: c.time, quality: 8, classification: "type", thought: thought(e.archetype), domain: e.domain === "node" ? "land" : "surface", by: sat.id, errorM: wfig("world/domains/space.ERROR_M") });
  }
  const lo = wfig("world/domains/space.PROCESS_MIN_S");
  const hi = wfig("world/domains/space.PROCESS_MAX_S");
  const batch: ImageryBatch = { id: `img-${c.world.nextId}`, side, satelliteId: sat.id, takenAt: c.time, deliveredAt: c.time + lo + c.rng.int(Math.max(1, hi - lo)), at, detections };
  (c.world as { nextId: number }).nextId = c.world.nextId + 1;
  c.world.imagery = [...(c.world.imagery ?? []), batch];
  c.schedule(batch.deliveredAt, "imageryDelivered", batch.id);
  c.log({ side, domain: "space", kind: "imaged", text: `${sat.label} images ${detections.length ? "the area" : "an empty area"}; processing until +${Math.round((batch.deliveredAt - c.time) / 60)} min`, ids: [sat.id], at });
}

onScheduled("imageryDelivered", (c, e) => {
  const batch = c.world.imagery?.find((b) => b.id === e.key);
  if (!batch) return;
  c.stat((s) => ({ ...s, imageryLatencyS: [...(s.imageryLatencyS ?? []), c.time - batch.takenAt] }));
  c.log({ side: batch.side, domain: "space", kind: "imagery", text: `imagery from ${batch.satelliteId} reaches ${batch.side}: ${batch.detections.length} detections, ${Math.round((c.time - batch.takenAt) / 60)} min old`, ids: batch.detections.map((d) => d.id), at: batch.at });
});

/** Ballistic launches seen from orbit: in the side's picture at once, cueing its defences. */
function missileWarning(c: WorldCtx): void {
  for (const side of SIDES) {
    const warner = Object.values(c.world.entities).find((e) => e.owner === side && e.archetype === "generic/missileWarning" && e.status === "active");
    if (!warner) continue;
    for (const m of Object.values(c.world.entities)) {
      if (m.status !== "active" || m.owner === side || !m.munition || weaponSpec(m.munition.weaponId).kind !== "ballistic") continue;
      if (c.world.memo[`warned:${side}:${m.id}`] == null) {
        c.world.memo[`warned:${side}:${m.id}`] = c.time;
        const left = Math.max(0, (m.munition.impactAt ?? c.time) - c.time);
        c.log({ side, domain: "space", kind: "missileWarning", text: `launch warning: a ballistic missile, ${Math.round(left)} s to impact near ${m.munition.targetId ?? "its aim point"}`, ids: [m.id], at: m.munition.aimAt });
      }
      c.world.tracks[side][m.id] = { id: m.id, truthId: m.id, at: m.pos, altM: m.altM, seenAt: c.time, quality: 10, classification: "class", thought: "ballistic missile", domain: "munition", by: warner.id, errorM: 2000 };
    }
  }
}

// ── Scenario events and counterspace ─────────────────────────────────────

onScheduled("scenarioEvent", (c, e) => {
  const d = e.data ?? {};
  const side = d.side as Side;
  const until = d.durationS != null ? c.time + Number(d.durationS) : Number.MAX_SAFE_INTEGER;
  switch (d.kind) {
    case "cutCommercialImagery":
      c.world.outages[`imagery:${side}`] = until;
      c.log({ side, domain: "space", kind: "event", text: `the commercial imagery provider cuts ${side} off` });
      break;
    case "cutLeoSatcom":
      c.world.outages[`satcomLeo:${side}`] = until;
      c.log({ side, domain: "space", kind: "event", text: `the LEO SATCOM provider cuts ${side}'s terminals off` });
      break;
    case "jamSatcom":
      c.world.outages[`satcomMil:${side}`] = until;
      c.log({ side, domain: "space", kind: "event", text: `counterspace jamming of ${side}'s military SATCOM uplinks` });
      break;
    case "jamGnss":
      c.world.pnt = { ...c.world.pnt, [side]: { global: Number(d.quality ?? 0.3) } };
      if (d.durationS != null) c.schedule(until, "pntRestore", side);
      c.log({ side, domain: "space", kind: "event", text: `counterspace jamming: ${side}'s GNSS falls to ${Math.round(Number(d.quality ?? 0.3) * 100)}%` });
      break;
    case "dazzleEo":
      if (d.target) c.world.outages[`dazzle:${d.target}`] = until;
      c.log({ side, domain: "space", kind: "event", text: `${d.target ?? "an imaging satellite"} is dazzled` });
      break;
    case "asat": {
      const sat = d.target ? c.entity(String(d.target)) : undefined;
      if (sat) c.setEntity(sat.id, { status: "destroyed" });
      c.log({ side, domain: "space", kind: "event", text: `a kinetic ASAT destroys ${sat?.label ?? "a satellite"}`, ids: sat ? [sat.id] : [] });
      break;
    }
    case "restore":
      for (const k of Object.keys(c.world.outages)) if (k.endsWith(`:${side}`)) delete c.world.outages[k];
      c.log({ side, domain: "space", kind: "event", text: `${side}'s services are restored` });
      break;
  }
});

onScheduled("pntRestore", (c, e) => {
  c.world.pnt = { ...c.world.pnt, [e.key]: { global: 1 } };
});

registerHandlers({
  "space.imageArea": {
    available: (c, q) => c.entity(q.agentId)?.status === "active",
    describe: (_c, q) => `image ${q.about.label} on this pass`,
    apply: (c, q) => image(c, c.entity(q.agentId)!, { lat: q.about.lat as number, lng: q.about.lng as number }),
  },
  "space.skipPass": { apply: () => {} },
});

registerRules({
  // Not worth an EO pass under thick cloud; anything else is imaged.
  spaceTasking: (_c, q, options) => (q.about.eo && (q.about.cloud as number) > 0.85 ? prefer(options, "space.skipPass") : prefer(options, "space.imageArea")),
});

registerSystem({
  name: "space",
  order: 66,
  when: (c) => c.simulated("space"),
  run(c) {
    passes(c);
    missileWarning(c);
  },
});
