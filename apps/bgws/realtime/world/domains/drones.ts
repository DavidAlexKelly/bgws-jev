// ── world/domains/drones.ts ──────────────────────────────────────────────
// Drones and counter-drone (MULTI_DOMAIN_PLAN.md W5): the transparent
// battlefield, with the kill zone emerging from drone teams, links,
// batteries and counter-drone measures — never from a table that names it.
//
// DRONE TEAMS are land units (operators in a dugout) that carry drones in
// their stores. A launched drone is an air entity with a battery, a camera,
// a warhead or none, and a CONTROL LINK:
//   rf          line of sight to the operator; fades with range, terrain and
//               jamming; its emissions give the operator away
//   fibre       cannot be jammed; the spool caps the range [ref R4, P8]
//   autonomous  terminal guidance after link loss, if the drone has it
//
// A STRIKE IS A CHAIN of chances: reach the target (link, battery,
// interceptors), acquire it (the camera, cover), hit it (the pilot's skill:
// 30–50% skilled, about 10% novice [ref R5.1, P9]), then damage, through the
// same effects as any warhead (landEffects.ts). "60–80% of strikes fail to
// destroy" [P9] is a calibration target, not an input.
//
// COUNTER-DRONE is a set of components: guns and shotguns at short range,
// interceptor drones, short-range missiles (dear), cover, hiding, scattering.

import type { LatLng } from "../../../lib/board";
import { inCover } from "../../../lib/proceduralTerrain";
import { opposing, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import { nearestCover } from "../../engine/engine";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { makeEntity } from "../create";
import { smallDronesFly } from "../environment";
import { wfig } from "../figures";
import { bearingDeg, distanceM, groundAt, offsetBy, seesBetween, step3d } from "../geo";
import { applyHit, impactOnLand } from "../landEffects";
import { damageEntity, launch } from "../munitions";
import { jamLoss } from "../net";
import { raiseQuestion } from "../questions";
import { onDetection } from "../hooks";
import { registerSystem } from "../step";
import { requestFire } from "./fires";
import type { Entity } from "../types";

const TEAMS = new Set(["generic/droneIsrTeam", "generic/fpvTeamRf", "generic/fpvTeamFibre", "generic/loiterTeam"]);
const DRONES = new Set(["generic/quadIsr", "generic/fpvRf", "generic/fpvFibre", "generic/loiter", "generic/owaDrone", "generic/owaDecoy", "generic/male"]);

const isDrone = (e: Entity) => DRONES.has(e.archetype) && e.status === "active";

/** How far a drone's link reaches from its operator, m. */
function controlRangeM(archetype: string): number {
  if (archetype === "generic/quadIsr") return wfig("world/domains/drones.ISR_LINK_M");
  if (archetype === "generic/loiter") return weaponSpec("loitering").maxRangeM;
  if (archetype === "generic/fpvFibre") return weaponSpec("fpvFibre").maxRangeM;
  return weaponSpec("fpv").maxRangeM;
}

/** The pilot's chance of hitting what the drone has acquired, by troop quality (P9). */
export function pilotSkill(quality: number): number {
  return Math.max(wfig("world/domains/drones.NOVICE_HIT"), Math.min(wfig("world/domains/drones.SKILLED_HIT"), 0.08 + quality * 0.055));
}

function dronesOf(c: WorldCtx, teamId: string): Entity[] {
  return Object.values(c.world.entities).filter((e) => e.parent === teamId && isDrone(e));
}

/** What a team knows of the enemy on land: its own sightings and its side's picture, with where. */
function knownTargets(c: WorldCtx, teamId: string): { id: string; at: LatLng }[] {
  const fe = c.fe(teamId)!;
  const out: { id: string; at: LatLng }[] = [];
  for (const id of Object.keys(c.game.forceElements).sort()) {
    const enemy = c.fe(id)!;
    if (enemy.side === fe.side || !c.landAlive(id)) continue;
    if (c.land.known(teamId, id) === "none") continue;
    const seen = c.land.units[teamId].ownSeen[id];
    out.push({ id, at: seen?.at ?? c.world.tracks[fe.side][id]?.at ?? enemy.position });
  }
  // What the side's drones and radars see but the team has not been told yet.
  for (const t of Object.values(c.world.tracks[fe.side])) {
    if (t.domain !== "land" || !t.truthId || out.some((o) => o.id === t.truthId) || !c.landAlive(t.truthId)) continue;
    if (c.time - t.seenAt > 120) continue;
    out.push({ id: t.truthId, at: t.at });
  }
  return out;
}

function value(c: WorldCtx, id: string): number {
  const fe = c.fe(id)!;
  const a = c.comps(id)?.archetype ?? "";
  return (fe.targetClass === "armoured_vehicle" ? 3 : fe.targetClass === "soft_skin" ? 2 : 1) + (/cp-|mortar|sph|mlrs|sam|shorad|ewJammer/.test(a) ? 3 : 0);
}

/** Put a drone in the air from a team. */
export function launchDrone(c: WorldCtx, teamId: string, task: Entity["task"], reason: string): Entity | undefined {
  const comps = c.comps(teamId);
  const team = c.fe(teamId);
  if (!comps || !team || comps.drones <= 0 || !hasRecord(comps.archetype)) return undefined;
  const archetype = recordFor(comps.archetype).carried.drones?.archetype;
  if (!archetype) return undefined;
  const id = c.newId(`drone-${teamId}`);
  const fibre = archetype === "generic/fpvFibre";
  const entity: Entity = {
    ...makeEntity({ id, owner: team.side, archetype, at: team.position, altM: groundAt(c.config.terrain, team.position) + 20, parent: teamId, task }),
    status: "active",
    link: { kind: fibre ? "fibre" : "rf", operatorId: teamId, quality: 1, ...(fibre ? { spoolM: weaponSpec("fpvFibre").maxRangeM } : {}) },
  };
  c.addEntity(entity);
  c.setLand(teamId, { drones: comps.drones - 1, stores: { ...comps.stores, ammo: { ...comps.stores.ammo, drone: Math.max(0, (comps.stores.ammo.drone ?? 0) - 1) } } });
  c.stat((s) => ({ ...s, spentUsd: { ...s.spentUsd, [team.side]: s.spentUsd[team.side] + entity.costUsd } }));
  c.log({ side: team.side, domain: "drones", kind: "launch", text: `${teamId} launches ${recordFor(archetype).name} (${reason})`, ids: [teamId, id], at: team.position });
  return entity;
}

// ── Teams ────────────────────────────────────────────────────────────────

function teams(c: WorldCtx): void {
  if (c.time % wfig("world/domains/drones.TEAM_CYCLE_S") !== 0) return;
  const flyable = smallDronesFly(c.world.env);
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!TEAMS.has(comps.archetype) || !c.landAlive(id)) continue;
    const unit = c.land.units[id];
    if (unit.cohesion !== "steady" || comps.emcon === "silent" || comps.drones <= 0) continue;
    if (!flyable && comps.archetype !== "generic/loiterTeam") continue;
    if (unit.lastMovedAt === c.time) continue;
    const fe = c.fe(id)!;
    const aloft = dronesOf(c, id);
    if (comps.archetype === "generic/droneIsrTeam") {
      if (aloft.length) continue;
      const near = knownTargets(c, id).sort((a, b) => distanceM(a.at, fe.position) - distanceM(b.at, fe.position))[0];
      const objective = c.game.objectives?.[fe.side];
      const watch = near && distanceM(near.at, fe.position) < controlRangeM("generic/quadIsr") * 0.8 ? near.at : objective ? offsetBy(fe.position, bearingDeg(fe.position, objective), wfig("world/domains/drones.ISR_PUSH_M")) : fe.position;
      launchDrone(c, id, { kind: "orbit", at: watch, radiusM: 300, altM: 150 }, near ? `watch ${near.id}` : "watch the front");
      continue;
    }
    // Strike teams: one target at a time each, a couple aloft, a launch every minute or so.
    if (aloft.length >= wfig("world/domains/drones.MAX_ALOFT") || c.time - c.last(`launch:${id}`) < wfig("world/domains/drones.LAUNCH_GAP_S")) continue;
    const reach = controlRangeM(recordFor(comps.archetype).carried.drones!.archetype) * 0.9;
    const handed = c.world.memo[`handoff:${id}`] != null ? Object.keys(c.world.memo).find((k) => k.startsWith(`handoffTarget:${id}:`))?.split(":")[2] : undefined;
    const taken = new Set(Object.values(c.world.entities).filter((e) => isDrone(e) && e.owner === fe.side && e.task.kind === "strike").map((e) => (e.task as { targetId?: string }).targetId));
    const candidates = knownTargets(c, id)
      .filter((t) => distanceM(t.at, fe.position) <= reach && !taken.has(t.id))
      .sort((a, b) => Number(b.id === handed) - Number(a.id === handed) || value(c, b.id) - value(c, a.id) || distanceM(a.at, fe.position) - distanceM(b.at, fe.position));
    const target = candidates[0];
    if (target) {
      if (handed) {
        delete c.world.memo[`handoff:${id}`];
        delete c.world.memo[`handoffTarget:${id}:${handed}`];
      }
      c.mark(`launch:${id}`);
      launchDrone(c, id, { kind: "strike", targetId: target.id, aimAt: target.at, altM: 100 }, `strike ${target.id}`);
    } else if (comps.archetype === "generic/loiterTeam") {
      const objective = c.game.objectives?.[fe.side];
      if (!objective) continue;
      c.mark(`launch:${id}`);
      launchDrone(c, id, { kind: "loiterSearch", at: offsetBy(fe.position, bearingDeg(fe.position, objective), Math.min(reach * 0.7, wfig("world/domains/drones.LOITER_PUSH_M"))), radiusM: 1500, until: c.time + 3000 }, "hunt the enemy's rear");
    }
  }
}

// ── Flight ───────────────────────────────────────────────────────────────

function fly(c: WorldCtx): void {
  const dt = c.config.timing.tickS;
  for (const id of Object.keys(c.world.entities).sort()) {
    const d = c.world.entities[id];
    if (!isDrone(d)) continue;
    const rec = recordFor(d.archetype);
    const speed = (rec.mover.cruiseKmh / 3.6) * (c.world.env.windMs > 8 && rec.mover.kind === "multirotor" ? 0.7 : 1);
    // Battery or fuel.
    const endurance = (d.enduranceS ?? Infinity) - dt;
    if (endurance <= 0) {
      lose(c, d, "battery flat");
      continue;
    }
    const link = updateLink(c, d);
    if (link === "lost") continue;
    let task = d.task;
    let goal: LatLng = d.pos;
    let goalAlt = d.altM;
    const ground = groundAt(c.config.terrain, d.pos);
    switch (task.kind) {
      case "orbit": {
        const there = distanceM(d.pos, task.at) <= task.radiusM;
        goal = there ? offsetBy(task.at, (d.headingDeg + 30) % 360, task.radiusM) : task.at;
        goalAlt = ground + task.altM;
        break;
      }
      case "loiterSearch":
        goal = distanceM(d.pos, task.at) <= task.radiusM ? offsetBy(task.at, (d.headingDeg + 20) % 360, task.radiusM) : task.at;
        goalAlt = ground + 300;
        if (c.time > task.until) task = { kind: "returnToBase", baseId: d.parent ?? "" };
        break;
      case "strike": {
        const t = task.targetId ? c.fe(task.targetId) : undefined;
        const alive = task.targetId != null && c.landAlive(task.targetId);
        // While it has the target in its own camera it follows it; otherwise it flies to where it was told.
        if (alive && t && distanceM(d.pos, t.position) <= rec.sensors[0]?.rangeM) goal = t.position;
        else goal = task.aimAt;
        goalAlt = distanceM(d.pos, goal) > 600 ? groundAt(c.config.terrain, goal) + (task.altM ?? 100) : groundAt(c.config.terrain, goal) + 2;
        if (distanceM(d.pos, goal) <= wfig("world/domains/drones.STRIKE_REACH_M")) {
          strike(c, d, task.targetId, goal);
          continue;
        }
        break;
      }
      case "route": {
        const next = task.waypoints[0];
        if (!next) {
          task = task.then ?? { kind: "idle" };
          break;
        }
        goal = next;
        goalAlt = task.altM ?? d.altM;
        if (distanceM(d.pos, next) < speed * dt * 1.5) task = task.waypoints.length > 1 ? { ...task, waypoints: task.waypoints.slice(1) } : (task.then ?? { kind: "idle" });
        break;
      }
      case "returnToBase": {
        const home = c.where(task.baseId);
        if (!home) {
          lose(c, d, "nowhere to land");
          continue;
        }
        goal = home;
        goalAlt = groundAt(c.config.terrain, home) + 5;
        if (distanceM(d.pos, home) <= 30) {
          recover(c, d);
          continue;
        }
        break;
      }
      default:
        break;
    }
    // A one-way drone's route ends on its target.
    if (task.kind === "idle" && (d.archetype === "generic/owaDrone" || d.archetype === "generic/owaDecoy")) {
      strike(c, d, undefined, d.pos);
      continue;
    }
    const step = step3d(d.pos, d.altM, goal, goalAlt, speed, dt);
    const flown = distanceM(d.pos, step.at);
    const spool = d.link?.spoolM != null ? d.link.spoolM - flown : undefined;
    if (spool != null && spool <= 0) {
      lose(c, d, "fibre spool run out");
      continue;
    }
    c.setEntity(id, {
      pos: step.at,
      altM: step.altM,
      headingDeg: bearingDeg(d.pos, goal),
      speedMs: speed,
      task,
      enduranceS: endurance,
      ...(spool != null ? { link: { ...d.link!, spoolM: spool } } : {}),
    });
    // Battery low on a drone meant to come back.
    const full = (rec.mover.enduranceH ?? 1) * 3600;
    if ((d.archetype === "generic/quadIsr" || d.archetype === "generic/loiter") && endurance / full < wfig("world/domains/drones.BATTERY_LOW") && c.world.memo[`battery:${id}`] == null && d.parent && c.landAlive(d.parent)) {
      c.mark(`battery:${id}`);
      raiseQuestion(c, { agentId: d.parent, side: d.owner as Side, trigger: "batteryLow", about: { droneId: id }, quality: c.fe(d.parent)!.troopQuality });
    }
  }
}

/** The control link this second: "ok", "degraded" (asked about) or "lost" (the drone is gone, or flies on alone). */
function updateLink(c: WorldCtx, d: Entity): "ok" | "degraded" | "lost" {
  const link = d.link;
  if (!link || link.kind === "autonomous" || link.kind === "fibre") return "ok";
  const op = c.fe(link.operatorId);
  if (!op || !c.landAlive(link.operatorId)) return switchOrLose(c, d, "operator lost");
  // The operator is on the air for as long as the drone flies: ESM can hear him.
  c.world.emissions.push({ emitterId: link.operatorId, side: op.side, kind: "droneLink", at: op.position, time: c.time });
  const comps = c.comps(link.operatorId);
  if (comps) c.setLand(link.operatorId, { onAirS: comps.onAirS + c.config.timing.tickS });
  const range = distanceM(op.position, d.pos);
  const reach = controlRangeM(d.archetype);
  let q = Math.max(0, 1 - (range / reach) ** 2);
  if (!seesBetween(c.config.terrain, op.position, groundAt(c.config.terrain, op.position) + 3, d.pos, d.altM, 12)) q *= wfig("world/domains/drones.NO_LOS_LINK");
  q *= 1 - jamLoss(c, "datalink", op.position, d.pos, { band: "l", resistance: wfig("world/domains/drones.LINK_JAM_RESIST"), side: op.side, ownFactor: wfig("world/domains/drones.OWN_JAM_DECONFLICT") });
  q = Math.round(q * 100) / 100;
  if (q !== link.quality) c.setEntity(d.id, { link: { ...link, quality: q } });
  if (q < wfig("world/domains/drones.LINK_LOST")) return switchOrLose(c, d, "link lost");
  if (q < wfig("world/domains/drones.LINK_POOR") && c.world.memo[`linkAsk:${d.id}`] == null) {
    c.mark(`linkAsk:${d.id}`);
    raiseQuestion(c, { agentId: link.operatorId, side: op.side, trigger: "linkDegraded", about: { droneId: d.id }, quality: op.troopQuality });
    return "degraded";
  }
  return "ok";
}

/** Link gone: a drone with terminal guidance flies on alone at what it was given; the rest fall. */
function switchOrLose(c: WorldCtx, d: Entity, why: string): "ok" | "lost" {
  if ((d.archetype === "generic/loiter" || d.archetype === "generic/owaDrone") && d.task.kind === "strike") {
    c.setEntity(d.id, { link: { ...d.link!, kind: "autonomous", quality: 0 } });
    c.log({ side: d.owner, domain: "drones", kind: "autonomous", text: `${d.label} (${d.id}) goes autonomous: ${why}`, ids: [d.id] });
    return "ok";
  }
  lose(c, d, why);
  const team = d.parent;
  if (team && c.landAlive(team) && /link/.test(why)) {
    const key = `linkLosses:${team}`;
    const recent = (c.world.memo[key] ?? 0) + 1;
    c.world.memo[key] = recent;
    if (recent >= wfig("world/domains/drones.JAMMED_LOSSES") && c.time - c.last(`jammedAsk:${team}`) > 600) {
      c.mark(`jammedAsk:${team}`);
      c.world.memo[key] = 0;
      raiseQuestion(c, { agentId: team, side: d.owner as Side, trigger: "jammedHeavily", about: { losses: recent }, quality: c.fe(team)!.troopQuality });
    }
  }
  return "lost";
}

function lose(c: WorldCtx, d: Entity, why: string): void {
  c.setEntity(d.id, { status: "destroyed" });
  c.log({ side: d.owner, domain: "drones", kind: "lost", text: `${d.label} (${d.id}) lost: ${why}`, ids: [d.id, ...(d.parent ? [d.parent] : [])], at: d.pos });
}

function recover(c: WorldCtx, d: Entity): void {
  c.setEntity(d.id, { status: "expended" });
  const team = d.parent && c.comps(d.parent);
  if (team) c.setLand(d.parent!, { drones: team.drones + 1 });
  c.log({ side: d.owner, domain: "drones", kind: "recovered", text: `${d.label} (${d.id}) back with ${d.parent}`, ids: [d.id] });
}

/** The end of a strike: acquire, then the pilot's hit, then damage. */
function strike(c: WorldCtx, d: Entity, targetId: string | undefined, at: LatLng): void {
  const rec = recordFor(d.archetype);
  const weapon = rec.weapons[0];
  c.setEntity(d.id, { status: "expended", pos: at });
  if (!weapon) return; // a decoy
  const side = d.owner === "neutral" ? "blue" : d.owner;
  const target = targetId && c.landAlive(targetId) ? c.fe(targetId) : undefined;
  let hit = true;
  if (target && weapon.guidance === "operator") {
    const covered = inCover(c.config.terrain, target.position) || c.world.memo[`hiding:${targetId}`] != null;
    const acquire = covered ? wfig("world/domains/drones.ACQUIRE_IN_COVER") : 1;
    const moving = c.land.units[targetId!]?.lastMovedAt === c.time ? wfig("world/domains/drones.MOVING_HIT") : 1;
    const jammed = d.link && d.link.kind === "rf" && d.link.quality < 0.6 ? wfig("world/domains/drones.JAMMED_HIT") : 1;
    const skill = d.link?.kind === "autonomous" ? wfig("world/domains/drones.AUTONOMOUS_HIT") : pilotSkill(c.fe(d.parent ?? "")?.troopQuality ?? 4);
    hit = c.chance(acquire) && c.chance(skill * moving * jammed);
  }
  c.stat((s) => {
    const m = s.munitions[weapon.id] ?? { fired: 0, hits: 0, kills: 0, intercepted: 0 };
    return { ...s, munitions: { ...s.munitions, [weapon.id]: { ...m, fired: m.fired + 1, hits: m.hits + (hit ? 1 : 0) } } };
  });
  if (!hit) {
    c.log({ side, domain: "drones", kind: "miss", text: `${d.label} (${d.id}) misses ${targetId ?? "its target"}`, ids: [d.id, ...(targetId ? [targetId] : [])], at });
    return;
  }
  const operator = d.parent ? c.where(d.parent) : undefined;
  if (operator) c.stat((s) => ({ ...s, droneStrikeRangesKm: [...(s.droneStrikeRangesKm ?? []), Math.round(distanceM(operator, at) / 100) / 10] }));
  const results = impactOnLand(c, target?.position ?? at, weapon, { firerSide: side, ...(target ? { targetId } : {}), label: `${rec.name} from ${d.parent ?? "far off"}`, firerAt: c.where(d.parent ?? "") ?? at });
  let killed = results.reduce((s, r) => s + r.knocked, 0);
  // A fixed site or a ship where it lands: the airbase, depot or port a one-way drone was sent at.
  for (const e of Object.values(c.world.entities)) {
    if ((e.domain !== "node" && e.domain !== "surface") || e.owner === d.owner || e.hp <= 0 || distanceM(e.pos, at) > weapon.lethalRadiusM + 150) continue;
    if (damageEntity(c, e.id, weapon, d.id).killed) killed++;
    c.stat((s) => ({ ...s, structureHits: { ...s.structureHits, [e.owner]: (s.structureHits?.[e.owner] ?? 0) + 1 } }));
  }
  if (killed > 0) {
    c.stat((s) => {
      const m = s.munitions[weapon.id];
      return { ...s, munitions: { ...s.munitions, [weapon.id]: { ...m, kills: m.kills + 1 } } };
    });
  }
  c.log({ side, domain: "drones", kind: "strike", text: `${d.label} (${d.id}) strikes ${targetId ?? "the ground"}: ${killed ? `${killed} knocked out` : "no kill"}`, ids: [d.id, ...(targetId ? [targetId] : [])], at });
}

// ── What drones see: targets found ───────────────────────────────────────

onDetection((c, o, t, track, isNew) => {
  const drone = c.entity(o.id);
  if (!drone || !isDrone(drone) || t.domain !== "land" || t.side === o.side || !drone.parent || !c.landAlive(drone.parent)) return;
  // A loitering munition that finds a vehicle dives on it.
  if (drone.task.kind === "loiterSearch" && (c.fe(t.id)?.targetClass ?? "foot") !== "foot") {
    c.setEntity(drone.id, { task: { kind: "strike", targetId: t.id, aimAt: t.at, altM: 150 } });
    c.log({ side: o.side, domain: "drones", kind: "found", text: `${drone.label} (${drone.id}) finds ${t.id} and dives`, ids: [drone.id, t.id], at: t.at });
    return;
  }
  if (drone.archetype !== "generic/quadIsr" && drone.archetype !== "generic/male") return;
  if (!isNew || c.time - c.last(`found:${o.side}:${t.id}`) < wfig("world/domains/drones.FOUND_ASK_S")) return;
  c.mark(`found:${o.side}:${t.id}`);
  raiseQuestion(c, { agentId: drone.parent, side: o.side, trigger: "targetFound", about: { droneId: drone.id, targetId: t.id, lat: track.at.lat, lng: track.at.lng, identified: track.classification === "identity" || track.classification === "type" }, quality: c.fe(drone.parent)!.troopQuality });
});

// ── Drones overhead, and counter-drone ───────────────────────────────────

function overhead(c: WorldCtx): void {
  if (c.time % 5 !== 0) return;
  const drones = Object.values(c.world.entities).filter((e) => isDrone(e) && e.altM - groundAt(c.config.terrain, e.pos) < 1000);
  if (!drones.length) return;
  for (const id of Object.keys(c.game.forceElements).sort()) {
    if (!c.landAlive(id)) continue;
    const fe = c.fe(id)!;
    const unit = c.land.units[id];
    if (unit.cohesion !== "steady") continue;
    const above = drones.find((d) => d.owner !== fe.side && d.owner !== "neutral" && distanceM(d.pos, fe.position) <= wfig("world/domains/drones.OVERHEAD_M"));
    if (!above || !c.chance(wfig("world/domains/drones.HEAR_DRONE"))) continue;
    if (c.time - c.last(`overhead:${id}`) < wfig("world/domains/drones.OVERHEAD_ASK_S")) continue;
    c.mark(`overhead:${id}`);
    raiseQuestion(c, { agentId: id, side: fe.side, trigger: "droneOverhead", about: { droneId: above.id }, quality: fe.troopQuality });
  }
}

/** Guns, shotguns, short-range missiles and interceptor drones against the drones they can see. */
function counterDrone(c: WorldCtx): void {
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id) || comps.damage.firepower || comps.weaponControl === "hold") continue;
    const cd = comps.mounts.filter((m) => ["gunAd", "shotgun", "samShort", "interceptorDrone"].includes(weaponSpec(m.weaponId).kind) && m.rounds > 0 && m.readyAt <= c.time);
    if (!cd.length) continue;
    const fe = c.fe(id)!;
    const tracks = c.world.tracks[fe.side];
    for (const mount of cd) {
      const w = weaponSpec(mount.weaponId);
      // Cheapest that will do: guns at what is close, interceptors and missiles at what is worth them.
      const target = Object.values(c.world.entities)
        .filter((e) => isDrone(e) && e.owner === opposing(fe.side) && (tracks[e.id] || distanceM(e.pos, fe.position) <= 300))
        .filter((e) => distanceM(e.pos, fe.position) <= w.maxRangeM)
        .filter((e) => w.kind !== "samShort" || e.archetype !== "generic/quadIsr" || distanceM(e.pos, fe.position) > weaponSpec("gunAd").maxRangeM)
        .filter((e) => !Object.values(c.world.entities).some((m) => m.status === "active" && m.munition?.targetId === e.id))
        .sort((a, b) => distanceM(a.pos, fe.position) - distanceM(b.pos, fe.position))[0];
      if (!target) continue;
      c.setLand(id, { mounts: c.comps(id)!.mounts.map((m) => (m === mount || m.weaponId === mount.weaponId ? { ...m, rounds: m.rounds - 1, readyAt: c.time + w.reloadS } : m)) });
      if (w.flight === "none") {
        // Guns and shotguns: resolved in the second.
        c.stat((s) => ({ ...s, interceptsTried: s.interceptsTried + 1, spentUsd: { ...s.spentUsd, [fe.side]: s.spentUsd[fe.side] + w.costUsd } }));
        if (c.chance(w.pk)) {
          c.setEntity(target.id, { status: "destroyed" });
          c.stat((s) => ({ ...s, interceptsMade: s.interceptsMade + 1, lostUsd: { ...s.lostUsd, [target.owner as Side]: s.lostUsd[target.owner as Side] + target.costUsd } }));
          c.log({ side: fe.side, domain: "drones", kind: "shotDown", text: `${id} shoots down ${target.label} (${target.id}) with ${w.id}`, ids: [id, target.id], at: target.pos });
        }
      } else {
        launch(c, { launcherId: id, owner: fe.side, weaponId: w.id, from: fe.position, fromAltM: groundAt(c.config.terrain, fe.position) + 2, targetId: target.id, aimAt: target.pos, aimAltM: target.altM });
      }
    }
  }
}

// ── Handlers ─────────────────────────────────────────────────────────────

const droneOf = (c: WorldCtx, q: { about: Record<string, unknown> }) => c.entity(q.about.droneId as string);
const targetAt = (q: { about: Record<string, unknown> }): LatLng => ({ lat: q.about.lat as number, lng: q.about.lng as number });

function nearestStrikeTeam(c: WorldCtx, side: Side, at: LatLng): string | undefined {
  return Object.keys(c.world.land)
    .filter((id) => ["generic/fpvTeamRf", "generic/fpvTeamFibre", "generic/loiterTeam"].includes(c.world.land[id].archetype) && c.landAlive(id) && c.fe(id)!.side === side && c.world.land[id].drones > 0)
    .filter((id) => distanceM(c.fe(id)!.position, at) <= controlRangeM(recordFor(c.world.land[id].archetype).carried.drones!.archetype) * 0.9)
    .sort((a, b) => distanceM(c.fe(a)!.position, at) - distanceM(c.fe(b)!.position, at))[0];
}

registerHandlers({
  "drones.reportAndWatch": {
    apply(c, q) {
      const d = droneOf(c, q);
      if (d && d.task.kind === "orbit") c.setEntity(d.id, { task: { ...d.task, at: targetAt(q) } });
    },
  },
  "drones.callFire": {
    available: (c) => c.mode("fires") !== "off",
    apply(c, q) {
      requestFire(c, { side: q.side, requestedBy: q.agentId, targetAt: targetAt(q), targetId: q.about.targetId as string, priority: 2, found: c.time, reason: "drone sighting" });
    },
  },
  "drones.handOff": {
    available: (c, q) => nearestStrikeTeam(c, q.side, targetAt(q)) != null,
    describe: (c, q) => `hand ${q.about.targetId} to ${nearestStrikeTeam(c, q.side, targetAt(q))}`,
    apply(c, q) {
      const team = nearestStrikeTeam(c, q.side, targetAt(q));
      if (!team) return;
      c.mark(`handoff:${team}`);
      c.mark(`handoffTarget:${team}:${q.about.targetId}`);
      delete c.world.memo[`launch:${team}`];
    },
  },
  "drones.strike": {
    available: (c, q) => {
      const d = droneOf(c, q);
      return d != null && hasRecord(d.archetype) && recordFor(d.archetype).weapons.length > 0;
    },
    apply(c, q) {
      const d = droneOf(c, q);
      if (d) c.setEntity(d.id, { task: { kind: "strike", targetId: q.about.targetId as string, aimAt: targetAt(q), altM: 100 } });
    },
  },
  "drones.pushOn": { apply: () => {} },
  "drones.climbForLink": {
    apply(c, q) {
      const d = droneOf(c, q);
      if (!d) return;
      // Higher, the link clears the ground between: it loses its eyes on the detail for a while.
      if (d.task.kind === "orbit") c.setEntity(d.id, { task: { ...d.task, altM: Math.min(600, d.task.altM + 250) } });
      else if (d.task.kind === "strike") c.setEntity(d.id, { task: { ...d.task, altM: (d.task.altM ?? 100) + 200 } });
      c.setEntity(d.id, { altM: d.altM + 100 });
      delete c.world.memo[`linkAsk:${d.id}`];
    },
  },
  "drones.returnHome": {
    available: (c, q) => droneOf(c, q) != null,
    apply(c, q) {
      const d = droneOf(c, q);
      if (d?.parent) c.setEntity(d.id, { task: { kind: "returnToBase", baseId: d.parent } });
    },
  },
  "drones.goAutonomous": {
    available: (c, q) => {
      const d = droneOf(c, q);
      return d != null && (d.archetype === "generic/loiter" || d.archetype === "generic/owaDrone");
    },
    apply(c, q) {
      const d = droneOf(c, q);
      if (d?.link) c.setEntity(d.id, { link: { ...d.link, kind: "autonomous" } });
    },
  },
  "drones.onePass": { apply: () => {} },
  "drones.ditchOnTarget": {
    available: (c, q) => {
      const d = droneOf(c, q);
      return d != null && recordFor(d.archetype).weapons.length > 0;
    },
    apply: () => {},
  },
  "drones.switchToFibre": {
    // Only a team that has fibre drones to switch to.
    available: (c, q) => c.comps(q.agentId)?.archetype === "generic/fpvTeamRf",
    apply(c, q) {
      // Fibre drones drawn from the brigade's stocks: fewer of them, and slower to come.
      const comps = c.comps(q.agentId)!;
      c.setLand(q.agentId, { archetype: "generic/fpvTeamFibre", drones: Math.min(comps.drones, recordFor("generic/fpvTeamFibre").carried.drones!.count) });
      c.mark(`launch:${q.agentId}`);
      c.log({ side: q.side, domain: "drones", kind: "fibre", text: `${q.agentId} switches to fibre-optic drones`, ids: [q.agentId] });
    },
  },
  "drones.relocate": {
    available: (c, q) => c.fe(q.agentId) != null,
    apply(c, q) {
      const fe = c.fe(q.agentId)!;
      const away = (bearingDeg(c.game.objectives?.[fe.side] ?? fe.position, fe.position) + c.rng.int(90) - 45 + 360) % 360;
      c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, away, wfig("world/domains/drones.RELOCATE_M")), mode: "march" });
      c.setLand(q.agentId, { onAirS: 0 });
    },
  },
  "drones.callFireOnJammer": {
    available: (c, q) => c.mode("fires") !== "off" && Object.values(c.world.fixes[q.side]).some((f) => f.kind === "jammer"),
    apply(c, q) {
      const fix = Object.values(c.world.fixes[q.side]).filter((f) => f.kind === "jammer").sort((a, b) => a.errorM - b.errorM)[0];
      if (fix) requestFire(c, { side: q.side, requestedBy: q.agentId, targetAt: fix.at, ...(c.landAlive(fix.emitterId) ? { targetId: fix.emitterId } : {}), priority: 3, found: fix.time, reason: "jammer" });
    },
  },
  "drones.wait": {
    apply(c, q) {
      c.setLand(q.agentId, { emcon: "silent" });
      c.schedule(c.time + wfig("world/domains/drones.WAIT_S"), "dronesUnsilence", q.agentId);
    },
  },
  "drones.finishThenRelocate": {
    apply(c, q) {
      c.schedule(c.time + 600, "dronesRelocate", q.agentId);
    },
  },
  "drones.stay": { apply: () => {} },
  "drones.freezeAndHide": {
    apply(c, q) {
      c.land.setUnit(q.agentId, { order: { kind: "hold" }, bound: undefined });
      c.mark(`hiding:${q.agentId}`);
      c.schedule(c.time + 300, "unhide", q.agentId);
    },
  },
  "drones.shootItDown": {
    available: (c, q) => droneOf(c, q) != null,
    apply(c, q) {
      const d = droneOf(c, q);
      const fe = c.fe(q.agentId);
      if (!d || !fe || distanceM(d.pos, fe.position) > 600) return;
      // Small arms against a drone: a poor chance, better with a shotgun or a gun.
      const best = c.comps(q.agentId)?.mounts.find((m) => ["shotgun", "gunAd"].includes(weaponSpec(m.weaponId).kind));
      const p = best ? weaponSpec(best.weaponId).pk : wfig("world/domains/drones.SMALL_ARMS_VS_DRONE");
      c.stat((s) => ({ ...s, interceptsTried: s.interceptsTried + 1 }));
      if (c.chance(p)) {
        c.setEntity(d.id, { status: "destroyed" });
        c.stat((s) => ({ ...s, interceptsMade: s.interceptsMade + 1 }));
        c.log({ side: q.side, domain: "drones", kind: "shotDown", text: `${q.agentId} shoots down ${d.label} (${d.id})`, ids: [q.agentId, d.id] });
      }
    },
  },
  "drones.scatter": {
    apply(c, q) {
      const fe = c.fe(q.agentId);
      if (fe) c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, c.rng.int(360), 200), mode: "march", dash: true });
    },
  },
  "drones.moveToCover": {
    available: (c, q) => c.fe(q.agentId) != null && !inCover(c.config.terrain, c.fe(q.agentId)!.position),
    apply(c, q) {
      const fe = c.fe(q.agentId)!;
      const cover = nearestCover(fe, c.config, 400);
      if (cover) c.land.order(q.agentId, { kind: "move", to: cover, mode: "march", dash: true });
    },
  },
  "drones.callCounterDrone": {
    available: (c, q) => counterDroneNear(c, q.side, c.where(q.agentId)!) != null,
    apply(c, q) {
      const d = droneOf(c, q);
      const team = counterDroneNear(c, q.side, c.where(q.agentId)!);
      if (!d || !team) return;
      // Tell it where the drone is: it becomes a track for that team's side.
      c.world.tracks[q.side][d.id] = { id: d.id, truthId: d.id, at: d.pos, altM: d.altM, seenAt: c.time, quality: 8, classification: "class", thought: "drone", domain: "air", by: q.agentId, errorM: 50 };
      raiseQuestion(c, { agentId: team, side: q.side, trigger: "interceptTasked", about: { droneId: d.id }, quality: c.fe(team)!.troopQuality });
    },
  },
  "drones.engageTarget": {
    expand(c, q) {
      const fe = c.fe(q.agentId);
      if (!fe) return [];
      return Object.values(c.world.entities)
        .filter((e) => isDrone(e) && e.owner === opposing(fe.side) && c.world.tracks[fe.side][e.id] && distanceM(e.pos, fe.position) <= weaponSpec("interceptorDrone").maxRangeM)
        .sort((a, b) => b.costUsd - a.costUsd)
        .slice(0, 4)
        .map((e) => ({ key: e.id, summary: `go for ${e.label} (${e.id}), ${Math.round(distanceM(e.pos, fe.position) / 100) / 10} km`, params: { droneId: e.id } }));
    },
    apply(c, q, option) {
      const fe = c.fe(q.agentId);
      const d = c.entity(option.params?.droneId as string);
      const mount = c.comps(q.agentId)?.mounts.find((m) => m.weaponId === "interceptorDrone" && m.rounds > 0);
      if (!fe || !d || !mount) return;
      c.setLand(q.agentId, { mounts: c.comps(q.agentId)!.mounts.map((m) => (m.weaponId === "interceptorDrone" ? { ...m, rounds: m.rounds - 1, readyAt: c.time + weaponSpec("interceptorDrone").reloadS } : m)) });
      launch(c, { launcherId: q.agentId, owner: fe.side, weaponId: "interceptorDrone", from: fe.position, fromAltM: groundAt(c.config.terrain, fe.position) + 2, targetId: d.id, aimAt: d.pos, aimAltM: d.altM });
    },
  },
  "drones.holdForBetter": { apply: () => {} },
});

function counterDroneNear(c: WorldCtx, side: Side, at: LatLng): string | undefined {
  return Object.keys(c.world.land)
    .filter((id) => ["generic/interceptorTeam", "generic/shorad"].includes(c.world.land[id].archetype) && c.landAlive(id) && c.fe(id)!.side === side)
    .filter((id) => distanceM(c.fe(id)!.position, at) <= weaponSpec("interceptorDrone").maxRangeM)
    .sort((a, b) => distanceM(c.fe(a)!.position, at) - distanceM(c.fe(b)!.position, at))[0];
}

registerRules({
  targetFound: (c, q, options) => {
    const target = q.about.targetId as string;
    const armoured = c.fe(target)?.targetClass === "armoured_vehicle";
    return armoured ? prefer(options, "drones.handOff", "drones.callFire", "drones.strike", "drones.reportAndWatch") : prefer(options, "drones.callFire", "drones.handOff", "drones.reportAndWatch");
  },
  linkDegraded: (c, q, options) => {
    const d = c.entity(q.about.droneId as string);
    return d?.task.kind === "strike" ? prefer(options, "drones.goAutonomous", "drones.pushOn") : prefer(options, "drones.climbForLink", "drones.returnHome");
  },
  batteryLow: (_c, _q, options) => prefer(options, "drones.returnHome", "drones.ditchOnTarget"),
  jammedHeavily: (_c, _q, options) => prefer(options, "drones.switchToFibre", "drones.callFireOnJammer", "drones.relocate", "drones.wait"),
  operatorLocated: (_c, _q, options) => prefer(options, "drones.finishThenRelocate", "drones.relocate"),
  droneOverhead: (c, q, options) => {
    const covered = inCover(c.config.terrain, c.fe(q.agentId)!.position);
    return covered ? prefer(options, "drones.freezeAndHide", "drones.callCounterDrone") : prefer(options, "drones.callCounterDrone", "drones.moveToCover", "drones.scatter");
  },
  interceptTasked: (_c, _q, options) => options.find((o) => o.action === "drones.engageTarget")?.id,
});

// The operators give themselves away the longer they fly.
function operators(c: WorldCtx): void {
  if (c.time % 30 !== 0) return;
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!TEAMS.has(comps.archetype) || !c.landAlive(id)) continue;
    const enemyFix = Object.values(c.world.fixes[opposing(c.fe(id)!.side)]).some((f) => f.emitterId === id && f.errorM < 500);
    if (comps.onAirS < wfig("world/domains/drones.OPERATOR_EXPOSED_S") && !enemyFix) continue;
    if (c.time - c.last(`opAsk:${id}`) < 900) continue;
    c.mark(`opAsk:${id}`);
    raiseQuestion(c, { agentId: id, side: c.fe(id)!.side, trigger: "operatorLocated", about: { located: enemyFix }, quality: c.fe(id)!.troopQuality });
  }
}

import { onScheduled } from "../step";
onScheduled("dronesUnsilence", (c, e) => c.comps(e.key) && c.setLand(e.key, { emcon: "free" }));
onScheduled("dronesRelocate", (c, e) => {
  const fe = c.fe(e.key);
  if (fe && c.landAlive(e.key)) {
    c.land.order(e.key, { kind: "move", to: offsetBy(fe.position, c.rng.int(360), wfig("world/domains/drones.RELOCATE_M")), mode: "march" });
    c.setLand(e.key, { onAirS: 0 });
  }
});
onScheduled("unhide", (c, e) => void delete c.world.memo[`hiding:${e.key}`]);

registerSystem({
  name: "drones",
  order: 70,
  when: (c) => c.simulated("drones"),
  run(c) {
    teams(c);
    fly(c);
    overhead(c);
    counterDrone(c);
    operators(c);
  },
});

void applyHit;
