// ── world/domains/air.ts ─────────────────────────────────────────────────
// Air and air defence (MULTI_DOMAIN_PLAN.md W7).
//
// AIRCRAFT fly as FLIGHTS of two to four, one entity each, from AIRBASES that
// generate sorties by turnaround time (slower with the runway damaged).
// Each flight carries fuel (bingo), weapons (winchester), a radar and an RWR.
//
// AIR TASKING, a light ATO: on its cycle the air component gives idle flights
// missions — CAP stations, strikes and SEAD on what the side knows of the
// enemy, CAS on call near its own front, ISR, AEW and tanker tracks.
//
// ENGAGEMENTS. Missiles are munitions in flight (munitions.ts). Their kill
// probability falls off toward maximum range (energy) and against a flight
// that defends (notch, chaff, flares); a stealthy target and a low one are
// harder for radar seekers. A pilot's belief about the threat's range comes
// from his picture, never the truth.
//
// GROUND-BASED AIR DEFENCE engages air tracks in its zone under its weapon
// control status — free, tight (identified hostile only) or hold — choosing
// the interceptor by cost and need: a $40k drone is not worth a $4M missile
// if a gun will do [ref R4.4, P11]. A radar that is on can be found and hit
// by an anti-radiation missile; one that goes silent survives but stops
// defending.
//
// CAS: a unit in contact sends a 9-line over the air–ground net to a flight
// on CAS station, which decides whether to take it. AIRSPACE: guns and
// aircraft sharing the sky are deconflicted by the fires cell, or not.

import type { LatLng } from "../../../lib/board";
import { opposing, SIDES, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import type { WeaponSpec } from "../../catalogue/schema";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { makeEntity, onCreateWorld } from "../create";
import { wfig } from "../figures";
import { bearingDeg, distanceM, groundAt, offsetBy, step3d } from "../geo";
import { registerHitModifier } from "../hooks";
import { launch } from "../munitions";
import { onDelivered, sendNet } from "../net";
import { raiseQuestion } from "../questions";
import { registerSystem } from "../step";
import type { AirMission, Entity, WorldTrack } from "../types";

const DRONE_LIKE = new Set(["generic/quadIsr", "generic/fpvRf", "generic/fpvFibre", "generic/loiter", "generic/owaDrone", "generic/owaDecoy", "generic/male"]);
export const isAircraft = (e: Entity) => e.domain === "air" && !DRONE_LIKE.has(e.archetype) && hasRecord(e.archetype);

// ── Airbases and their flights ───────────────────────────────────────────

onCreateWorld(30, (world) => {
  for (const base of Object.values(world.entities)) {
    if (base.domain !== "node" || !hasRecord(base.archetype)) continue;
    const aircraft = recordFor(base.archetype).carried.aircraft;
    if (!aircraft || Object.values(world.entities).some((e) => e.parent === base.id && e.domain === "air")) continue;
    const per = recordFor(aircraft.archetype).body.count;
    for (let i = 0; i < Math.floor(aircraft.count / per); i++) {
      const id = `${base.id}-flight-${i + 1}`;
      world.entities[id] = { ...makeEntity({ id, owner: base.owner, archetype: aircraft.archetype, at: base.pos, parent: base.id, label: `${base.label} flight ${i + 1}` }), status: "landed", air: { sorties: 0, readyAt: 0 } };
    }
  }
  for (const e of Object.values(world.entities)) if (isAircraft(e) && !e.air) world.entities[e.id] = { ...e, air: { sorties: 0, readyAt: 0 } };
});

function turnaroundS(c: WorldCtx, flight: Entity): number {
  const base = flight.parent ? c.entity(flight.parent) : undefined;
  const rotary = recordFor(flight.archetype).mover.kind === "rotary";
  const fullHp = base && hasRecord(base.archetype) ? recordFor(base.archetype).body.stayingPower : 1;
  // A damaged runway turns aircraft round slower; a destroyed one not at all.
  const damage = base ? Math.max(0.05, base.hp / fullHp) : 1;
  return (rotary ? wfig("world/domains/air.TURNAROUND_HELO_S") : wfig("world/domains/air.TURNAROUND_S")) / damage;
}

// ── Air tasking ──────────────────────────────────────────────────────────

/** What the side knows of the enemy worth an air mission: AD sites, high-value land targets, aircraft. */
function airTargets(c: WorldCtx, side: Side): { ad: WorldTrack[]; hvt: WorldTrack[]; air: WorldTrack[] } {
  const tracks = Object.values(c.world.tracks[side]).filter((t) => c.time - t.seenAt < 900);
  const hq = Object.values(c.world.hqs).filter((h) => h.side === side && !h.down);
  const cop = hq.flatMap((h) => Object.values(h.cop));
  const all = [...tracks, ...cop];
  const archetypeOf = (t: WorldTrack) => (t.truthId ? (c.comps(t.truthId)?.archetype ?? c.entity(t.truthId)?.archetype ?? "") : "");
  const ad = all.filter((t) => /sam|shorad|gbad/i.test(archetypeOf(t)));
  const hvt = all.filter((t) => /cp-|sph155|mlrs|depot|airbase|railhead|port|coastal/i.test(archetypeOf(t)));
  // Fixes on radars count as AD sites even unidentified.
  for (const f of Object.values(c.world.fixes[side])) {
    // Only radars on the ground: a fighter's radar is fixed too, but it is no site to suppress.
    const ground = c.comps(f.emitterId) != null || ["node", "surface"].includes(c.entity(f.emitterId)?.domain ?? "");
    if (ground && f.kind === "radar" && f.errorM < 2000 && !ad.some((t) => t.truthId === f.emitterId)) {
      ad.push({ id: f.emitterId, truthId: f.emitterId, at: f.at, altM: 0, seenAt: f.time, quality: 5, classification: "class", thought: "radar", domain: "land", by: "esm", errorM: f.errorM });
    }
  }
  const air = tracks.filter((t) => t.domain === "air");
  const unique = (xs: WorldTrack[]) => [...new Map(xs.map((t) => [t.truthId ?? t.id, t])).values()];
  return { ad: unique(ad), hvt: unique(hvt), air: unique(air) };
}

function canCarry(flight: Entity, kind: WeaponSpec["kind"]): boolean {
  return flight.mounts.some((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === kind);
}

export function assignMission(c: WorldCtx, flight: Entity, mission: Omit<AirMission, "tasked" | "by">): void {
  c.setEntity(flight.id, { air: { ...(flight.air ?? { sorties: 0, readyAt: 0 }), mission: { ...mission, tasked: c.time, by: "air tasking" } } });
  c.log({ side: flight.owner, domain: "air", kind: "tasked", text: `${flight.label} (${flight.id}) tasked: ${mission.kind}${mission.targetId ? ` on ${mission.targetId}` : ""}`, ids: [flight.id] });
}

function airTasking(c: WorldCtx): void {
  for (const side of SIDES) {
    if (c.time % wfig("world/domains/air.TASKING_S") !== 0 && c.time !== 1) continue;
    const idle = Object.values(c.world.entities)
      .filter((e) => isAircraft(e) && e.owner === side && !e.air?.mission && (e.status === "landed" || e.status === "active") && e.fit > 0)
      .sort((a, b) => a.id.localeCompare(b.id));
    if (!idle.length) continue;
    const { ad, hvt, air } = airTargets(c, side);
    const front = c.world.startLines[opposing(side)];
    const home = c.world.startLines[side];
    const toward = bearingDeg(home, front);
    const take = (pred: (e: Entity) => boolean) => {
      const i = idle.findIndex(pred);
      return i >= 0 ? idle.splice(i, 1)[0] : undefined;
    };
    const until = c.time + wfig("world/domains/air.SORTIE_S");
    // SEAD first on known air defence: nothing else flies safely until it is down.
    for (const site of ad.slice(0, 2)) {
      if (alreadyOn(c, side, site.truthId)) continue;
      const f = take((e) => canCarry(e, "arm"));
      if (!f) break;
      assignMission(c, f, { kind: "sead", at: site.at, radiusM: 5000, ...(site.truthId ? { targetId: site.truthId } : {}), altM: 8000, until, releaseAt: offsetBy(site.at, bearingDeg(site.at, home), weaponSpec("arm").maxRangeM * 0.6) });
    }
    // CAP over the front when the enemy flies.
    if (air.length || Object.values(c.world.entities).some((e) => isAircraft(e) && e.owner === opposing(side))) {
      const f = take((e) => canCarry(e, "aam"));
      if (f) assignMission(c, f, { kind: "cap", at: offsetBy(home, toward, distanceM(home, front) * 0.4), radiusM: 15_000, altM: 9000, until });
    }
    // Strikes on high-value targets, from stand-off where the weapon allows.
    for (const t of hvt.slice(0, 2)) {
      if (alreadyOn(c, side, t.truthId)) continue;
      const f = take((e) => canCarry(e, "glideBomb") || canCarry(e, "agm"));
      if (!f) break;
      const glide = canCarry(f, "glideBomb");
      const range = glide ? weaponSpec("glideBomb").maxRangeM * 0.8 : weaponSpec("agm").maxRangeM * 0.8;
      assignMission(c, f, { kind: "strike", at: t.at, radiusM: 2000, ...(t.truthId ? { targetId: t.truthId } : {}), altM: glide ? 9000 : 300, until, releaseAt: offsetBy(t.at, bearingDeg(t.at, home), range) });
    }
    // CAS on call over the land fight.
    if (c.simulated("land") && Object.keys(c.game.forceElements).some((id) => c.fe(id)!.side === side && c.landAlive(id))) {
      const f = take((e) => canCarry(e, "agm") || canCarry(e, "glideBomb"));
      if (f) assignMission(c, f, { kind: "cas", at: offsetBy(home, toward, distanceM(home, front) * 0.3), radiusM: 8000, altM: recordFor(f.archetype).mover.kind === "rotary" ? 150 : 6000, until });
    }
    // AEW, tankers and maritime patrol to their tracks.
    for (const kind of ["aew", "tanker", "asw"] as const) {
      const f = take((e) => (kind === "aew" ? e.archetype === "generic/aew" : kind === "tanker" ? e.archetype === "generic/tanker" : e.archetype === "generic/mpa"));
      if (f) assignMission(c, f, { kind, at: offsetBy(home, toward, distanceM(home, front) * (kind === "asw" ? 0.6 : 0.15)), radiusM: 20_000, altM: kind === "asw" ? 1500 : 9000, until: c.time + wfig("world/domains/air.SORTIE_S") * 2 });
    }
  }
}

function alreadyOn(c: WorldCtx, side: Side, targetId: string | undefined): boolean {
  return targetId != null && Object.values(c.world.entities).some((e) => e.owner === side && e.air?.mission?.targetId === targetId);
}

// ── Flight ───────────────────────────────────────────────────────────────

function flights(c: WorldCtx): void {
  const dt = c.config.timing.tickS;
  for (const id of Object.keys(c.world.entities).sort()) {
    const f = c.world.entities[id];
    if (!isAircraft(f) || f.status === "destroyed" || f.status === "expended") continue;
    const rec = recordFor(f.archetype);
    const air = f.air ?? { sorties: 0, readyAt: 0 };
    const mission = air.mission;
    if (f.status === "landed") {
      if (!mission || air.readyAt > c.time) continue;
      const base = f.parent ? c.entity(f.parent) : undefined;
      if (base && base.hp <= 0) continue; // no runway
      c.setEntity(id, { status: "active", altM: (base?.altM ?? 0) + 50, pos: base?.pos ?? f.pos });
      c.log({ side: f.owner, domain: "air", kind: "takeoff", text: `${f.label} (${id}) takes off: ${mission.kind}`, ids: [id] });
      continue;
    }
    // Fuel, bingo.
    const fuel = Math.max(0, f.stores.fuelL - rec.stores.burnLph.moving * (dt / 3600));
    const share = fuel / Math.max(1, rec.stores.fuelL);
    if (fuel <= 0) {
      c.setEntity(id, { status: "destroyed" });
      c.log({ side: f.owner, domain: "air", kind: "lost", text: `${f.label} (${id}) out of fuel: lost`, ids: [id], at: f.pos });
      continue;
    }
    const rtb = f.task.kind === "returnToBase";
    if (!rtb && share < (f.bingo ?? 0.3) && c.world.memo[`bingo:${id}:${air.sorties}`] == null) {
      c.world.memo[`bingo:${id}:${air.sorties}`] = c.time;
      raiseQuestion(c, { agentId: id, side: f.owner as Side, trigger: "bingoFuel", about: { fuelPct: Math.round(share * 100) }, quality: rec.crewQuality });
    }
    const speed = rec.mover.cruiseKmh / 3.6;
    let goal = f.pos;
    let goalAlt = f.altM;
    let task = f.task;
    if (rtb) {
      const base = c.entity((task as { baseId: string }).baseId);
      if (!base || base.hp <= 0) {
        // No base to land at: a diversion, abstracted as the nearest friendly base.
        const other = Object.values(c.world.entities).find((e) => e.domain === "node" && e.owner === f.owner && e.hp > 0 && hasRecord(e.archetype) && recordFor(e.archetype).carried.aircraft);
        if (!other) {
          c.setEntity(id, { status: "destroyed" });
          continue;
        }
        task = { kind: "returnToBase", baseId: other.id };
        c.setEntity(id, { parent: other.id });
      }
      const home = c.entity((task as { baseId: string }).baseId)!;
      goal = home.pos;
      goalAlt = home.altM + 300;
      if (distanceM(f.pos, home.pos) < speed * dt * 2) {
        land(c, f, home.id);
        continue;
      }
    } else if (mission) {
      const target = mission.targetId ? (c.where(mission.targetId) ?? mission.at) : mission.at;
      const release = mission.releaseAt;
      const strike = mission.kind === "strike" || mission.kind === "sead";
      goal = strike && release && !air.winchester ? release : strike ? target : mission.at;
      // A detour round a threat ring is flown first.
      if (mission.via && !air.winchester) {
        if (distanceM(f.pos, mission.via) < speed * dt * 2) c.setEntity(id, { air: { ...air, mission: { ...mission, via: undefined } } });
        else goal = mission.via;
      }
      goalAlt = (mission.altM ?? 6000) + (rec.mover.kind === "rotary" ? 0 : 0);
      // On station: orbit.
      if (!strike && distanceM(f.pos, mission.at) <= mission.radiusM) goal = offsetBy(mission.at, (f.headingDeg + 15) % 360, mission.radiusM * 0.8);
      if (strike && !air.winchester && distanceM(f.pos, release ?? target) < speed * dt * 2 + 500) release$(c, f, mission);
      if (c.time >= mission.until || (strike && f.air?.winchester)) task = { kind: "returnToBase", baseId: f.parent ?? "" };
      else task = { kind: "route", waypoints: [goal], altM: goalAlt };
      // A route that crosses a known threat ring: asked once per mission.
      if (!rtb && !strike && c.world.memo[`ring:${id}:${mission.tasked}`] == null) threatRing(c, f, mission);
      if (strike && c.world.memo[`ring:${id}:${mission.tasked}`] == null) threatRing(c, f, mission);
    } else {
      task = { kind: "returnToBase", baseId: f.parent ?? "" };
    }
    const lowAlt = c.world.memo[`low:${id}`] != null ? groundAt(c.config.terrain, f.pos) + 150 : goalAlt;
    const step = step3d(f.pos, f.altM, goal, Math.min(goalAlt, lowAlt), speed, dt);
    c.setEntity(id, { pos: step.at, altM: step.altM, headingDeg: bearingDeg(f.pos, goal), speedMs: speed, stores: { ...f.stores, fuelL: fuel }, task });
  }
}

function land(c: WorldCtx, f: Entity, baseId: string): void {
  const rec = recordFor(f.archetype);
  const air = f.air ?? { sorties: 0, readyAt: 0 };
  c.setEntity(f.id, {
    status: "landed",
    pos: c.entity(baseId)!.pos,
    altM: c.entity(baseId)!.altM,
    task: { kind: "landed", baseId, readyAt: c.time + turnaroundS(c, f) },
    stores: { ...f.stores, fuelL: rec.stores.fuelL },
    mounts: rec.weapons.map((w) => ({ weaponId: w.id, rounds: w.magazine, readyAt: 0 })),
    air: { sorties: air.sorties + 1, readyAt: c.time + turnaroundS(c, f) },
  });
  delete c.world.memo[`low:${f.id}`];
  c.log({ side: f.owner, domain: "air", kind: "landed", text: `${f.label} (${f.id}) lands; ready again in ${Math.round(turnaroundS(c, f) / 60)} min`, ids: [f.id] });
}

/** Release stand-off weapons at the target: ARMs at a radar, glide bombs or missiles at a land target. */
function release$(c: WorldCtx, f: Entity, mission: AirMission): void {
  const target = mission.targetId ? (c.where(mission.targetId) ?? mission.at) : mission.at;
  const kinds: WeaponSpec["kind"][] = mission.kind === "sead" ? ["arm"] : ["glideBomb", "agm", "arm"];
  const mount = f.mounts.find((m) => m.rounds > 0 && kinds.includes(weaponSpec(m.weaponId).kind) && distanceM(f.pos, target) <= weaponSpec(m.weaponId).maxRangeM);
  if (!mount) return;
  const w = weaponSpec(mount.weaponId);
  const shots = Math.min(mount.rounds, Math.max(1, f.fit) * (w.kind === "arm" ? 1 : 2));
  for (let i = 0; i < shots; i++) {
    launch(c, { launcherId: f.id, owner: f.owner as Side, weaponId: w.id, from: f.pos, fromAltM: f.altM, ...(mission.targetId ? { targetId: mission.targetId } : {}), aimAt: target });
  }
  c.setEntity(f.id, { mounts: f.mounts.map((m) => (m === mount ? { ...m, rounds: m.rounds - shots } : m)), air: { ...f.air!, winchester: true }, lastFiredAt: c.time });
  c.log({ side: f.owner, domain: "air", kind: "release", text: `${f.label} (${f.id}) releases ${shots} ${w.id} at ${mission.targetId ?? "the target"} from ${Math.round(distanceM(f.pos, target) / 1000)} km`, ids: [f.id, ...(mission.targetId ? [mission.targetId] : [])], at: f.pos });
  // An ARM homing on a radar: the radar's crew may see it coming.
  if (w.kind === "arm" && mission.targetId && c.landAlive(mission.targetId) && c.chance(wfig("world/domains/air.SEE_ARM"))) {
    raiseQuestion(c, { agentId: mission.targetId, side: opposing(f.owner as Side), trigger: "radarTargetedByArm", about: { by: f.id }, quality: c.fe(mission.targetId)!.troopQuality });
  }
}

/** Engagement radius a side believes an air-defence track has, by what it thinks it is. */
function believedRingM(c: WorldCtx, t: WorldTrack): number {
  const a = t.truthId ? (c.comps(t.truthId)?.archetype ?? c.entity(t.truthId)?.archetype ?? "") : "";
  // Belief from classification only: an unidentified radar is taken as the worst of its kind.
  if (t.classification === "track" || t.classification === "class") return /samLong/.test(a) ? 150_000 : 40_000;
  if (/samLong/.test(a)) return 150_000;
  if (/samMedium/.test(a)) return 40_000;
  return 8000;
}

function threatRing(c: WorldCtx, f: Entity, mission: AirMission): void {
  const side = f.owner as Side;
  const { ad } = airTargets(c, side);
  const target = mission.at;
  for (const site of ad) {
    if (site.truthId === mission.targetId) continue;
    const ring = believedRingM(c, site);
    // Distance from the site to the straight path ahead.
    const along = distanceM(f.pos, target);
    if (along < 1) continue;
    const mid = { lat: (f.pos.lat + target.lat) / 2, lng: (f.pos.lng + target.lng) / 2 };
    const closest = Math.min(distanceM(site.at, f.pos), distanceM(site.at, target), distanceM(site.at, mid));
    if (closest > ring) continue;
    c.world.memo[`ring:${f.id}:${mission.tasked}`] = c.time;
    raiseQuestion(c, { agentId: f.id, side, trigger: "threatRingAhead", about: { siteId: site.truthId ?? site.id, lat: site.at.lat, lng: site.at.lng, ringM: ring }, quality: recordFor(f.archetype).crewQuality });
    return;
  }
}

// ── Air-to-air and missile warnings ──────────────────────────────────────

function airToAir(c: WorldCtx): void {
  for (const id of Object.keys(c.world.entities).sort()) {
    const f = c.world.entities[id];
    if (!isAircraft(f) || f.status !== "active" || f.weaponControl === "hold") continue;
    const aam = f.mounts.filter((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === "aam");
    if (!aam.length) continue;
    const reach = Math.max(...aam.map((m) => weaponSpec(m.weaponId).maxRangeM));
    for (const t of Object.values(c.world.tracks[f.owner as Side])) {
      if (t.domain !== "air" || !t.truthId || c.time - t.seenAt > 10) continue;
      const truth = c.entity(t.truthId);
      if (!truth || truth.status !== "active") continue;
      const range = distanceM(f.pos, t.at);
      if (range > reach) continue;
      if (c.time - c.last(`wez:${id}:${t.truthId}`) < wfig("world/domains/air.WEZ_ASK_S")) continue;
      if (Object.values(c.world.entities).some((m) => m.status === "active" && m.munition?.targetId === t.truthId && m.munition?.launcherId === id)) continue;
      c.mark(`wez:${id}:${t.truthId}`);
      raiseQuestion(c, { agentId: id, side: f.owner as Side, trigger: "targetInWez", about: { targetId: t.truthId, identified: identifiedHostile(c, f, t), rangeKm: Math.round(range / 1000), inRange: true }, quality: recordFor(f.archetype).crewQuality });
    }
  }
}

/**
 * Hostile by electronic means, as tight rules allow: a typed or identified
 * track, or a classified air track that does not answer IFF while the side's
 * air picture (an AEW, or its own radar close in) holds it.
 */
function identifiedHostile(c: WorldCtx, f: Entity, t: WorldTrack): boolean {
  if (t.classification === "identity" || t.classification === "type") return true;
  if (t.classification !== "class" || !t.truthId) return false;
  const truth = c.entity(t.truthId);
  const answersIff = truth?.owner === f.owner;
  const aew = Object.values(c.world.entities).some((e) => e.owner === f.owner && e.archetype === "generic/aew" && e.status === "active");
  return !answersIff && (aew || distanceM(f.pos, t.at) <= wfig("world/domains/air.NCTR_M"));
}

function missileWarnings(c: WorldCtx): void {
  for (const m of Object.values(c.world.entities)) {
    // Launched this second or the last (air defence fires after this check runs), warned once.
    if (m.domain !== "munition" || m.status !== "active" || !m.munition?.targetId || c.time - m.munition.launchedAt > 1 || c.world.memo[`warned:${m.id}`] != null) continue;
    c.world.memo[`warned:${m.id}`] = c.time;
    const target = c.entity(m.munition.targetId);
    if (!target || !isAircraft(target) || target.status !== "active") continue;
    const w = weaponSpec(m.munition.weaponId);
    const rec = recordFor(target.archetype);
    // An RWR hears a radar-guided missile's seeker or the guiding radar; an IR missile needs a missile-approach warner.
    const warned = rec.sensors.some((s) => s.kind === "rwr") && (w.guidance === "radar" || w.guidance === "activeRadar");
    if (!warned && !c.chance(wfig("world/domains/air.SEE_IR_MISSILE"))) continue;
    raiseQuestion(c, { agentId: target.id, side: target.owner as Side, trigger: "missileWarning", about: { missileId: m.id, from: m.munition.launcherId }, quality: rec.crewQuality });
  }
}

/** Kill probability against aircraft and missiles: energy at range, a defending flight, stealth, low flyers. */
registerHitModifier((c, munition, weapon, targetId) => {
  const target = c.entity(targetId);
  if (!target || (target.domain !== "air" && target.domain !== "munition")) return 1;
  let p = 1;
  if (["aam", "samLong", "samMedium", "samShort"].includes(weapon.kind) && munition.munition) {
    const launchRange = distanceM(munition.munition.from, munition.munition.aimAt);
    p *= 1 - wfig("world/domains/air.ENERGY_FALLOFF") * (launchRange / weapon.maxRangeM) ** 2;
  }
  if ((target.air?.defendingUntil ?? -Infinity) >= c.time) p *= wfig("world/domains/air.DEFEND_FACTOR");
  const rcs = hasRecord(target.archetype) ? recordFor(target.archetype).signatures.rcsM2 : 0.1;
  if ((weapon.guidance === "radar" || weapon.guidance === "activeRadar") && rcs < 0.01) p *= wfig("world/domains/air.STEALTH_FACTOR");
  if ((weapon.guidance === "radar" || weapon.guidance === "activeRadar") && target.altM - groundAt(c.config.terrain, target.pos) < 300) p *= wfig("world/domains/air.LOW_FACTOR");
  return Math.max(0, p);
});

// ── Ground-based air defence ─────────────────────────────────────────────

/** Air-defence weapons a shooter (land unit or ship) has ready, that reach this point. */
export function adMounts(c: WorldCtx, shooterId: string, at: LatLng): { weapon: WeaponSpec; rounds: number }[] {
  const mounts = c.comps(shooterId)?.mounts ?? c.entity(shooterId)?.mounts ?? [];
  const from = c.where(shooterId);
  if (!from) return [];
  const range = distanceM(from, at);
  return mounts
    .filter((m) => m.rounds > 0 && m.readyAt <= c.time)
    .map((m) => ({ weapon: weaponSpec(m.weaponId), rounds: m.rounds }))
    .filter((m) => m.weapon.targets.includes("air") || m.weapon.targets.includes("munition"))
    .filter((m) => range >= m.weapon.minRangeM && range <= m.weapon.maxRangeM);
}

/** Fire an air-defence weapon at a track: a salvo of interceptors in flight, or a gun burst resolved now. */
export function adFire(c: WorldCtx, shooterId: string, weapon: WeaponSpec, targetId: string): void {
  const target = c.entity(targetId);
  const from = c.where(shooterId);
  const side = c.sideOf(shooterId) as Side;
  if (!target || !from) return;
  const salvo = weapon.flight === "none" ? 1 : target.domain === "air" && !DRONE_LIKE.has(target.archetype) ? weapon.salvo : 1;
  const mounts = c.comps(shooterId)?.mounts ?? c.entity(shooterId)!.mounts;
  // A ship's vertical launch cells ripple-fire: no launcher to reload between shots, only a rate (naval.ts).
  const vls = c.entity(shooterId)?.domain === "surface" && weapon.flight !== "none";
  const next = mounts.map((m) => (m.weaponId === weapon.id ? { ...m, rounds: Math.max(0, m.rounds - salvo), readyAt: vls ? c.time : c.time + weapon.reloadS } : m));
  if (vls) c.world.memo[`vls:${shooterId}:${c.time}`] = (c.world.memo[`vls:${shooterId}:${c.time}`] ?? 0) + salvo;
  const comps = c.comps(shooterId);
  if (comps) c.setLand(shooterId, { mounts: next, stores: { ...comps.stores, ammo: { ...comps.stores.ammo, [weapon.family]: Math.max(0, (comps.stores.ammo[weapon.family] ?? 0) - salvo) } } });
  else c.setEntity(shooterId, { mounts: next });
  if (weapon.flight === "none") {
    c.stat((s) => ({ ...s, interceptsTried: s.interceptsTried + 1, spentUsd: { ...s.spentUsd, [side]: s.spentUsd[side] + weapon.costUsd * salvo } }));
    if (c.chance(weapon.pk)) {
      c.setEntity(targetId, { status: "destroyed" });
      c.stat((s) => ({ ...s, interceptsMade: s.interceptsMade + 1 }));
      c.log({ side, domain: "airDefence", kind: "kill", text: `${shooterId}'s ${weapon.id} brings down ${target.label} (${targetId})`, ids: [shooterId, targetId], at: target.pos });
    }
    return;
  }
  const alt = c.entity(shooterId)?.altM ?? groundAt(c.config.terrain, from) + 5;
  for (let i = 0; i < salvo; i++) launch(c, { launcherId: shooterId, owner: side, weaponId: weapon.id, from, fromAltM: alt, targetId, aimAt: target.pos, aimAltM: target.altM });
  c.log({ side, domain: "airDefence", kind: "engage", text: `${shooterId} fires ${salvo} ${weapon.id} at ${target.label} (${targetId})`, ids: [shooterId, targetId], at: from });
}

/** Empty launchers reloaded from the unit's own stores, after the reload drill (W7). */
function reloadLaunchers(c: WorldCtx): void {
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id)) continue;
    const empty = comps.mounts.filter((m) => m.rounds === 0 && ["samShort", "samMedium", "samLong"].includes(weaponSpec(m.weaponId).kind));
    for (const m of empty) {
      const w = weaponSpec(m.weaponId);
      const left = comps.stores.ammo[w.family] ?? 0;
      if (left <= 0) continue;
      const key = `reload:${id}:${m.weaponId}`;
      if (c.world.memo[key] == null) {
        c.world.memo[key] = c.time;
        continue;
      }
      if (c.time - c.world.memo[key] < wfig("world/domains/air.AD_RELOAD_S")) continue;
      delete c.world.memo[key];
      const now = c.comps(id)!;
      c.setLand(id, { mounts: now.mounts.map((x) => (x.weaponId === m.weaponId ? { ...x, rounds: Math.min(w.magazine, left), readyAt: c.time } : x)) });
      c.log({ side: c.sideOf(id) as Side, domain: "airDefence", kind: "reload", text: `${id} reloads its ${w.id} launchers (${Math.min(w.magazine, left)} ready, ${left} left in all)`, ids: [id] });
    }
  }
}

function gbad(c: WorldCtx): void {
  const shooters = [
    ...Object.keys(c.world.land).filter((id) => c.landAlive(id) && c.world.land[id].mounts.some((m) => ["samShort", "samMedium", "samLong"].includes(weaponSpec(m.weaponId).kind))),
    ...Object.keys(c.world.entities).filter((id) => {
      const e = c.world.entities[id];
      return e.status === "active" && e.domain === "surface" && e.mounts.some((m) => ["samShort", "samMedium", "samLong", "ciws"].includes(weaponSpec(m.weaponId).kind));
    }),
  ].sort();
  for (const id of shooters) {
    const comps = c.comps(id);
    const ent = c.entity(id);
    const emcon = comps?.emcon ?? ent?.emcon ?? "free";
    const wcs = comps?.weaponControl ?? ent?.weaponControl ?? "tight";
    if (emcon === "silent" || wcs === "hold" || comps?.damage.firepower || ent?.damage.firepower) continue;
    const side = c.sideOf(id) as Side;
    for (const t of Object.values(c.world.tracks[side])) {
      if ((t.domain !== "air" && t.domain !== "munition") || !t.truthId || c.time - t.seenAt > 5) continue;
      const truth = c.entity(t.truthId);
      if (!truth || truth.status !== "active" || truth.owner === side) continue;
      // A ship alongside has its radars down; a warship's defence against missiles and drones is the naval domain's (vampireInbound).
      if (ent?.domain === "surface" && ent.speedMs < 0.5 && Object.values(c.world.entities).some((p) => p.archetype === "generic/port" && p.owner === ent.owner && distanceM(p.pos, ent.pos) < 2000)) continue;
      if (ent?.domain === "surface" && (truth.domain === "munition" || DRONE_LIKE.has(truth.archetype)) && c.simulated("naval")) continue;
      // Our own rounds in flight are not threats to engage (artillery shells, our missiles).
      if (truth.domain === "munition" && !["cruise", "ballistic", "ascm", "agm", "glideBomb", "arm", "owa"].includes(weaponSpec(truth.munition!.weaponId).kind)) continue;
      if (!adMounts(c, id, t.at).length) continue;
      if (Object.values(c.world.entities).some((m) => m.status === "active" && m.munition?.targetId === t.truthId && m.owner === side)) continue;
      if (c.time - c.last(`ad:${id}:${t.truthId}`) < wfig("world/domains/air.AD_ASK_S")) continue;
      c.mark(`ad:${id}:${t.truthId}`);
      raiseQuestion(c, {
        agentId: id,
        side,
        trigger: "trackAppears",
        about: { targetId: t.truthId, identified: t.classification === "identity" || t.classification === "type", targetCostUsd: truth.costUsd, inRange: true },
        quality: c.fe(id)?.troopQuality ?? recordFor(ent!.archetype).crewQuality,
      });
    }
  }
}

/** The cheapest weapon that will do against this target, and the surest. */
export function interceptorChoice(c: WorldCtx, shooterId: string, targetId: string): { cheapest?: WeaponSpec; surest?: WeaponSpec } {
  const target = c.entity(targetId);
  if (!target) return {};
  const options = adMounts(c, shooterId, target.pos).map((m) => m.weapon);
  if (!options.length) return {};
  const viable = options.filter((w) => w.pk >= 0.2);
  const cheap = [...viable].sort((a, b) => a.costUsd - b.costUsd)[0];
  const sure = [...options].sort((a, b) => b.pk - a.pk || a.costUsd - b.costUsd)[0];
  return { ...(cheap ? { cheapest: cheap } : {}), ...(sure ? { surest: sure } : {}) };
}

// ── CAS: 9-lines ─────────────────────────────────────────────────────────

function nineLines(c: WorldCtx): void {
  if (c.time % 15 !== 0) return;
  for (const id of Object.keys(c.game.forceElements).sort()) {
    if (!c.landAlive(id)) continue;
    const fe = c.fe(id)!;
    const unit = c.land.units[id];
    // A shaken unit calls for help too; a broken one has stopped fighting.
    if (unit.cohesion === "broken" || c.time - c.last(`nineLine:${id}`) < wfig("world/domains/air.NINE_LINE_GAP_S")) continue;
    // In contact with armour, or under fire for a while.
    const enemy = Object.keys(unit.attackers).find((e) => c.time - unit.attackers[e] < 60 && c.landAlive(e) && c.land.known(id, e) !== "none");
    if (!enemy) continue;
    const cas = Object.values(c.world.entities).find((e) => isAircraft(e) && e.owner === fe.side && e.status === "active" && e.air?.mission?.kind === "cas" && !e.air.winchester && distanceM(e.pos, fe.position) < wfig("world/domains/air.CAS_REACH_M"));
    if (!cas) continue;
    c.mark(`nineLine:${id}`);
    sendNet(c, { side: fe.side, kind: "nineLine", from: id, to: cas.id, text: `9-line from ${id}: ${enemy}`, body: { targetId: enemy, lat: c.fe(enemy)!.position.lat, lng: c.fe(enemy)!.position.lng }, pace: ["uhf", "vhf", "satcomLeo"] });
  }
}

onDelivered("nineLine", (c, m) => {
  const flight = c.entity(m.to);
  if (!flight || flight.status !== "active") return;
  raiseQuestion(c, { agentId: flight.id, side: m.side, trigger: "nineLineReceived", about: { requester: m.from, targetId: m.body.targetId as string, lat: m.body.lat as number, lng: m.body.lng as number }, quality: recordFor(flight.archetype).crewQuality });
});

// ── Handlers ─────────────────────────────────────────────────────────────

const flightOf = (c: WorldCtx, q: { agentId: string }) => c.entity(q.agentId);
function rtb(c: WorldCtx, id: string): void {
  const f = c.entity(id);
  if (f?.parent) c.setEntity(id, { task: { kind: "returnToBase", baseId: f.parent }, air: { ...f.air!, mission: undefined } });
}
function defend(c: WorldCtx, id: string): void {
  const f = c.entity(id);
  if (!f) return;
  c.setEntity(id, { air: { ...f.air!, defendingUntil: c.time + wfig("world/domains/air.DEFEND_S") }, headingDeg: (f.headingDeg + 180) % 360 });
  c.log({ side: f.owner, domain: "air", kind: "defend", text: `${f.label} (${id}) defends: notch, chaff and flares`, ids: [id], at: f.pos });
}
function goSilent(c: WorldCtx, id: string, why: string): void {
  if (c.comps(id)) c.setLand(id, { emcon: "silent" });
  else c.setEntity(id, { emcon: "silent" });
  c.schedule(c.time + wfig("world/domains/air.SILENT_S"), "radarOn", id);
  c.log({ side: c.sideOf(id) as Side, domain: "airDefence", kind: "silent", text: `${id} shuts its radar down: ${why}`, ids: [id] });
}

registerHandlers({
  "air.defend": { apply: (c, q) => defend(c, q.agentId) },
  "air.pressAttack": { apply: () => {} },
  "air.abort": {
    apply(c, q) {
      defend(c, q.agentId);
      rtb(c, q.agentId);
    },
  },
  "air.shootNow": {
    available: (c, q) => {
      const f = flightOf(c, q);
      const t = c.entity(q.about.targetId as string);
      return f != null && t != null && f.mounts.some((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === "aam" && distanceM(f.pos, t.pos) <= weaponSpec(m.weaponId).maxRangeM);
    },
    apply(c, q) {
      const f = flightOf(c, q)!;
      const t = c.entity(q.about.targetId as string);
      if (!t || t.status !== "active") return;
      const range = distanceM(f.pos, t.pos);
      // The short-range missile inside its envelope, the long-range one beyond it.
      const mount = [...f.mounts].filter((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === "aam" && range <= weaponSpec(m.weaponId).maxRangeM && range >= weaponSpec(m.weaponId).minRangeM).sort((a, b) => weaponSpec(a.weaponId).maxRangeM - weaponSpec(b.weaponId).maxRangeM)[0];
      if (!mount) return;
      launch(c, { launcherId: f.id, owner: f.owner as Side, weaponId: mount.weaponId, from: f.pos, fromAltM: f.altM, targetId: t.id, aimAt: t.pos, aimAltM: t.altM });
      c.setEntity(f.id, { mounts: f.mounts.map((m) => (m === mount ? { ...m, rounds: m.rounds - 1 } : m)), lastFiredAt: c.time });
      c.log({ side: f.owner, domain: "air", kind: "shoot", text: `${f.label} (${f.id}) fires ${mount.weaponId} at ${t.label} (${t.id}) from ${Math.round(range / 1000)} km`, ids: [f.id, t.id], at: f.pos });
    },
  },
  "air.closeForPk": {
    apply(c, q) {
      const f = flightOf(c, q);
      const t = c.entity(q.about.targetId as string);
      if (!f || !t || !f.air?.mission) return;
      c.setEntity(f.id, { air: { ...f.air, mission: { ...f.air.mission, at: t.pos, radiusM: 5000 } } });
    },
  },
  "air.holdFire": { apply: () => {} },
  "air.rtb": { apply: (c, q) => rtb(c, q.agentId) },
  "air.tanker": {
    available: (c, q) => Object.values(c.world.entities).some((e) => e.owner === q.side && e.archetype === "generic/tanker" && e.status === "active"),
    apply(c, q) {
      const f = flightOf(c, q)!;
      const tanker = Object.values(c.world.entities).find((e) => e.owner === q.side && e.archetype === "generic/tanker" && e.status === "active")!;
      // Abstracted rendezvous: the time to go there and back is spent, the fuel is full.
      const there = distanceM(f.pos, tanker.pos) / (recordFor(f.archetype).mover.cruiseKmh / 3.6);
      c.setEntity(f.id, { stores: { ...f.stores, fuelL: recordFor(f.archetype).stores.fuelL * 0.9 }, pos: tanker.pos });
      c.log({ side: q.side, domain: "air", kind: "tanker", text: `${f.label} (${f.id}) refuels from ${tanker.id} (${Math.round(there / 60)} min away)`, ids: [f.id, tanker.id] });
    },
  },
  "air.onePass": { apply: () => {} },
  "air.goAround": {
    apply(c, q) {
      const f = flightOf(c, q);
      if (!f?.air?.mission) return;
      const site = { lat: q.about.lat as number, lng: q.about.lng as number };
      const ring = q.about.ringM as number;
      const mission = f.air.mission;
      const strike = mission.kind === "strike" || mission.kind === "sead";
      if (!strike) {
        // A station inside the ring moves out of it, along the line from the site.
        if (distanceM(site, mission.at) < ring * wfig("world/domains/air.DETOUR_MARGIN")) {
          const out = offsetBy(site, bearingDeg(site, mission.at), ring * wfig("world/domains/air.DETOUR_MARGIN") + mission.radiusM);
          c.setEntity(f.id, { air: { ...f.air, mission: { ...mission, at: out } } });
        }
        return;
      }
      // A strike goes round: a waypoint off the ring's side, flown before the release point.
      const detour = offsetBy(site, (bearingDeg(site, f.pos) + 90) % 360, ring * wfig("world/domains/air.DETOUR_MARGIN"));
      c.setEntity(f.id, { air: { ...f.air, mission: { ...mission, via: detour } } });
    },
  },
  "air.goLow": { apply: (c, q) => c.mark(`low:${q.agentId}`) },
  "air.requestSead": {
    apply(c, q) {
      c.mark(`seadWanted:${q.side}:${q.about.siteId}`);
      const f = flightOf(c, q);
      if (f?.air?.mission) c.setEntity(f.id, { air: { ...f.air, mission: { ...f.air.mission, until: Math.max(c.time + 600, f.air.mission.until) } } });
    },
  },
  "air.engageCheapest": {
    available: (c, q) => interceptorChoice(c, q.agentId, q.about.targetId as string).cheapest != null,
    describe: (c, q) => {
      const w = interceptorChoice(c, q.agentId, q.about.targetId as string).cheapest;
      return w ? `engage with ${w.id} ($${Math.round(w.costUsd / 1000)}k)` : "engage";
    },
    apply(c, q) {
      const w = interceptorChoice(c, q.agentId, q.about.targetId as string).cheapest;
      if (w) adFire(c, q.agentId, w, q.about.targetId as string);
    },
  },
  "air.engageSurest": {
    available: (c, q) => interceptorChoice(c, q.agentId, q.about.targetId as string).surest != null,
    describe: (c, q) => {
      const w = interceptorChoice(c, q.agentId, q.about.targetId as string).surest;
      return w ? `engage with ${w.id} ($${Math.round(w.costUsd / 1000)}k, the surest)` : "engage";
    },
    apply(c, q) {
      const w = interceptorChoice(c, q.agentId, q.about.targetId as string).surest;
      if (w) adFire(c, q.agentId, w, q.about.targetId as string);
    },
  },
  "air.holdTight": { apply: () => {} },
  "air.goSilentLetPass": { apply: (c, q) => goSilent(c, q.agentId, "letting the raid pass") },
  "air.shutDown": { apply: (c, q) => goSilent(c, q.agentId, `an anti-radiation missile from ${q.about.by}`) },
  "air.stayUp": { apply: () => {} },
  "air.relocateRadar": {
    available: (c, q) => c.fe(q.agentId) != null,
    apply(c, q) {
      const fe = c.fe(q.agentId)!;
      c.setLand(q.agentId, { emcon: "silent" });
      c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, c.rng.int(360), 1500), mode: "march" });
      c.schedule(c.time + wfig("world/domains/air.SILENT_S"), "radarOn", q.agentId);
    },
  },
  "air.acceptNineLine": {
    apply: (c, q) => casStrike(c, q.agentId, q.about.targetId as string, 0),
  },
  "air.acceptWithDelay": {
    apply: (c, q) => casStrike(c, q.agentId, q.about.targetId as string, wfig("world/domains/air.CAS_DELAY_S")),
  },
  "air.declineNineLine": {
    apply(c, q) {
      c.log({ side: q.side, domain: "air", kind: "casDeclined", text: `${q.agentId} declines ${q.about.requester}'s 9-line`, ids: [q.agentId, q.about.requester as string] });
    },
  },
});

function casStrike(c: WorldCtx, flightId: string, targetId: string, delayS: number): void {
  const f = c.entity(flightId);
  const t = c.fe(targetId);
  if (!f?.air || !t) return;
  const glide = canCarry(f, "glideBomb") && !canCarry(f, "agm");
  const range = (glide ? weaponSpec("glideBomb").maxRangeM : weaponSpec("agm").maxRangeM) * 0.7;
  c.setEntity(f.id, { air: { ...f.air, mission: { kind: "strike", at: t.position, radiusM: 1000, targetId, altM: f.air.mission?.altM ?? 3000, until: c.time + 1800, tasked: c.time + delayS, by: "9-line", releaseAt: offsetBy(t.position, bearingDeg(t.position, f.pos), range) } } });
}

import { onScheduled } from "../step";
onScheduled("radarOn", (c, e) => {
  if (c.comps(e.key)) c.setLand(e.key, { emcon: "free" });
  else if (c.entity(e.key)) c.setEntity(e.key, { emcon: "free" });
});

registerRules({
  missileWarning: (_c, _q, options) => prefer(options, "air.defend"),
  targetInWez: (c, q, options) => {
    const f = c.entity(q.agentId);
    const free = f?.weaponControl === "free";
    if (!q.about.identified && !free) return prefer(options, "air.closeForPk", "air.holdFire");
    return prefer(options, "air.shootNow", "air.closeForPk");
  },
  bingoFuel: (_c, _q, options) => prefer(options, "air.tanker", "air.rtb"),
  threatRingAhead: (c, q, options) => {
    // Unless it is the strike's own target, go round a ring; a SEAD request if the side has SEAD.
    const sead = Object.values(c.world.entities).some((e) => e.owner === q.side && e.air?.mission?.kind === "sead");
    return sead ? prefer(options, "air.requestSead", "air.goAround") : prefer(options, "air.goAround", "air.goLow");
  },
  trackAppears: (c, q, options) => {
    // Cost-aware: a cheap target gets the cheap weapon; a dangerous one gets the sure one.
    const cost = (q.about.targetCostUsd as number) ?? 0;
    if (!q.about.identified && c.comps(q.agentId)?.weaponControl === "tight" && cost < 1_000_000) return prefer(options, "air.engageCheapest", "air.holdTight");
    return cost >= 1_000_000 ? prefer(options, "air.engageSurest", "air.engageCheapest") : prefer(options, "air.engageCheapest", "air.engageSurest");
  },
  radarTargetedByArm: (_c, _q, options) => prefer(options, "air.shutDown", "air.relocateRadar"),
  nineLineReceived: (c, q, options) => {
    const f = c.entity(q.agentId);
    const share = f && hasRecord(f.archetype) ? f.stores.fuelL / recordFor(f.archetype).stores.fuelL : 1;
    return share < 0.35 ? prefer(options, "air.declineNineLine") : prefer(options, "air.acceptNineLine");
  },
});

registerSystem({
  name: "air",
  order: 72,
  when: (c) => c.simulated("air") || c.simulated("airDefence"),
  run(c) {
    if (c.simulated("air")) {
      airTasking(c);
      flights(c);
      airToAir(c);
      nineLines(c);
    }
    reloadLaunchers(c);
    gbad(c);
    missileWarnings(c);
  },
});
