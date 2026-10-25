// ── world/domains/naval.ts ───────────────────────────────────────────────
// Naval warfare (MULTI_DOMAIN_PLAN.md W8): surface, subsurface and littoral,
// on the same clock as land and air.
//
//   MOVEMENT     ships and submarines move on the water (world/sea.ts) at
//                their cruise speed, sprinting when told; a submarine's depth
//                is bounded by the sea floor; small craft and USVs stay in
//                harbour above sea state 4. Neutral merchants sail their lanes
//                and stop sailing when ships are being hit near them.
//   AIR DEFENCE  an incoming anti-ship missile or one-way drone is put to the
//                ship it threatens (vampireInbound): the long, medium or short
//                layer, cost-aware, decoys and manoeuvre, or turn to unmask
//                more launchers. CIWS fires on its own at what gets close.
//                Ship SAM radars need EMCON lifted: a silent ship has only
//                its point defence. Cooperative engagement is the side's
//                shared track picture: any ship can fire on another's track.
//   MAGAZINES    vertical launch cells are reloaded only in port [ref R6.2].
//                Below a share of its load, a ship asks (magazineLow): leave
//                station to reload, conserve (point defence only), or ask for
//                an escort.
//   STRIKE       a surface contact is put to every ship, submarine and coastal
//                battery that can reach it (surfaceContact): engage beyond the
//                horizon on the track it has, close to identify, shadow or
//                avoid. Salvo outcomes emerge from who saw whom first, how
//                many get through the layers, and staying power [ref, Hughes].
//                An unidentified "ship" may be a neutral merchant.
//   UNDERSEA     a sonar contact (sonarContact) — attack, track, evade; a
//                submarine counter-detected by an active sonar or a torpedo in
//                the water (counterDetected) — evade, attack first, go silent.
//   LITTORAL     explosive USVs run in on ships and ports; a swarm of them is
//                put to the ship (usvSwarm): guns and helicopter, or withdraw.
//                Sea mines are areas that strike ships crossing them. A port
//                hit hard drives the fleet based there to its next port.

import type { LatLng } from "../../../lib/board";
import { opposing, SIDES, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import type { WeaponSpec } from "../../catalogue/schema";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { smallCraftOperate } from "../environment";
import { wfig } from "../figures";
import { bearingDeg, distanceM, offsetBy, towards } from "../geo";
import { onImpact, registerHitModifier } from "../hooks";
import { damageEntity, launch } from "../munitions";
import { raiseQuestion } from "../questions";
import { depthAt, isWater } from "../sea";
import { onScheduled, registerSystem } from "../step";
import { cellLoad } from "../create";
import type { Entity, WorldTrack } from "../types";
import { adFire } from "./air";

const SAM_KINDS = new Set(["samLong", "samMedium", "samShort"]);
const THREAT_KINDS = new Set(["ascm", "cruise", "ballistic", "owa", "glideBomb", "agm"]);
const isShip = (e: Entity) => e.domain === "surface" && hasRecord(e.archetype) && e.archetype !== "generic/usv";
const isWarship = (e: Entity) => isShip(e) && e.owner !== "neutral" && e.archetype !== "generic/merchant";
const isSub = (e: Entity) => e.domain === "subsurface" && hasRecord(e.archetype);
const active = (e: Entity | undefined): e is Entity => e != null && e.status === "active";

// ── Movement ─────────────────────────────────────────────────────────────

/** One second on the water towards `goal`: around the shore if the straight line runs aground. */
function sail(c: WorldCtx, e: Entity, goal: LatLng, speedMs: number): LatLng {
  const step = speedMs * c.config.timing.tickS;
  if (distanceM(e.pos, goal) < 1) return e.pos;
  const straight = towards(e.pos, goal, step);
  if (isWater(c.world, straight)) return straight;
  const head = bearingDeg(e.pos, goal);
  for (const turn of [30, -30, 60, -60, 90, -90, 135, -135]) {
    const p = offsetBy(e.pos, (head + turn + 360) % 360, step);
    if (isWater(c.world, p)) return p;
  }
  return e.pos;
}

function homePort(c: WorldCtx, e: Entity): Entity | undefined {
  const parent = e.parent ? c.entity(e.parent) : undefined;
  if (parent?.archetype === "generic/port" && parent.hp > 0 && c.world.memo[`portClosed:${parent.id}`] == null) return parent;
  return Object.values(c.world.entities)
    .filter((p) => p.archetype === "generic/port" && p.owner === e.owner && p.hp > 0 && c.world.memo[`portClosed:${p.id}`] == null)
    .sort((a, b) => distanceM(a.pos, e.pos) - distanceM(b.pos, e.pos) || a.id.localeCompare(b.id))[0];
}

function move(c: WorldCtx): void {
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (!active(e) || (e.domain !== "surface" && e.domain !== "subsurface") || !hasRecord(e.archetype)) continue;
    const rec = recordFor(e.archetype);
    const small = e.archetype === "generic/usv" || rec.body.stayingPower < 1;
    if (small && !smallCraftOperate(c.world.env)) {
      if (e.speedMs !== 0) c.setEntity(id, { speedMs: 0 });
      continue;
    }
    if (e.damage.mobility) continue;
    const sprint = c.world.memo[`sprint:${id}`] != null && c.time - c.world.memo[`sprint:${id}`] < wfig("world/domains/naval.SPRINT_S");
    const quiet = c.world.memo[`silent:${id}`] != null && c.time - c.world.memo[`silent:${id}`] < wfig("world/domains/naval.SILENT_S");
    let speed = ((sprint ? rec.mover.speedKmh : rec.mover.cruiseKmh) / 3.6) * (quiet ? 0.3 : 1);
    const task = e.task;
    let goal: LatLng | undefined;
    switch (task.kind) {
      case "route": {
        const next = task.waypoints[0];
        if (!next) break;
        goal = next;
        if (distanceM(e.pos, next) < speed * 2) c.setEntity(id, { task: task.waypoints.length > 1 ? { ...task, waypoints: task.waypoints.slice(1) } : (task.then ?? { kind: "idle" }) });
        break;
      }
      case "station":
      case "orbit": {
        const there = distanceM(e.pos, task.at) <= task.radiusM;
        goal = there ? offsetBy(task.at, (e.headingDeg + 10) % 360, task.radiusM * 0.8) : task.at;
        if (there && !sprint) speed *= 0.5;
        break;
      }
      case "strike": {
        const t = task.targetId ? c.entity(task.targetId) : undefined;
        goal = active(t) ? t.pos : task.aimAt;
        if (e.archetype === "generic/usv" && distanceM(e.pos, goal) <= wfig("world/domains/naval.USV_HIT_M")) {
          usvDetonates(c, e, task.targetId);
          continue;
        }
        break;
      }
      case "returnToBase": {
        const port = c.entity(task.baseId);
        if (!port || port.hp <= 0) break;
        goal = port.pos;
        if (distanceM(e.pos, port.pos) < wfig("world/domains/naval.IN_PORT_M")) {
          c.setEntity(id, { task: { kind: "reload", portId: port.id, readyAt: c.time + wfig("world/domains/naval.PORT_RELOAD_S") }, speedMs: 0 });
          c.log({ side: e.owner, domain: "naval", kind: "inPort", text: `${e.label} (${id}) in ${port.label}: reloading for ${Math.round(wfig("world/domains/naval.PORT_RELOAD_S") / 3600)} h`, ids: [id, port.id] });
          continue;
        }
        break;
      }
      case "reload": {
        if (c.time >= task.readyAt) reload(c, e, task.portId);
        continue;
      }
      default:
        break;
    }
    // Neutral merchants follow their lane, round and round.
    if (e.lane?.length && (task.kind === "idle" || task.kind === "route")) {
      if (task.kind === "idle") {
        if (shippingHalted(c, e)) continue;
        c.setEntity(id, { task: { kind: "route", waypoints: e.lane.slice(1) } });
        goal = e.lane[1];
      }
      const next = c.entity(id)!.task;
      if (next.kind === "idle" || (next.kind === "route" && !next.waypoints.length)) {
        c.stat((s) => ({ ...s, transits: (s.transits ?? 0) + 1 }));
        c.setEntity(id, { pos: e.lane[0], task: { kind: "idle" } });
        continue;
      }
    }
    if (!goal) {
      if (e.speedMs !== 0) c.setEntity(id, { speedMs: 0 });
      continue;
    }
    const pos = sail(c, e, goal, speed);
    // A submarine keeps off the bottom; shallower when it has to.
    const floor = -Math.max(0, depthAt(c.world, pos) - wfig("world/domains/naval.KEEL_M"));
    const altM = e.domain === "subsurface" ? Math.max(floor, Math.min(-20, c.world.memo[`deep:${id}`] != null ? -(rec.mover.ceilingM ?? 300) : e.altM)) : e.altM;
    c.setEntity(id, { pos, altM, headingDeg: bearingDeg(e.pos, goal), speedMs: distanceM(e.pos, pos) / c.config.timing.tickS });
    mines(c, c.entity(id)!);
  }
}

/** Merchants stay in port while ships are being hit near their lane. */
function shippingHalted(c: WorldCtx, e: Entity): boolean {
  const last = c.world.memo[`shipHit:${e.owner}`] ?? c.world.memo["shipHit:any"];
  return last != null && c.time - last < wfig("world/domains/naval.SHIPPING_PAUSE_S");
}

function reload(c: WorldCtx, ship: Entity, portId: string): void {
  const port = c.entity(portId);
  const ammo = { ...(port?.stores.ammo ?? {}) };
  const rec = recordFor(ship.archetype);
  const mounts = ship.mounts.map((m) => {
    const w = weaponSpec(m.weaponId);
    const want = cellLoad(rec, w) - m.rounds;
    const have = ammo[w.family] ?? 0;
    const give = Math.max(0, Math.min(want, have));
    ammo[w.family] = have - give;
    return { ...m, rounds: m.rounds + give };
  });
  if (port) c.setEntity(portId, { stores: { ...port.stores, ammo } });
  // Back to the station it left, if it left one.
  const lat = c.world.memo[`stationLat:${ship.id}`];
  const lng = c.world.memo[`stationLng:${ship.id}`];
  const station = lat != null && lng != null ? { lat, lng } : undefined;
  delete c.world.memo[`stationLat:${ship.id}`];
  delete c.world.memo[`stationLng:${ship.id}`];
  c.setEntity(ship.id, { mounts, task: station ? { kind: "station", at: station, radiusM: 10_000 } : { kind: "idle" } });
  delete c.world.memo[`magLow:${ship.id}`];
  delete c.world.memo[`conserve:${ship.id}`];
  c.log({ side: ship.owner, domain: "naval", kind: "reloaded", text: `${ship.label} (${ship.id}) reloaded: ${mounts.filter((m) => SAM_KINDS.has(weaponSpec(m.weaponId).kind)).reduce((s, m) => s + m.rounds, 0)} interceptors in its cells`, ids: [ship.id] });
}

function mines(c: WorldCtx, e: Entity): void {
  if (!c.world.obstacles.length || e.altM < -30) return;
  for (const o of c.world.obstacles) {
    if (o.kind !== "seaMines" || o.owner === e.owner) continue;
    if (distanceM(o.at, e.pos) > Math.max(o.widthM, o.depthM) / 2) continue;
    if (!c.chance(wfig("world/domains/naval.MINE_P_PER_S") * o.density)) continue;
    const r = damageEntity(c, e.id, weaponSpec("torpedo"), `mines:${o.id}`);
    shipHit(c, e);
    c.log({ side: e.owner, domain: "naval", kind: "mine", text: `${e.label} (${e.id}) strikes a mine: ${r.text}`, ids: [e.id], at: e.pos });
    if (!o.knownTo.includes(e.owner as Side) && e.owner !== "neutral") o.knownTo.push(e.owner as Side);
  }
}

function shipHit(c: WorldCtx, e: Entity): void {
  c.mark(`shipHit:${e.owner}`);
  c.mark("shipHit:any");
  if (e.owner === "neutral") c.stat((s) => ({ ...s, neutralHits: (s.neutralHits ?? 0) + 1 }));
}

// A ship hit by a missile or torpedo: shipping near it stops sailing for a while.
onImpact((c, _m, outcome) => {
  const t = outcome.targetId ? c.entity(outcome.targetId) : undefined;
  if (outcome.hit && t && (t.domain === "surface" || t.domain === "subsurface")) shipHit(c, t);
});

// ── USVs ─────────────────────────────────────────────────────────────────

function usvDetonates(c: WorldCtx, usv: Entity, targetId: string | undefined): void {
  c.setEntity(usv.id, { status: "expended" });
  const target = targetId ? c.entity(targetId) : undefined;
  if (!active(target) || distanceM(target.pos, usv.pos) > wfig("world/domains/naval.USV_HIT_M") * 2) {
    c.log({ side: usv.owner, domain: "naval", kind: "usvMiss", text: `${usv.label} (${usv.id}) explodes short of its target`, ids: [usv.id], at: usv.pos });
    return;
  }
  const r = damageEntity(c, target.id, weaponSpec("ascm"), usv.id);
  shipHit(c, target);
  c.log({ side: usv.owner, domain: "naval", kind: "usvHit", text: `${usv.label} (${usv.id}) strikes ${target.label} (${target.id}): ${r.text}`, ids: [usv.id, target.id], at: target.pos });
}

// ── Ship air defence ─────────────────────────────────────────────────────

/** An enemy weapon in flight, or a one-way drone, that threatens this ship: aimed at it or closing on it. */
function threatsTo(c: WorldCtx, ship: Entity): Entity[] {
  const side = ship.owner as Side;
  const out: Entity[] = [];
  for (const t of Object.values(c.world.tracks[side])) {
    if (!t.truthId || c.time - t.seenAt > 5) continue;
    const m = c.entity(t.truthId);
    if (!active(m) || m.owner === side) continue;
    const kind = m.munition ? weaponSpec(m.munition.weaponId).kind : m.archetype === "generic/owaDrone" || m.archetype === "generic/owaDecoy" ? "owa" : undefined;
    if (!kind || !THREAT_KINDS.has(kind)) continue;
    const aimed = m.munition?.targetId === ship.id || (m.task.kind === "strike" && m.task.targetId === ship.id);
    const closing = distanceM(m.pos, ship.pos) < wfig("world/domains/naval.THREAT_RING_M") && Math.abs(((bearingDeg(m.pos, ship.pos) - m.headingDeg + 540) % 360) - 180) < 30;
    if (aimed || closing) out.push(m);
  }
  return out.sort((a, b) => distanceM(a.pos, ship.pos) - distanceM(b.pos, ship.pos) || a.id.localeCompare(b.id));
}

const emitting = (e: Entity) => e.emcon !== "silent";

/** The ship's layers that can reach this threat now: weapon by kind. */
function layers(c: WorldCtx, ship: Entity, threat: Entity): Partial<Record<"long" | "medium" | "short", WeaponSpec>> {
  const range = distanceM(ship.pos, threat.pos);
  const out: Partial<Record<"long" | "medium" | "short", WeaponSpec>> = {};
  const conserve = c.world.memo[`conserve:${ship.id}`] != null;
  // Cells fire at a rate: so many interceptors a second, whatever the layer.
  if ((c.world.memo[`vls:${ship.id}:${c.time}`] ?? 0) >= wfig("world/domains/naval.VLS_PER_S")) return out;
  for (const m of ship.mounts) {
    if (m.rounds <= 0 || m.readyAt > c.time) continue;
    const w = weaponSpec(m.weaponId);
    if (!SAM_KINDS.has(w.kind) || range > w.maxRangeM || range < w.minRangeM) continue;
    // Radar-guided interceptors need the ship's radar on: EMCON is a hard gate.
    if ((w.guidance === "radar" || w.guidance === "activeRadar") && !emitting(ship)) continue;
    const layer = w.kind === "samLong" ? "long" : w.kind === "samMedium" ? "medium" : "short";
    if (conserve && layer !== "short") continue;
    out[layer] = w;
  }
  return out;
}

function alongside(c: WorldCtx, ship: Entity): boolean {
  return ship.speedMs < 0.5 && Object.values(c.world.entities).some((p) => p.archetype === "generic/port" && p.owner === ship.owner && distanceM(p.pos, ship.pos) < wfig("world/domains/naval.IN_PORT_M"));
}

function engaged(c: WorldCtx, side: Side, threatId: string): boolean {
  return Object.values(c.world.entities).some((m) => m.status === "active" && m.owner === side && m.munition?.targetId === threatId);
}

function shipAirDefence(c: WorldCtx): void {
  for (const id of Object.keys(c.world.entities).sort()) {
    const ship = c.world.entities[id];
    if (!active(ship) || !isWarship(ship)) continue;
    const side = ship.owner as Side;
    // The combat system takes the whole raid at once: one question for every threat not yet engaged.
    const open: Entity[] = [];
    for (const t of threatsTo(c, ship)) {
      const range = distanceM(ship.pos, t.pos);
      // CIWS: on its own, at whatever gets inside its reach — one burst at a time.
      const now = c.entity(id)!;
      const ciws = now.mounts.find((m) => weaponSpec(m.weaponId).kind === "ciws" && m.rounds > 0 && m.readyAt <= c.time);
      if (ciws && range <= weaponSpec(ciws.weaponId).maxRangeM && !ship.damage.firepower) {
        adFire(c, id, weaponSpec(ciws.weaponId), t.id);
        continue;
      }
      // Alongside, crews are ashore and radars down: only the point defence answers [declared].
      if (engaged(c, side, t.id) || ship.weaponControl === "hold" || alongside(c, ship)) continue;
      if (c.world.memo[`pd:${id}:${t.id}`] != null || (c.world.memo[`vampire:${id}:${t.id}`] ?? -Infinity) > c.time) continue;
      if (!Object.keys(layers(c, ship, t)).length) continue;
      open.push(t);
    }
    if (open.length && c.time - c.last(`vampire:${id}`) >= wfig("world/domains/naval.VAMPIRE_ASK_S")) {
      c.mark(`vampire:${id}`);
      const lead = open[0];
      const have = layers(c, ship, lead);
      raiseQuestion(c, {
        agentId: id,
        side,
        trigger: "vampireInbound",
        about: { threatId: lead.id, threats: open.map((t) => t.id).join(","), count: open.length, rangeKm: Math.round(distanceM(ship.pos, lead.pos) / 1000), threatCostUsd: Math.max(...open.map((t) => t.costUsd)), long: !!have.long, medium: !!have.medium, short: !!have.short },
        // The ship's combat system works at the speed of its doctrine, not of a crew deciding.
        quality: wfig("world/domains/naval.CIC_QUALITY"),
      });
    }
    magazine(c, ship);
  }
}

function magazine(c: WorldCtx, ship: Entity): void {
  const rec = recordFor(ship.archetype);
  const full = rec.weapons.filter((w) => SAM_KINDS.has(w.kind)).reduce((s, w) => s + cellLoad(rec, w), 0);
  if (!full) return;
  const left = ship.mounts.filter((m) => SAM_KINDS.has(weaponSpec(m.weaponId).kind)).reduce((s, m) => s + m.rounds, 0);
  if (left / full >= wfig("world/domains/naval.MAGAZINE_LOW") || c.world.memo[`magLow:${ship.id}`] != null) return;
  c.world.memo[`magLow:${ship.id}`] = c.time;
  c.log({ side: ship.owner, domain: "naval", kind: "magazineLow", text: `${ship.label} (${ship.id}) is down to ${left} of ${full} interceptors`, ids: [ship.id] });
  raiseQuestion(c, { agentId: ship.id, side: ship.owner as Side, trigger: "magazineLow", about: { left, full }, quality: rec.crewQuality });
}

/** Decoys and a hard turn: a sea-skimmer's seeker is more likely to miss. */
registerHitModifier((c, _munition, weapon, targetId) => {
  if (weapon.kind !== "ascm" && weapon.kind !== "cruise") return 1;
  const until = c.world.memo[`decoy:${targetId}`];
  return until != null && until >= c.time ? wfig("world/domains/naval.DECOY_FACTOR") : 1;
});

// ── Surface and subsurface contacts ──────────────────────────────────────

/** Who can strike a surface contact: warships, submarines and coastal batteries with anti-ship missiles. */
function strikers(c: WorldCtx, side: Side): { id: string; at: LatLng; mount: { weaponId: string; rounds: number } }[] {
  const out: { id: string; at: LatLng; mount: { weaponId: string; rounds: number } }[] = [];
  for (const e of Object.values(c.world.entities)) {
    if (!active(e) || e.owner !== side || !(isWarship(e) || isSub(e))) continue;
    const m = e.mounts.find((x) => weaponSpec(x.weaponId).kind === "ascm" && x.rounds > 0);
    if (m) out.push({ id: e.id, at: e.pos, mount: m });
  }
  for (const id of Object.keys(c.world.land)) {
    if (!c.landAlive(id) || c.fe(id)!.side !== side) continue;
    const m = c.world.land[id].mounts.find((x) => weaponSpec(x.weaponId).kind === "ascm" && x.rounds > 0);
    if (m) out.push({ id, at: c.fe(id)!.position, mount: m });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

const identifiedTrack = (t: WorldTrack) => t.classification === "type" || t.classification === "identity";

function surfaceContacts(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const side of SIDES) {
    const shooters = strikers(c, side);
    if (!shooters.length) continue;
    for (const t of Object.values(c.world.tracks[side])) {
      if (t.domain !== "surface" || !t.truthId || c.time - t.seenAt > 120) continue;
      const truth = c.entity(t.truthId);
      if (!active(truth) || truth.owner === side || truth.archetype === "generic/usv") continue;
      // A ship of ours close enough to see it identifies it by eye; an identified neutral is left alone.
      const seen = identifiedTrack(t) || Object.values(c.world.entities).some((e) => active(e) && e.owner === side && isShip(e) && distanceM(e.pos, truth.pos) <= wfig("world/domains/naval.IDENTIFY_M") * 1.1);
      if (seen && truth.owner === "neutral") continue;
      // Our own missiles already on the way: no need to ask again.
      if (Object.values(c.world.entities).some((m) => m.status === "active" && m.owner === side && m.munition?.targetId === t.truthId)) continue;
      for (const s of shooters) {
        const range = distanceM(s.at, t.at);
        if (range > weaponSpec(s.mount.weaponId).maxRangeM) continue;
        if (c.time - c.last(`surface:${s.id}:${t.truthId}`) < wfig("world/domains/naval.CONTACT_ASK_S")) continue;
        c.mark(`surface:${s.id}:${t.truthId}`);
        const quality = c.fe(s.id)?.troopQuality ?? recordFor(c.entity(s.id)!.archetype).crewQuality;
        raiseQuestion(c, { agentId: s.id, side, trigger: "surfaceContact", about: { targetId: t.truthId, rangeKm: Math.round(range / 1000), identified: seen, classification: t.classification, thought: t.thought }, quality });
        break;
      }
    }
  }
}

/** Torpedoes and active sonar: what tells a submarine it has been found. */
function undersea(c: WorldCtx): void {
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (!active(e)) continue;
    const side = e.owner as Side;
    // Sonar contacts for submarines and ASW ships.
    const hunter = isSub(e) || (isWarship(e) && e.mounts.some((m) => weaponSpec(m.weaponId).kind === "torpedo"));
    if (hunter && c.time % 10 === 0) {
      for (const t of Object.values(c.world.tracks[side])) {
        if (t.by !== "sonar" || !t.truthId || c.time - t.seenAt > 30) continue;
        const truth = c.entity(t.truthId);
        if (!active(truth) || truth.owner === side || truth.owner === "neutral") continue;
        if (c.time - c.last(`sonar:${id}:${t.truthId}`) < wfig("world/domains/naval.CONTACT_ASK_S")) continue;
        c.mark(`sonar:${id}:${t.truthId}`);
        raiseQuestion(c, { agentId: id, side, trigger: "sonarContact", about: { targetId: t.truthId, rangeKm: Math.round(distanceM(e.pos, t.at) / 1000) }, quality: recordFor(e.archetype).crewQuality });
      }
    }
    if (!isSub(e)) continue;
    const torpedo = Object.values(c.world.entities).find((m) => m.status === "active" && m.munition?.targetId === id && weaponSpec(m.munition.weaponId).kind === "torpedo");
    const pinged = Object.values(c.world.entities).find((s) => active(s) && s.owner === opposing(side) && isWarship(s) && emitting(s) && recordFor(s.archetype).sensors.some((x) => x.kind === "sonar" && x.active) && distanceM(s.pos, e.pos) < wfig("world/domains/naval.PING_HEARD_M"));
    const by = torpedo?.munition?.launcherId ?? pinged?.id;
    if (by && c.time - c.last(`counter:${id}`) > wfig("world/domains/naval.CONTACT_ASK_S")) {
      c.mark(`counter:${id}`);
      raiseQuestion(c, { agentId: id, side, trigger: "counterDetected", about: { by, torpedo: torpedo != null }, quality: recordFor(e.archetype).crewQuality });
    }
  }
}

function usvSwarms(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const id of Object.keys(c.world.entities).sort()) {
    const ship = c.world.entities[id];
    if (!active(ship) || !isWarship(ship)) continue;
    const side = ship.owner as Side;
    const boats = Object.values(c.world.tracks[side]).filter((t) => t.truthId && c.entity(t.truthId)?.archetype === "generic/usv" && active(c.entity(t.truthId)) && distanceM(t.at, ship.pos) < wfig("world/domains/naval.SWARM_M"));
    if (boats.length >= wfig("world/domains/naval.SWARM_SIZE") && c.time - c.last(`swarm:${id}`) > 120) {
      c.mark(`swarm:${id}`);
      raiseQuestion(c, { agentId: id, side, trigger: "usvSwarm", about: { count: boats.length }, quality: recordFor(ship.archetype).crewQuality });
    }
    // Guns and helicopter, once ordered, work the boats inside gun range.
    if (c.world.memo[`guns:${id}`] != null && c.time - c.world.memo[`guns:${id}`] < 1800) {
      for (const t of boats) {
        if (distanceM(t.at, ship.pos) > wfig("world/domains/naval.GUN_RANGE_M")) continue;
        if (c.chance(wfig("world/domains/naval.GUN_PK"))) {
          c.setEntity(t.truthId!, { status: "destroyed" });
          c.log({ side, domain: "naval", kind: "usvKilled", text: `${ship.label} (${id})'s guns sink ${t.truthId}`, ids: [id, t.truthId!] });
        }
      }
    }
  }
}

// ── Ports ────────────────────────────────────────────────────────────────

function ports(c: WorldCtx): void {
  for (const port of Object.values(c.world.entities)) {
    if (port.archetype !== "generic/port" || port.owner === "neutral") continue;
    const key = `portHp:${port.id}`;
    const was = c.world.memo[key];
    c.world.memo[key] = port.hp;
    if (was == null || port.hp >= was) continue;
    // Ships alongside may be hit too.
    for (const s of Object.values(c.world.entities)) {
      if (!active(s) || !isShip(s) || distanceM(s.pos, port.pos) > wfig("world/domains/naval.IN_PORT_M") || !c.chance(wfig("world/domains/naval.ALONGSIDE_HIT_P"))) continue;
      const r = damageEntity(c, s.id, weaponSpec("cruise"), `strike on ${port.id}`);
      shipHit(c, s);
      c.log({ side: s.owner, domain: "naval", kind: "hitInPort", text: `${s.label} (${s.id}) is hit alongside in ${port.label}: ${r.text}`, ids: [s.id, port.id] });
    }
    const full = recordFor(port.archetype).body.stayingPower;
    if (port.hp > full * wfig("world/domains/naval.PORT_UNTENABLE") || c.world.memo[`portClosed:${port.id}`] != null) continue;
    // Too dangerous to stay: the fleet based here moves to its next port.
    c.mark(`portClosed:${port.id}`);
    const next = Object.values(c.world.entities)
      .filter((p) => p.archetype === "generic/port" && p.owner === port.owner && p.id !== port.id && p.hp > 0 && c.world.memo[`portClosed:${p.id}`] == null)
      .sort((a, b) => distanceM(a.pos, port.pos) - distanceM(b.pos, port.pos))[0];
    const fleet = Object.values(c.world.entities).filter((s) => active(s) && (isShip(s) || isSub(s)) && s.owner === port.owner && (s.parent === port.id || distanceM(s.pos, port.pos) < wfig("world/domains/naval.IN_PORT_M")));
    for (const s of fleet) c.setEntity(s.id, { ...(next ? { parent: next.id, task: { kind: "route", waypoints: [next.pos] } } : { task: { kind: "station", at: offsetBy(port.pos, bearingDeg(c.world.startLines[opposing(port.owner as Side)], port.pos), 100_000), radiusM: 20_000 } }) });
    c.log({ side: port.owner, domain: "naval", kind: "fleetRelocates", text: `${port.label} is no longer tenable: ${fleet.length} ships ${next ? `move to ${next.label}` : "put to sea"}`, ids: [port.id, ...fleet.map((s) => s.id)] });
  }
}

// ── Handlers ─────────────────────────────────────────────────────────────

const ship = (c: WorldCtx, q: { agentId: string }) => c.entity(q.agentId);
/** A ship on its way to reload, or reloading, keeps to that: contacts do not divert it. */
const goingHome = (c: WorldCtx, id: string) => {
  const k = c.entity(id)?.task.kind;
  return k === "returnToBase" || k === "reload";
};

/** Every threat the question named, still in the air and not yet engaged. */
function threatsOf(c: WorldCtx, q: { about: Record<string, unknown> }): Entity[] {
  const ids = typeof q.about.threats === "string" ? (q.about.threats as string).split(",") : [q.about.threatId as string];
  return ids.map((id) => c.entity(id)).filter(active);
}

function engageWith(c: WorldCtx, q: { agentId: string; side: Side; about: Record<string, unknown> }, layer: "long" | "medium" | "short"): void {
  const s = ship(c, q);
  if (!active(s)) return;
  for (const t of threatsOf(c, q)) {
    if (engaged(c, q.side, t.id)) continue;
    // Down the layers if the one chosen cannot reach this one.
    const have = layers(c, c.entity(s.id)!, t);
    const w = have[layer] ?? (layer === "long" ? have.medium : layer === "medium" ? (have.long ?? have.short) : undefined);
    if (w) adFire(c, s.id, w, t.id);
  }
}

registerHandlers({
  "sea.engageLong": { available: (c, q) => !!q.about.long && threatsOf(c, q).length > 0, apply: (c, q) => engageWith(c, q, "long") },
  "sea.engageMedium": { available: (c, q) => !!q.about.medium && threatsOf(c, q).length > 0, apply: (c, q) => engageWith(c, q, "medium") },
  "sea.engageShort": {
    // Leave it to point defence: the short layer now, CIWS when it closes; not asked again about this one.
    available: (_c, q) => !!q.about.short,
    apply: (c, q) => {
      engageWith(c, q, "short");
      for (const t of threatsOf(c, q)) c.mark(`pd:${q.agentId}:${t.id}`);
    },
  },
  "sea.decoyManoeuvre": {
    apply(c, q) {
      const s = ship(c, q);
      if (!s) return;
      c.world.memo[`decoy:${s.id}`] = c.time + wfig("world/domains/naval.DECOY_S");
      // Not asked again about these threats while the decoys are out.
      for (const t of threatsOf(c, q)) c.world.memo[`vampire:${s.id}:${t.id}`] = c.time + wfig("world/domains/naval.DECOY_S");
      c.mark(`sprint:${s.id}`);
      c.log({ side: q.side, domain: "naval", kind: "decoy", text: `${s.label} (${s.id}) fires decoys and turns hard`, ids: [s.id] });
    },
  },
  "sea.unmask": {
    available: (_c, q) => [q.about.long, q.about.medium, q.about.short].filter(Boolean).length >= 2,
    apply(c, q) {
      // Both layers at once: more launchers bear.
      engageWith(c, q, q.about.long ? "long" : "medium");
      engageWith(c, q, q.about.medium && q.about.long ? "medium" : "short");
    },
  },
  "sea.leaveToReload": {
    available: (c, q) => {
      const s = ship(c, q);
      return s != null && homePort(c, s) != null;
    },
    apply(c, q) {
      const s = ship(c, q)!;
      const port = homePort(c, s)!;
      if (s.task.kind === "station") {
        c.world.memo[`stationLat:${s.id}`] = s.task.at.lat;
        c.world.memo[`stationLng:${s.id}`] = s.task.at.lng;
      }
      c.setEntity(s.id, { task: { kind: "returnToBase", baseId: port.id } });
      c.log({ side: q.side, domain: "naval", kind: "leaveStation", text: `${s.label} (${s.id}) leaves station for ${port.label} to reload, ${Math.round(distanceM(s.pos, port.pos) / 1000)} km away`, ids: [s.id, port.id] });
    },
  },
  "sea.conserve": { apply: (c, q) => c.mark(`conserve:${q.agentId}`) },
  "sea.requestEscort": {
    available: (c, q) => Object.values(c.world.entities).some((e) => active(e) && isWarship(e) && e.owner === q.side && e.id !== q.agentId),
    apply(c, q) {
      const s = ship(c, q)!;
      const escort = Object.values(c.world.entities)
        .filter((e) => active(e) && isWarship(e) && e.owner === q.side && e.id !== s.id && !goingHome(c, e.id))
        .sort((a, b) => distanceM(a.pos, s.pos) - distanceM(b.pos, s.pos))[0]!;
      c.setEntity(escort.id, { task: { kind: "station", at: s.pos, radiusM: 5000 } });
      c.log({ side: q.side, domain: "naval", kind: "escort", text: `${escort.label} (${escort.id}) closes to escort ${s.label}`, ids: [escort.id, s.id] });
    },
  },
  "sea.engageBeyondHorizon": {
    available: (c, q) => strikers(c, q.side).some((s) => s.id === q.agentId),
    apply(c, q) {
      const s = strikers(c, q.side).find((x) => x.id === q.agentId);
      const t = c.entity(q.about.targetId as string);
      const track = c.world.tracks[q.side][q.about.targetId as string];
      if (!s || !active(t)) return;
      const w = weaponSpec(s.mount.weaponId);
      const salvo = Math.min(s.mount.rounds, w.salvo * (t.archetype === "generic/destroyer" || t.archetype === "generic/frigate" ? 2 : 1));
      // Fired at the track: where the side believes the ship is; the seeker finds it from there.
      for (let i = 0; i < salvo; i++) launch(c, { launcherId: s.id, owner: q.side, weaponId: w.id, from: s.at, fromAltM: 10, targetId: t.id, aimAt: track?.at ?? t.pos });
      const spend = (mounts: { weaponId: string; rounds: number; readyAt: number }[]) => mounts.map((m) => (m.weaponId === w.id ? { ...m, rounds: m.rounds - salvo, readyAt: c.time + w.reloadS } : m));
      if (c.comps(s.id)) c.setLand(s.id, { mounts: spend(c.comps(s.id)!.mounts) });
      else c.setEntity(s.id, { mounts: spend(c.entity(s.id)!.mounts), lastFiredAt: c.time });
      c.log({ side: q.side, domain: "naval", kind: "salvo", text: `${s.id} fires a salvo of ${salvo} ${w.id} at ${q.about.thought ?? "a contact"} ${Math.round(distanceM(s.at, t.pos) / 1000)} km away`, ids: [s.id, t.id] });
    },
  },
  "sea.closeToIdentify": {
    available: (c, q) => c.entity(q.agentId) != null,
    apply(c, q) {
      if (goingHome(c, q.agentId)) return;
      const t = c.world.tracks[q.side][q.about.targetId as string];
      if (t) c.setEntity(q.agentId, { task: { kind: "route", waypoints: [offsetBy(t.at, bearingDeg(t.at, c.entity(q.agentId)!.pos), wfig("world/domains/naval.IDENTIFY_M"))] } });
    },
  },
  "sea.shadow": {
    available: (c, q) => c.entity(q.agentId) != null,
    apply(c, q) {
      if (goingHome(c, q.agentId)) return;
      const t = c.world.tracks[q.side][q.about.targetId as string];
      if (t) c.setEntity(q.agentId, { task: { kind: "station", at: offsetBy(t.at, bearingDeg(t.at, c.entity(q.agentId)!.pos), wfig("world/domains/naval.SHADOW_M")), radiusM: 5000 } });
    },
  },
  "sea.avoid": {
    available: (c, q) => c.entity(q.agentId) != null,
    apply(c, q) {
      const e = c.entity(q.agentId)!;
      const t = c.world.tracks[q.side][q.about.targetId as string];
      if (t) c.setEntity(e.id, { task: { kind: "route", waypoints: [offsetBy(e.pos, bearingDeg(t.at, e.pos), 50_000)] } });
    },
  },
  "sea.attack": {
    available: (c, q) => {
      const e = c.entity(q.agentId);
      const t = c.entity(q.about.targetId as string);
      return active(e) && active(t) && e.mounts.some((m) => weaponSpec(m.weaponId).kind === "torpedo" && m.rounds > 0);
    },
    apply(c, q) {
      const e = c.entity(q.agentId)!;
      const t = c.entity((q.about.targetId ?? q.about.by) as string);
      if (!active(t)) return;
      const mount = e.mounts.find((m) => weaponSpec(m.weaponId).kind === "torpedo" && m.rounds > 0);
      if (!mount) return;
      const w = weaponSpec(mount.weaponId);
      if (distanceM(e.pos, t.pos) > w.maxRangeM) {
        c.setEntity(e.id, { task: { kind: "strike", targetId: t.id, aimAt: t.pos } });
        return;
      }
      launch(c, { launcherId: e.id, owner: q.side, weaponId: w.id, from: e.pos, fromAltM: e.altM, targetId: t.id, aimAt: t.pos, aimAltM: t.altM });
      c.setEntity(e.id, { mounts: e.mounts.map((m) => (m === mount ? { ...m, rounds: m.rounds - 1, readyAt: c.time + w.reloadS } : m)), lastFiredAt: c.time });
      c.log({ side: q.side, domain: "naval", kind: "torpedo", text: `${e.label} (${e.id}) fires a torpedo at ${t.label} (${t.id})`, ids: [e.id, t.id] });
    },
  },
  "sea.track": {
    apply(c, q) {
      if (goingHome(c, q.agentId)) return;
      const t = c.entity(q.about.targetId as string);
      if (active(t)) c.setEntity(q.agentId, { task: { kind: "station", at: t.pos, radiusM: wfig("world/domains/naval.SHADOW_M") / 4 } });
    },
  },
  "sea.evade": {
    apply(c, q) {
      const e = c.entity(q.agentId);
      if (!e) return;
      const from = c.entity((q.about.by ?? q.about.targetId) as string)?.pos ?? e.pos;
      // Go deep, sprint away, then drift quiet.
      c.mark(`deep:${e.id}`);
      c.mark(`sprint:${e.id}`);
      c.setEntity(e.id, { task: { kind: "route", waypoints: [offsetBy(e.pos, bearingDeg(from, e.pos), 20_000)] } });
      c.schedule(c.time + wfig("world/domains/naval.SPRINT_S"), "drift", e.id);
    },
  },
  "sea.goSilent": {
    apply(c, q) {
      c.mark(`silent:${q.agentId}`);
      c.setEntity(q.agentId, { emcon: "silent" });
    },
  },
  "sea.gunsAndHelo": { apply: (c, q) => c.mark(`guns:${q.agentId}`) },
  "sea.withdraw": {
    apply(c, q) {
      const e = c.entity(q.agentId);
      if (!e) return;
      const port = homePort(c, e);
      c.mark(`sprint:${e.id}`);
      c.setEntity(e.id, { task: port ? { kind: "returnToBase", baseId: port.id } : { kind: "route", waypoints: [offsetBy(e.pos, (e.headingDeg + 180) % 360, 30_000)] } });
    },
  },
});

onScheduled("drift", (c, e) => {
  delete c.world.memo[`sprint:${e.key}`];
  c.mark(`silent:${e.key}`);
});

registerRules({
  vampireInbound: (_c, q, options) => {
    // Cost-aware layers: a cheap drone gets the cheapest layer that will do; a missile the outermost.
    const cheap = ((q.about.threatCostUsd as number) ?? 0) < 200_000;
    const close = (q.about.rangeKm as number) < 15;
    // Whatever layer is left before decoys alone: a ship lost costs more than any interceptor.
    if (cheap) return prefer(options, "sea.engageShort", "sea.engageMedium", "sea.engageLong", "sea.decoyManoeuvre");
    if (close) return prefer(options, "sea.unmask", "sea.engageMedium", "sea.engageLong", "sea.engageShort", "sea.decoyManoeuvre");
    return prefer(options, "sea.engageLong", "sea.engageMedium", "sea.engageShort", "sea.decoyManoeuvre");
  },
  magazineLow: (_c, _q, options) => prefer(options, "sea.leaveToReload", "sea.conserve"),
  surfaceContact: (c, q, options) => {
    // Hostile identified, or weapons free: engage. Otherwise find out first — it may be a merchant.
    const free = c.entity(q.agentId)?.weaponControl === "free" || c.comps(q.agentId)?.weaponControl === "free";
    if (q.about.identified || free) return prefer(options, "sea.engageBeyondHorizon", "sea.closeToIdentify");
    return prefer(options, "sea.closeToIdentify", "sea.shadow");
  },
  sonarContact: (_c, _q, options) => prefer(options, "sea.attack", "sea.track"),
  counterDetected: (_c, q, options) => (q.about.torpedo ? prefer(options, "sea.evade") : prefer(options, "sea.goSilent", "sea.evade")),
  usvSwarm: (_c, _q, options) => prefer(options, "sea.gunsAndHelo", "sea.withdraw"),
});

registerSystem({
  name: "naval",
  order: 74,
  when: (c) => c.simulated("naval"),
  run(c) {
    move(c);
    shipAirDefence(c);
    surfaceContacts(c);
    undersea(c);
    usvSwarms(c);
    ports(c);
  },
});
