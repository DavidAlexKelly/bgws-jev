// ── world/domains/landExt.ts ─────────────────────────────────────────────
// Land combined arms beyond the land engine's direct fire (MULTI_DOMAIN_PLAN.md W3):
//
//   UNIT SUSTAINMENT  fuel burnt by the hour moving and idling (the Abrams
//                     figures [ref R7, P17]), ammunition by every volley;
//                     at a threshold `stocksLow` asks the leader; at zero the
//                     capability goes — no fuel, no moving; no rounds, no
//                     firing. Only when logistics is not off: with logistics
//                     off stocks are unlimited, as every game was before.
//   OBSTACLES         minefields and ditches as areas. A minefield nobody
//                     knows of is found by striking it; one that is known
//                     stops a unit short of it to decide (breach, bypass,
//                     call engineers, wait). A breach takes about 1.5 h per
//                     6 m lane [ref R4.5, P13], longer under fire.
//   DISMOUNTS         an IFV close to the enemy asks whether to put its
//                     infantry out; the section becomes its own unit.
//   DIGGING IN        a unit that holds the same ground digs in, and its
//                     blast casualties fall as it does (landEffects.ts).

import type { LatLng } from "../../../lib/board";
import { toGameStateFromPlaced } from "../../../lib/forceBuilder";
import { inCover } from "../../../lib/proceduralTerrain";
import type { Side } from "../../../lib/state";
import { weaponFor } from "../../../rules/turnLoop";
import { hasRecord, recordFor } from "../../catalogue";
import type { AmmoFamily } from "../../catalogue/schema";
import { freshUnitFor, nearestCover } from "../../engine/engine";
import type { RtOrder } from "../../engine/types";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { landComponents } from "../create";
import { wfig } from "../figures";
import { bearingDeg, distanceM, offsetBy, seesBetween, groundAt } from "../geo";
import { applyHit } from "../landEffects";
import { raiseQuestion } from "../questions";
import { registerSystem } from "../step";
import type { Obstacle } from "../types";

// ── Obstacle geometry ────────────────────────────────────────────────────

/** A point in an obstacle's own frame: u along its front, v into its depth (towards its bearing). */
export function obstacleFrame(o: Obstacle, p: LatLng): { u: number; v: number } {
  const north = (p.lat - o.at.lat) * 111_320;
  const east = (p.lng - o.at.lng) * 111_320 * Math.cos((o.at.lat * Math.PI) / 180);
  const b = (o.bearingDeg * Math.PI) / 180;
  // v: along the bearing (depth); u: 90° to its right (along the front).
  const v = east * Math.sin(b) + north * Math.cos(b);
  const u = east * Math.cos(b) - north * Math.sin(b);
  return { u, v };
}

export function inside(o: Obstacle, p: LatLng): boolean {
  const { u, v } = obstacleFrame(o, p);
  return Math.abs(u) <= o.widthM / 2 && Math.abs(v) <= o.depthM / 2;
}

/** Back from the obstacle's frame to the map. */
function fromFrame(o: Obstacle, u: number, v: number): LatLng {
  return offsetBy(offsetBy(o.at, o.bearingDeg, v), (o.bearingDeg + 90) % 360, u);
}

/** On a cleared lane through it? */
export function onLane(o: Obstacle, p: LatLng): boolean {
  const { u } = obstacleFrame(o, p);
  return o.lanes.some((lane) => Math.abs(obstacleFrame(o, lane.at).u - u) <= lane.widthM / 2 + wfig("world/domains/landExt.LANE_SLACK_M"));
}

/** The point on the near edge of the obstacle opposite `p`, and the far edge. */
function edges(o: Obstacle, p: LatLng): { near: LatLng; far: LatLng } {
  const { u, v } = obstacleFrame(o, p);
  const uc = Math.max(-o.widthM / 2 + 10, Math.min(o.widthM / 2 - 10, u));
  const sign = v < 0 ? -1 : 1;
  return { near: fromFrame(o, uc, sign * (o.depthM / 2 + 20)), far: fromFrame(o, uc, -sign * (o.depthM / 2 + 20)) };
}

function obstacleById(c: WorldCtx, id: unknown): Obstacle | undefined {
  return c.world.obstacles.find((o) => o.id === id);
}
function setObstacle(c: WorldCtx, id: string, patch: Partial<Obstacle>): void {
  const i = c.world.obstacles.findIndex((o) => o.id === id);
  if (i >= 0) c.world.obstacles[i] = { ...c.world.obstacles[i], ...patch };
}

// ── Sustainment ──────────────────────────────────────────────────────────

const sustaining = (c: WorldCtx) => c.mode("logistics") !== "off";

/** The ammunition family a direct-fire volley draws on, and rounds per vehicle. */
function familyOf(c: WorldCtx, firerId: string, targetId: string, rangeM: number): { family: AmmoFamily; perVehicle: number } {
  const firer = c.fe(firerId)!;
  const target = c.fe(targetId);
  const cap = target ? weaponFor(firer, target, rangeM) : undefined;
  const load = c.comps(firerId)?.stores.ammo ?? {};
  if (cap?.kind === "atm" && (load.atgm ?? 0) > 0) return { family: "atgm", perVehicle: 1 };
  if (cap?.kind === "atk" && load.tank120 != null) return { family: "tank120", perVehicle: 1 };
  if (cap?.kind === "atk" && load.cannon30 != null) return { family: "cannon30", perVehicle: 6 };
  return { family: "smallArms", perVehicle: 30 };
}

function sustain(c: WorldCtx): void {
  const dt = c.config.timing.tickS;
  // Every volley the land engine fired this second.
  for (const shot of c.land.shots) {
    const comps = c.comps(shot.firerId);
    if (!comps) continue;
    const { family, perVehicle } = familyOf(c, shot.firerId, shot.targetId, shot.rangeM ?? 1000);
    const have = comps.stores.ammo[family];
    if (have == null) continue;
    c.setLand(shot.firerId, { stores: { ...comps.stores, ammo: { ...comps.stores.ammo, [family]: Math.max(0, have - perVehicle * (shot.rounds ?? 1)) } } });
  }
  for (const id of Object.keys(c.world.land).sort()) {
    if (!c.landAlive(id)) continue;
    const comps = c.world.land[id];
    const unit = c.land.units[id];
    const fe = c.fe(id)!;
    if (!hasRecord(comps.archetype)) continue;
    const rec = recordFor(comps.archetype);
    const moving = unit.lastMovedAt === c.time;
    const burn = (moving ? rec.stores.burnLph.moving : rec.stores.burnLph.idle) * unit.vehicles.fit * (dt / 3600);
    const fuel = rec.stores.fuelL > 0 ? Math.max(0, comps.stores.fuelL - burn) : comps.stores.fuelL;
    if (fuel !== comps.stores.fuelL) c.setLand(id, { stores: { ...c.comps(id)!.stores, fuelL: fuel } });
    // At zero the capability goes: no fuel, no moving; no rounds, no firing.
    if (rec.stores.fuelL > 0 && fuel <= 0 && (unit.order.kind === "move" || unit.order.kind === "withdraw") && fe.moveType !== "F") {
      c.land.setUnit(id, { order: { kind: "hold" }, bound: undefined });
      c.land.emit(id, "blocked", "out of fuel: it cannot move until resupplied");
    }
    const ammo = c.comps(id)!.stores.ammo;
    const families = Object.keys(rec.stores.ammo) as AmmoFamily[];
    const empty = families.length > 0 && families.every((f) => (ammo[f] ?? 0) <= 0);
    if (empty && unit.weaponReadyAt <= c.time) c.land.setUnit(id, { weaponReadyAt: c.time + 30 });
    // Low: ask the leader, once until resupplied.
    const ammoShare = families.length ? Math.min(...families.map((f) => (ammo[f] ?? 0) / Math.max(1, rec.stores.ammo[f] ?? 1))) : 1;
    const fuelShare = rec.stores.fuelL > 0 ? fuel / (rec.stores.fuelL * rec.body.count) : 1;
    const low = ammoShare < wfig("world/domains/landExt.AMMO_LOW") || fuelShare < wfig("world/domains/landExt.FUEL_LOW");
    if (low && comps.stocksLowAt == null && unit.cohesion === "steady") {
      c.setLand(id, { stocksLowAt: c.time });
      raiseQuestion(c, { agentId: id, side: fe.side, trigger: "stocksLow", about: { ammoPct: Math.round(ammoShare * 100), fuelPct: Math.round(fuelShare * 100) }, quality: fe.troopQuality });
    } else if (!low && comps.stocksLowAt != null) {
      c.setLand(id, { stocksLowAt: null });
    }
  }
}

/** Refill a land unit to its basic load (a resupply drop, a LOGPAC). */
export function refill(c: WorldCtx, id: string, share = 1): void {
  const comps = c.comps(id);
  if (!comps || !hasRecord(comps.archetype)) return;
  const rec = recordFor(comps.archetype);
  const full = { fuelL: rec.stores.fuelL * rec.body.count, ammo: rec.stores.ammo };
  const ammo = { ...comps.stores.ammo };
  for (const f of Object.keys(full.ammo) as AmmoFamily[]) ammo[f] = Math.min(full.ammo[f] ?? 0, (ammo[f] ?? 0) + (full.ammo[f] ?? 0) * share);
  c.setLand(id, {
    stores: { ...comps.stores, fuelL: Math.min(full.fuelL, comps.stores.fuelL + full.fuelL * share), ammo },
    mounts: comps.mounts.map((m) => {
      const w = rec.weapons.find((x) => x.id === m.weaponId);
      return w ? { ...m, rounds: Math.min(w.magazine, (ammo[w.family] ?? w.magazine)) } : m;
    }),
    stocksLowAt: null,
  });
}

// ── Obstacles ────────────────────────────────────────────────────────────

function obstacles(c: WorldCtx): void {
  if (!c.world.obstacles.length) return;
  for (const id of Object.keys(c.game.forceElements).sort()) {
    if (!c.landAlive(id)) continue;
    const fe = c.fe(id)!;
    const unit = c.land.units[id];
    // Who can see an obstacle: ditches from far off, mines only close, engineers and recce further.
    for (const o of c.world.obstacles) {
      if (o.knownTo.includes(fe.side)) continue;
      const near = distanceM(fe.position, o.at) - Math.max(o.widthM, o.depthM) / 2;
      const archetype = c.comps(id)?.archetype ?? "";
      const eyes = o.kind === "minefield" ? (/engineer|recce/.test(archetype) ? 400 : 150) : 1000;
      if (near > eyes) continue;
      if (!seesBetween(c.config.terrain, fe.position, groundAt(c.config.terrain, fe.position) + 2, o.at, groundAt(c.config.terrain, o.at) + 0.5)) continue;
      if (c.chance(o.kind === "minefield" ? wfig("world/domains/landExt.SPOT_MINES_PER_S") : 0.5)) {
        setObstacle(c, o.id, { knownTo: [...o.knownTo, fe.side] });
        c.log({ side: fe.side, domain: "land", kind: "obstacle", text: `${id} finds a ${o.kind}`, ids: [id], at: o.at });
      }
    }
    if (unit.order.kind !== "move" || unit.lastMovedAt !== c.time) continue;
    // Engineers going to make a breach go right up to it.
    if (c.world.breaches.some((b) => b.by === id)) continue;
    const order = unit.order;
    for (const o of c.world.obstacles) {
      if (o.owner === fe.side) continue;
      const here = inside(o, fe.position);
      const laned = onLane(o, fe.position);
      if (here && !laned && o.kind === "minefield") {
        const moved = (fe.speedKmh ?? 30) / 3.6 * 0.6;
        const p = wfig("world/domains/landExt.STRIKE_PER_100M") * (o.density / 0.5) * (moved / 100) * (fe.targetClass === "foot" ? 0.4 : 1);
        if (!c.chance(p)) continue;
        if (!o.knownTo.includes(fe.side)) setObstacle(c, o.id, { knownTo: [...o.knownTo, fe.side] });
        c.land.setUnit(id, { order: { kind: "hold" }, bound: undefined });
        applyHit(c, id, c.chance(wfig("world/domains/landExt.MINE_KILL")) ? 1 : 0, 10, { label: "a mine", firerSide: o.owner });
        c.setLand(id, { disabled: (c.comps(id)?.disabled ?? 0) + 1 });
        c.log({ side: fe.side, domain: "land", kind: "mine", text: `${id} strikes a mine in ${o.id}`, ids: [id], at: fe.position });
        raiseQuestion(c, { agentId: id, side: fe.side, trigger: "minefieldStruck", about: { obstacleId: o.id }, quality: fe.troopQuality });
        break;
      }
      // A known obstacle just ahead, not laned: stop short and decide.
      const aim = order.route?.[0] ?? order.to;
      const ahead = offsetBy(fe.position, bearingDeg(fe.position, aim), wfig("world/domains/landExt.LOOK_AHEAD_M"));
      const vehicleBlocked = o.kind !== "minefield" && fe.moveType !== "F";
      if (!here && inside(o, ahead) && !onLane(o, ahead) && (o.knownTo.includes(fe.side) || vehicleBlocked)) {
        if (c.last(`obs:${id}:${o.id}`) > c.time - 120 && c.world.memo[`wait:${id}`] == null) continue;
        c.mark(`obs:${id}:${o.id}`);
        c.land.setUnit(id, { order: { kind: "hold" }, bound: undefined });
        c.world.memo[`dest:${id}`] = c.time;
        c.world.memo[`destLat:${id}`] = order.to.lat;
        c.world.memo[`destLng:${id}`] = order.to.lng;
        raiseQuestion(c, { agentId: id, side: fe.side, trigger: "obstacleEncountered", about: { obstacleId: o.id }, quality: fe.troopQuality });
        break;
      }
    }
  }
  breaches(c);
}

/** Where a unit halted at an obstacle was going. */
function destinationOf(c: WorldCtx, id: string): LatLng | undefined {
  const lat = c.world.memo[`destLat:${id}`];
  const lng = c.world.memo[`destLng:${id}`];
  return lat != null && lng != null ? { lat, lng } : undefined;
}

function breaches(c: WorldCtx): void {
  for (const b of [...c.world.breaches]) {
    const o = obstacleById(c, b.obstacleId);
    const at = c.where(b.by);
    if (!o || !at || !c.landAlive(b.by)) {
      c.world.breaches.splice(c.world.breaches.indexOf(b), 1);
      continue;
    }
    const working = (b as typeof b & { state?: string }).state !== "moving";
    if (!working) {
      // Engineers on their way: at the breach point, and still, they start.
      if (distanceM(at, b.at) <= 60 && c.land.units[b.by].lastMovedAt !== c.time) {
        const i = c.world.breaches.indexOf(b);
        c.world.breaches[i] = { ...b, startedAt: c.time, doneAt: c.time + breachTime(c, b.by, o), state: "working" } as typeof b;
        c.log({ side: c.fe(b.by)!.side, domain: "land", kind: "breach", text: `${b.by} starts breaching ${o.id}`, ids: [b.by], at: b.at });
      }
      continue;
    }
    if (distanceM(at, b.at) > 80) {
      c.world.breaches.splice(c.world.breaches.indexOf(b), 1);
      c.log({ side: c.fe(b.by)!.side, domain: "land", kind: "breach", text: `${b.by} abandons the breach of ${o.id}`, ids: [b.by] });
      continue;
    }
    // Under fire, the work slows.
    if (c.land.units[b.by].suppression >= 25) {
      const i = c.world.breaches.indexOf(b);
      c.world.breaches[i] = { ...b, doneAt: b.doneAt + wfig("world/domains/landExt.UNDER_FIRE_SLOW") * c.config.timing.tickS };
      continue;
    }
    if (c.time < b.doneAt) continue;
    c.world.breaches.splice(c.world.breaches.indexOf(b), 1);
    setObstacle(c, o.id, { lanes: [...o.lanes, { at: b.at, widthM: wfig("world/domains/landExt.LANE_WIDTH_M"), by: b.by, doneAt: c.time }] });
    c.stat((s) => ({ ...s, breachS: [...s.breachS, c.time - b.startedAt] }));
    const side = c.fe(b.by)!.side;
    c.log({ side, domain: "land", kind: "breach", text: `${b.by} has cleared a lane through ${o.id} in ${Math.round((c.time - b.startedAt) / 60)} min`, ids: [b.by], at: b.at });
    // Everyone waiting at it goes through.
    for (const key of Object.keys(c.world.memo)) {
      const m = /^wait:(.+)$/.exec(key);
      if (!m || c.world.memo[`waitObs:${m[1]}`] == null) continue;
      const waiter = m[1];
      if (obstacleIdFor(c, waiter) !== o.id || !c.landAlive(waiter)) continue;
      delete c.world.memo[key];
      throughLane(c, waiter, obstacleById(c, o.id)!);
    }
  }
}

const OBSTACLE_IDS = new WeakMap<object, string>();
void OBSTACLE_IDS;
function obstacleIdFor(c: WorldCtx, unitId: string): string | undefined {
  const n = c.world.memo[`waitObs:${unitId}`];
  return n != null ? c.world.obstacles[n]?.id : undefined;
}
function waitAt(c: WorldCtx, unitId: string, o: Obstacle): void {
  const fe = c.fe(unitId);
  if (fe) c.land.setUnit(unitId, { mission: { task: "hold", at: fe.position, purpose: `wait for a lane through ${o.id}` } });
  c.mark(`wait:${unitId}`);
  c.world.memo[`waitObs:${unitId}`] = c.world.obstacles.indexOf(o);
}

/** Through the nearest lane, then on to where it was going. */
function throughLane(c: WorldCtx, id: string, o: Obstacle): void {
  const fe = c.fe(id);
  if (!fe || !o.lanes.length) return;
  const lane = [...o.lanes].sort((a, b) => distanceM(a.at, fe.position) - distanceM(b.at, fe.position))[0];
  const { u } = obstacleFrame(o, lane.at);
  const { v } = obstacleFrame(o, fe.position);
  const sign = v < 0 ? -1 : 1;
  const entry = fromFrame(o, u, sign * (o.depthM / 2 + 30));
  const exit = fromFrame(o, u, -sign * (o.depthM / 2 + 30));
  const dest = destinationOf(c, id);
  const then: RtOrder | undefined = dest ? { kind: "move", to: dest, mode: "tactical" } : undefined;
  // Straight through the lane: no route planning across the field.
  if (dest) c.land.setUnit(id, { mission: { task: "take", at: dest, purpose: "go on through the lane" } });
  c.land.setUnit(id, {
    order: { kind: "move", to: entry, mode: "tactical", route: [entry], then: { kind: "move", to: exit, mode: "tactical", route: [exit], ...(then ? { then } : {}) } },
    bound: undefined,
  });
}

function breachTime(c: WorldCtx, by: string, o: Obstacle): number {
  const archetype = c.comps(by)?.archetype;
  const engineers = archetype === "generic/engineerSection";
  const spread = 0.6 + c.rng.int(801) / 1000; // 0.6–1.4
  return wfig("world/domains/landExt.BREACH_S") * spread * Math.sqrt(Math.max(0.1, o.density) / 0.5) * (engineers ? 1 : wfig("world/domains/landExt.NON_ENGINEER_SLOW"));
}

function startBreach(c: WorldCtx, by: string, o: Obstacle, from: LatLng): void {
  if (c.world.breaches.some((b) => b.by === by)) return;
  const point = edges(o, from).near;
  c.world.breaches.push({ obstacleId: o.id, by, startedAt: c.time, doneAt: Infinity, at: point, state: "moving" } as (typeof c.world.breaches)[number]);
  // The breach is its mission now: asked "what next" on arrival, it stays and works.
  c.land.order(by, { kind: "move", to: point, mode: "tactical" }, { mission: { task: "hold", at: point, purpose: `breach ${o.id}` } });
}

// ── Dismounts ────────────────────────────────────────────────────────────

function dismounts(c: WorldCtx): void {
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (comps.dismounts.carried <= 0 || !c.landAlive(id)) continue;
    const unit = c.land.units[id];
    if (unit.cohesion !== "steady") continue;
    if (c.time - c.last(`dismountAsk:${id}`) < wfig("world/domains/landExt.DISMOUNT_ASK_S")) continue;
    const fe = c.fe(id)!;
    // A planned dismount point: it has a cover spot it is heading for, and is there.
    const planned = c.world.memo[`dismountAt:${id}`];
    if (planned != null && unit.lastMovedAt !== c.time) {
      delete c.world.memo[`dismountAt:${id}`];
      dismount(c, id);
      continue;
    }
    const enemyNear = Object.keys(c.game.forceElements).some((e) => {
      const enemy = c.fe(e)!;
      return enemy.side !== fe.side && c.landAlive(e) && c.land.known(id, e) !== "none" && distanceM(enemy.position, fe.position) <= wfig("world/domains/landExt.DISMOUNT_AT_M");
    });
    const objective = unit.mission.at && distanceM(fe.position, unit.mission.at) <= 300;
    // Fired on: the infantry are safer out than in a vehicle being shot at.
    const underFire = c.time - unit.lastIncomingAt <= 10;
    if (!enemyNear && !objective && !underFire) continue;
    c.mark(`dismountAsk:${id}`);
    raiseQuestion(c, { agentId: id, side: fe.side, trigger: "dismountPoint", about: { enemyNear: enemyNear || underFire, objective: !!objective }, quality: fe.troopQuality });
  }
}

/** The section gets out: a unit of its own beside the vehicle, on the same mission and orders. */
export function dismount(c: WorldCtx, carrierId: string): string | undefined {
  const comps = c.comps(carrierId);
  const carrier = c.fe(carrierId);
  if (!comps || !carrier || comps.dismounts.carried <= 0) return undefined;
  const archetype = (hasRecord(comps.archetype) && recordFor(comps.archetype).carried.dismounts?.archetype) || "generic/infantrySection";
  const id = `${carrierId}-dis${comps.dismounts.out.length + 1}`;
  const at = offsetBy(carrier.position, carrier.facing ?? 0, 30);
  const placed = toGameStateFromPlaced(
    [{ id, label: `${carrier.label} dismounts`, side: carrier.side, platform: archetype, platformCount: comps.dismounts.carried, troopQuality: qualityName(carrier.troopQuality), position: at }],
    c.config.ruleset,
  ).forceElements[id];
  const fe = { ...placed, facing: carrier.facing };
  const carrierUnit = c.land.units[carrierId];
  const unit = { ...freshUnitFor(fe), mission: carrierUnit.mission, roe: carrierUnit.roe, orders: carrierUnit.orders, lastEventAt: c.time };
  // It goes where its vehicle was going, on foot.
  const order: RtOrder = carrierUnit.order.kind === "move" ? { kind: "move", to: carrierUnit.order.to, mode: "tactical" } : { kind: "hold" };
  c.land.addUnit(fe, { ...unit, order });
  c.world.land[id] = { ...landComponents(archetype), carrier: carrierId };
  c.setLand(carrierId, { dismounts: { carried: 0, out: [...comps.dismounts.out, id] } });
  (c.world as { joinedStrength: Record<Side, number> }).joinedStrength = { ...c.world.joinedStrength, [carrier.side]: c.world.joinedStrength[carrier.side] + fe.combatStrength };
  c.log({ side: carrier.side, domain: "land", kind: "dismount", text: `${carrierId} dismounts its infantry (${id})`, ids: [carrierId, id], at });
  return id;
}

function qualityName(q: number): "conscript" | "regular" | "veteran" | "elite" {
  return q >= 8 ? "elite" : q >= 6 ? "veteran" : q >= 4 ? "regular" : "conscript";
}

// ── Digging in ───────────────────────────────────────────────────────────

function digging(c: WorldCtx): void {
  const dt = c.config.timing.tickS;
  for (const id of Object.keys(c.world.land).sort()) {
    if (!c.landAlive(id)) continue;
    const comps = c.world.land[id];
    const unit = c.land.units[id];
    if (unit.lastMovedAt === c.time) {
      if (comps.dugIn > 0) c.setLand(id, { dugIn: 0 });
      continue;
    }
    if (comps.dugIn > 0 && comps.dugIn < 1) {
      c.setLand(id, { dugIn: Math.min(1, comps.dugIn + dt / wfig("world/domains/landExt.DIG_IN_S")) });
      continue;
    }
    if (comps.dugIn > 0 || unit.cohesion !== "steady") continue;
    const still = ["hold", "overwatch", "observe", "wait"].includes(unit.order.kind);
    if (!still || c.time - unit.lastMovedAt < wfig("world/domains/landExt.SETTLE_TO_DIG_S")) continue;
    if (c.time - c.last(`digAsk:${id}`) < 1800) continue;
    c.mark(`digAsk:${id}`);
    raiseQuestion(c, { agentId: id, side: c.fe(id)!.side, trigger: "settledInPosition", quality: c.fe(id)!.troopQuality });
  }
}

// ── Handlers ─────────────────────────────────────────────────────────────

function nearestEngineer(c: WorldCtx, side: Side, near: LatLng): string | undefined {
  return Object.keys(c.world.land)
    .filter((id) => c.world.land[id].archetype === "generic/engineerSection" && c.landAlive(id) && c.fe(id)!.side === side)
    .filter((id) => !c.world.breaches.some((b) => b.by === id))
    .filter((id) => distanceM(c.fe(id)!.position, near) <= 8000)
    .sort((a, b) => distanceM(c.fe(a)!.position, near) - distanceM(c.fe(b)!.position, near))[0];
}

registerHandlers({
  "land.breach": {
    available: (c, q) => c.comps(q.agentId)?.archetype === "generic/engineerSection" || c.comps(q.agentId)?.archetype === "generic/mbt",
    apply(c, q) {
      const o = obstacleById(c, q.about.obstacleId);
      const fe = c.fe(q.agentId);
      if (o && fe) startBreach(c, q.agentId, o, fe.position);
    },
  },
  "land.bypass": {
    available: (c, q) => (obstacleById(c, q.about.obstacleId)?.widthM ?? Infinity) <= 3000,
    apply(c, q) {
      const o = obstacleById(c, q.about.obstacleId);
      const fe = c.fe(q.agentId);
      if (!o || !fe) return;
      const { u, v } = obstacleFrame(o, fe.position);
      const side = u >= 0 ? 1 : -1;
      const sign = v < 0 ? -1 : 1;
      const corner = fromFrame(o, side * (o.widthM / 2 + 150), sign * (o.depthM / 2 + 50));
      const beyond = fromFrame(o, side * (o.widthM / 2 + 150), -sign * (o.depthM / 2 + 100));
      const dest = destinationOf(c, q.agentId);
      c.land.setUnit(q.agentId, {
        order: { kind: "move", to: corner, mode: "tactical", route: [corner], then: { kind: "move", to: beyond, mode: "tactical", route: [beyond], ...(dest ? { then: { kind: "move", to: dest, mode: "tactical" } } : {}) } },
        bound: undefined,
      });
    },
  },
  "land.callEngineers": {
    available: (c, q) => nearestEngineer(c, q.side, c.where(q.agentId)!) != null,
    apply(c, q) {
      const o = obstacleById(c, q.about.obstacleId);
      const fe = c.fe(q.agentId);
      if (!o || !fe) return;
      const eng = nearestEngineer(c, q.side, fe.position);
      if (eng) startBreach(c, eng, o, fe.position);
      c.land.setUnit(q.agentId, { order: { kind: "hold" }, bound: undefined });
      waitAt(c, q.agentId, o);
      c.log({ side: q.side, domain: "land", kind: "engineers", text: `${q.agentId} calls the engineers (${eng ?? "none free"}) to ${o.id}`, ids: [q.agentId, ...(eng ? [eng] : [])] });
    },
  },
  "land.waitAtObstacle": {
    apply(c, q) {
      const o = obstacleById(c, q.about.obstacleId);
      if (o) waitAt(c, q.agentId, o);
    },
  },
  "land.pushThroughLane": {
    available: (c, q) => (obstacleById(c, q.about.obstacleId)?.lanes.length ?? 0) > 0,
    apply(c, q) {
      const o = obstacleById(c, q.about.obstacleId);
      if (o) throughLane(c, q.agentId, o);
    },
  },
  "land.reverseOut": {
    apply(c, q) {
      const fe = c.fe(q.agentId);
      if (fe) c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, ((fe.facing ?? 0) + 180) % 360, 120), mode: "march", dash: true });
    },
  },
  "land.dismountHere": { available: (c, q) => (c.comps(q.agentId)?.dismounts.carried ?? 0) > 0, apply: (c, q) => void dismount(c, q.agentId) },
  "land.dismountInCover": {
    available: (c, q) => (c.comps(q.agentId)?.dismounts.carried ?? 0) > 0,
    apply(c, q) {
      const fe = c.fe(q.agentId);
      if (!fe) return;
      if (inCover(c.config.terrain, fe.position)) return void dismount(c, q.agentId);
      const cover = nearestCover(fe, c.config, 300);
      if (!cover) return void dismount(c, q.agentId);
      c.land.order(q.agentId, { kind: "move", to: cover, mode: "tactical", dash: true });
      c.mark(`dismountAt:${q.agentId}`);
    },
  },
  "land.stayMounted": { apply: () => {} },
  "land.remount": {
    available: (c, q) => c.comps(q.agentId)?.carrier != null,
    apply(c, q) {
      const carrier = c.comps(q.agentId)?.carrier;
      const at = carrier ? c.fe(carrier)?.position : undefined;
      if (at) c.land.order(q.agentId, { kind: "move", to: at, mode: "march" });
    },
  },
  "land.requestResupply": {
    apply(c, q) {
      if (REQUEST_RESUPPLY) REQUEST_RESUPPLY(c, q.agentId);
    },
  },
  "land.conserve": { apply: (c, q) => c.land.setUnit(q.agentId, { roe: "ifFiredUpon" }) },
  "land.withdrawToResupply": {
    apply(c, q) {
      const fe = c.fe(q.agentId);
      if (!fe) return;
      const back = c.world.startLines[fe.side];
      c.land.order(q.agentId, { kind: "withdraw", to: back });
      if (REQUEST_RESUPPLY) REQUEST_RESUPPLY(c, q.agentId);
    },
  },
  "land.digIn": { apply: (c, q) => c.setLand(q.agentId, { dugIn: 0.01 }) },
  "c2.carryOn": { apply: () => {} },
});

/** How a resupply request is handled: set by the logistics domain. */
let REQUEST_RESUPPLY: ((c: WorldCtx, unitId: string) => void) | null = null;
export function setResupplyHandler(fn: (c: WorldCtx, unitId: string) => void): void {
  REQUEST_RESUPPLY = fn;
}

registerRules({
  obstacleEncountered: (c, q, options) => {
    const o = obstacleById(c, q.about.obstacleId);
    if (o?.lanes.length) return prefer(options, "land.pushThroughLane");
    if (c.comps(q.agentId)?.archetype === "generic/engineerSection") return prefer(options, "land.breach");
    return prefer(options, "land.callEngineers", "land.bypass", "land.waitAtObstacle");
  },
  minefieldStruck: (c, q, options) => (nearestEngineer(c, q.side, c.where(q.agentId)!) ? prefer(options, "land.callEngineers", "land.reverseOut") : prefer(options, "land.reverseOut")),
  dismountPoint: (_c, q, options) => (q.about.enemyNear ? prefer(options, "land.dismountInCover", "land.dismountHere") : prefer(options, "land.dismountHere")),
  stocksLow: (_c, q, options) => ((q.about.ammoPct as number) < 10 ? prefer(options, "land.withdrawToResupply", "land.requestResupply") : prefer(options, "land.requestResupply")),
  settledInPosition: (_c, _q, options) => prefer(options, "land.digIn"),
});

registerSystem({
  name: "landExt",
  order: 90,
  when: (c) => c.simulated("land"),
  run(c) {
    if (sustaining(c)) sustain(c);
    obstacles(c);
    dismounts(c);
    digging(c);
  },
});
