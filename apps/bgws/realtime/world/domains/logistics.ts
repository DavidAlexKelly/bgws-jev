// ── world/domains/logistics.ts ───────────────────────────────────────────
// Logistics and medical (MULTI_DOMAIN_PLAN.md W10): sustainment as a network
// that can be cut.
//
//   NODES       railheads and depots are node entities holding stocks. A
//               railhead tops up the depots in its reach every hour while
//               both stand; cut it and the depots only drain.
//   LOGPAC      units that run low send a LOGSTAT (stocksLow → request). The
//               logistics cell — the depot — is asked on its cycle who first
//               and by what: a convoy (trucks on the board, visible and
//               targetable), a ground robot or a drone for the last mile, or
//               wait for dark. A convoy drives to the unit, refills it from
//               the depot's stocks as far as its cargo goes, and comes home.
//   THREATS     a convoy whose route passes a known enemy is asked (push on,
//               alternate route, wait for dark, scatter); one a drone dives
//               on is asked too; a node that is hit asks the cell (relocate,
//               disperse, repair and stay).
//   MAINTENANCE vehicles knocked out but not destroyed are recovered and
//               repaired after a declared time, from the unit's spares, or one
//               is cannibalised for another.
//   MEDICAL     wounded wait at the unit; the unit is asked whether to
//               evacuate now under threat, wait for dark or send a robot.
//               An ambulance drives up, collects them and takes them back to
//               treatment; how long that took decides how many die of wounds
//               (a declared curve), and every evacuation time is reported.
//               Abstracted medical gives each casualty a drawn evacuation time.
//
// With logistics off, stocks are unlimited and none of this runs, as before.

import type { LatLng } from "../../../lib/board";
import { toGameStateFromPlaced } from "../../../lib/forceBuilder";
import { inCover } from "../../../lib/proceduralTerrain";
import { opposing, SIDES, type Side } from "../../../lib/state";
import { hasRecord, recordFor } from "../../catalogue";
import type { AmmoFamily } from "../../catalogue/schema";
import { freshUnitFor, nearestCover, strengthFor } from "../../engine/engine";
import type { RtOrder } from "../../engine/types";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { landComponents, onCreateWorld } from "../create";
import { isNight } from "../environment";
import { wfig } from "../figures";
import { bearingDeg, distanceM, offsetBy } from "../geo";
import { raiseQuestion } from "../questions";
import { onScheduled, registerSystem } from "../step";
import type { Evacuation, LogisticsState, SupplyJob } from "../types";
import { resumeStalled } from "./landExt";
import { setSimulatedResupply } from "./support";

/** Tonnes per round (or per litre of fuel), for what a load weighs. */
const KG: Record<AmmoFamily, number> = {
  smallArms: 0.025,
  tank120: 25,
  cannon30: 0.8,
  atgm: 30,
  mortar120: 18,
  artillery155: 50,
  rocket: 300,
  samLong: 900,
  samMedium: 400,
  samShort: 40,
  aam: 150,
  agm: 50,
  bomb: 250,
  antiShip: 700,
  torpedo: 1500,
  ciws: 0.3,
  drone: 3,
  missile: 300,
};
const FUEL_KG_PER_L = 0.85;

const simulatedLog = (c: WorldCtx) => c.simulated("logistics");
const medicalOn = (c: WorldCtx) => c.mode("medical") !== "off";

function state(c: WorldCtx): LogisticsState {
  return c.world.logistics ?? { requests: [], jobs: [], evacs: [], pool: { blue: [], red: [] } };
}
function setState(c: WorldCtx, patch: Partial<LogisticsState>): void {
  c.world.logistics = { ...state(c), ...patch };
}
function setJob(c: WorldCtx, id: string, patch: Partial<SupplyJob>): void {
  const s = state(c);
  setState(c, { jobs: s.jobs.map((j) => (j.id === id ? { ...j, ...patch } : j)) });
}
function setEvac(c: WorldCtx, id: string, patch: Partial<Evacuation>): void {
  const s = state(c);
  setState(c, { evacs: s.evacs.map((e) => (e.id === id ? { ...e, ...patch } : e)) });
}
function nextId(c: WorldCtx, prefix: string): string {
  const n = c.world.nextId;
  (c.world as { nextId: number }).nextId = n + 1;
  return `${prefix}-${n}`;
}

onCreateWorld(40, (world, state, config) => {
  const mode = config.scope?.domains;
  if (!mode || (mode.logistics !== "simulated" && mode.medical === "off")) return;
  // Convoys and ambulances the scenario placed start in the pool, free for the first jobs.
  const pool: Record<Side, string[]> = { blue: [], red: [] };
  for (const [id, comps] of Object.entries(world.land).sort(([a], [b]) => a.localeCompare(b))) {
    const fe = state.game.forceElements[id];
    if (fe && (comps.archetype === "generic/convoyTruck" || comps.archetype === "generic/medevac")) pool[fe.side].push(id);
  }
  world.logistics = { requests: [], jobs: [], evacs: [], pool };
});

// ── Nodes ────────────────────────────────────────────────────────────────

const isNode = (archetype: string) => archetype === "generic/depot" || archetype === "generic/railhead";

/** A side's depots and railheads still standing, nearest first to `to`. */
function nodesOf(c: WorldCtx, side: Side, to?: LatLng, kind?: string): string[] {
  return Object.values(c.world.entities)
    .filter((e) => e.owner === side && e.domain === "node" && isNode(e.archetype) && (kind == null || e.archetype === kind) && e.hp > 0 && (c.world.memo[`nodeDown:${e.id}`] ?? -Infinity) <= c.time)
    .sort((a, b) => (to ? distanceM(a.pos, to) - distanceM(b.pos, to) : 0) || a.id.localeCompare(b.id))
    .map((e) => e.id);
}

/** Where a side treats its wounded: its nearest depot (the support area), else well behind its start line. */
function treatmentAt(c: WorldCtx, side: Side, from: LatLng): LatLng {
  const node = nodesOf(c, side, from, "generic/depot")[0];
  if (node) return c.entity(node)!.pos;
  const start = c.world.startLines[side];
  return offsetBy(start, bearingDeg(c.world.startLines[opposing(side)], start), wfig("world/domains/logistics.TREATMENT_BACK_M"));
}

/** The railhead feeds the depots it reaches, every hour, while both stand (W10 rail). */
function rail(c: WorldCtx): void {
  if (c.time % 3600 !== 0) return;
  for (const side of SIDES) {
    for (const r of nodesOf(c, side, undefined, "generic/railhead")) {
      if (c.world.memo[`railCut:${side}`] != null) continue;
      const head = c.entity(r)!;
      for (const d of nodesOf(c, side, head.pos, "generic/depot")) {
        const depot = c.entity(d)!;
        if (distanceM(depot.pos, head.pos) > wfig("world/domains/logistics.RAIL_REACH_M")) continue;
        const full = recordFor(depot.archetype).stores;
        const share = wfig("world/domains/logistics.RAIL_SHARE_PER_H");
        const ammo = { ...depot.stores.ammo };
        for (const f of Object.keys(full.ammo) as AmmoFamily[]) ammo[f] = Math.min(full.ammo[f] ?? 0, (ammo[f] ?? 0) + (full.ammo[f] ?? 0) * share);
        c.setEntity(d, { stores: { ...depot.stores, fuelL: Math.min(full.fuelL, depot.stores.fuelL + full.fuelL * share), ammo } });
      }
    }
  }
}

/** A node that has been hit asks the logistics cell what to do (nodeStruck). */
function nodeHits(c: WorldCtx): void {
  for (const e of Object.values(c.world.entities)) {
    if (e.domain !== "node" || !isNode(e.archetype) || e.owner === "neutral") continue;
    const key = `nodeHp:${e.id}`;
    const was = c.world.memo[key];
    c.world.memo[key] = e.hp;
    if (was == null || e.hp >= was || e.hp <= 0) continue;
    // Each hit costs stocks, less if they are dispersed.
    const loss = wfig("world/domains/logistics.HIT_STOCK_LOSS") * (c.world.memo[`dispersed:${e.id}`] != null ? 0.4 : 1);
    const ammo = Object.fromEntries(Object.entries(e.stores.ammo).map(([f, n]) => [f, Math.floor((n ?? 0) * (1 - loss))]));
    c.setEntity(e.id, { stores: { ...e.stores, fuelL: e.stores.fuelL * (1 - loss), ammo } });
    c.log({ side: e.owner, domain: "logistics", kind: "nodeStruck", text: `${e.label} (${e.id}) is hit: ${Math.round(loss * 100)}% of its stocks lost`, ids: [e.id], at: e.pos });
    raiseQuestion(c, { agentId: e.id, side: e.owner, trigger: "nodeStruck", about: { nodeId: e.id, hp: e.hp }, quality: 4 });
  }
}

// ── Requests and the LOGPAC cycle ────────────────────────────────────────

/** How short of its basic load a unit is: the lowest share of fuel or any ammunition. */
export function needOf(c: WorldCtx, unitId: string): number {
  const comps = c.comps(unitId);
  if (!comps || !hasRecord(comps.archetype)) return 1;
  const rec = recordFor(comps.archetype);
  const fuel = rec.stores.fuelL > 0 ? comps.stores.fuelL / (rec.stores.fuelL * rec.body.count) : 1;
  const ammo = (Object.keys(rec.stores.ammo) as AmmoFamily[]).map((f) => (comps.stores.ammo[f] ?? 0) / Math.max(1, rec.stores.ammo[f] ?? 1));
  return Math.min(fuel, ...ammo, 1);
}

setSimulatedResupply((c, unitId) => {
  const side = c.fe(unitId)?.side;
  if (!side) return;
  const s = state(c);
  if (s.requests.some((r) => r.unitId === unitId) || s.jobs.some((j) => j.unitId === unitId && (j.state === "outbound" || j.state === "waiting"))) return;
  setState(c, { requests: [...s.requests, { unitId, side, at: c.time, need: needOf(c, unitId) }] });
  c.log({ side, domain: "logistics", kind: "logstat", text: `${unitId} sends its LOGSTAT: ${Math.round(needOf(c, unitId) * 100)}% of its load left`, ids: [unitId] });
});

/** Units low on stocks report without being asked, on the logistics cycle. */
function logstats(c: WorldCtx): void {
  for (const id of Object.keys(c.world.land).sort()) {
    if (!c.landAlive(id)) continue;
    const comps = c.world.land[id];
    if (comps.archetype === "generic/convoyTruck" || comps.archetype === "generic/medevac" || comps.archetype === "generic/ugv") continue;
    if (needOf(c, id) < wfig("world/domains/logistics.LOGSTAT_BELOW")) {
      const s = state(c);
      const side = c.fe(id)!.side;
      if (!s.requests.some((r) => r.unitId === id) && !s.jobs.some((j) => j.unitId === id && j.state !== "done" && j.state !== "failed")) {
        setState(c, { requests: [...s.requests, { unitId: id, side, at: c.time, need: needOf(c, id) }] });
      }
    }
  }
}

/** On its cycle, each side's logistics cell is asked about its most urgent request. */
function logCycle(c: WorldCtx): void {
  if (c.time % wfig("world/domains/logistics.CYCLE_S") !== 0) return;
  logstats(c);
  for (const side of SIDES) {
    const waiting = state(c).requests.filter((r) => r.side === side && c.landAlive(r.unitId));
    if (!waiting.length) continue;
    const unitAt = (id: string) => c.fe(id)!.position;
    const urgent = [...waiting].sort((a, b) => needOf(c, a.unitId) - needOf(c, b.unitId) || a.at - b.at)[0];
    const node = nodesOf(c, side, unitAt(urgent.unitId), "generic/depot")[0] ?? nodesOf(c, side, unitAt(urgent.unitId))[0];
    if (!node) {
      if (c.time - c.last(`noNode:${side}`) > 3600) {
        c.mark(`noNode:${side}`);
        c.log({ side, domain: "logistics", kind: "noSupply", text: `no depot or railhead stands: ${waiting.length} requests cannot be met`, ids: waiting.map((r) => r.unitId) });
      }
      continue;
    }
    const threat = routeThreat(c, side, c.entity(node)!.pos, unitAt(urgent.unitId));
    raiseQuestion(c, { agentId: node, side, trigger: "resupplyRequests", about: { unitId: urgent.unitId, nodeId: node, waiting: waiting.length, need: Math.round(needOf(c, urgent.unitId) * 100), threat: threat != null, distanceKm: Math.round(distanceM(c.entity(node)!.pos, unitAt(urgent.unitId)) / 1000) }, quality: 4 });
  }
}

function takeRequest(c: WorldCtx, unitId: string): void {
  const s = state(c);
  setState(c, { requests: s.requests.filter((r) => r.unitId !== unitId) });
}

/** A land unit put on the board by the logistics system: a convoy, an ambulance, a robot. */
function spawn(c: WorldCtx, side: Side, archetype: string, at: LatLng, label: string): string {
  const id = nextId(c, `${side === "blue" ? "b" : "r"}-${archetype.replace("generic/", "")}`);
  const fe = toGameStateFromPlaced([{ id, label, side, platform: archetype, platformCount: recordFor(archetype).body.count, troopQuality: "regular", position: at }], c.config.ruleset).forceElements[id];
  c.land.addUnit(fe, { ...freshUnitFor(fe), order: { kind: "hold" }, roe: "ifFiredUpon", lastEventAt: c.time });
  c.world.land[id] = landComponents(archetype);
  // Support vehicles do not count towards the breakpoint: they joined with no fighting strength to lose.
  (c.world as { joinedStrength: Record<Side, number> }).joinedStrength = { ...c.world.joinedStrength, [side]: c.world.joinedStrength[side] + fe.combatStrength };
  return id;
}

/** A free vehicle of this kind at or near the node, or a new one from it. */
function vehicleFor(c: WorldCtx, side: Side, archetype: string, nodeId: string): string {
  const s = state(c);
  const node = c.entity(nodeId)!;
  const free = s.pool[side].find((id) => c.comps(id)?.archetype === archetype && c.landAlive(id) && distanceM(c.fe(id)!.position, node.pos) < 2000);
  if (free) {
    setState(c, { pool: { ...s.pool, [side]: s.pool[side].filter((x) => x !== free) } });
    return free;
  }
  return spawn(c, side, archetype, offsetBy(node.pos, c.rng.int(360), 200), `${recordFor(archetype).name} from ${node.label}`);
}

function sendConvoy(c: WorldCtx, side: Side, unitId: string, nodeId: string, waitForDark = false): void {
  takeRequest(c, unitId);
  const vehicleId = vehicleFor(c, side, "generic/convoyTruck", nodeId);
  const job: SupplyJob = { id: nextId(c, "job"), side, unitId, nodeId, means: "convoy", vehicleId, state: waitForDark ? "waiting" : "outbound", startedAt: c.time, ...(waitForDark ? { waitForDark: true } : {}) };
  setState(c, { jobs: [...state(c).jobs, job] });
  if (!waitForDark) drive(c, vehicleId, c.fe(unitId)!.position);
  c.log({ side, domain: "logistics", kind: "convoy", text: `${nodeId} sends a convoy (${vehicleId}) to ${unitId}${waitForDark ? " after dark" : ""}`, ids: [nodeId, vehicleId, unitId] });
}

function drive(c: WorldCtx, vehicleId: string, to: LatLng): void {
  // Re-ordered only when the destination has moved: a fresh order costs the vehicle its start.
  const now = c.land.units[vehicleId]?.order;
  if (now?.kind === "move" && distanceM(now.to, to) < wfig("world/domains/logistics.HANDOVER_M")) return;
  const order: RtOrder = { kind: "move", to, mode: "march" };
  c.land.order(vehicleId, order);
}

/** Small loads by robot or drone: no trucks on the road, a fraction of a convoy's cargo, a time to arrive. */
function sendSmall(c: WorldCtx, side: Side, unitId: string, nodeId: string, means: "ugv" | "drone"): void {
  takeRequest(c, unitId);
  const node = c.entity(nodeId)!;
  const d = distanceM(node.pos, c.fe(unitId)!.position);
  const speed = means === "ugv" ? recordFor("generic/ugv").mover.cruiseKmh / 3.6 : wfig("world/domains/logistics.DRONE_SPEED_MS");
  const job: SupplyJob = { id: nextId(c, "job"), side, unitId, nodeId, means, state: "outbound", startedAt: c.time };
  setState(c, { jobs: [...state(c).jobs, job] });
  c.schedule(c.time + Math.round(d / speed), "smallDelivery", job.id);
  c.log({ side, domain: "logistics", kind: means, text: `${nodeId} sends ${means === "ugv" ? "a ground robot" : "a cargo drone"} to ${unitId} (${Math.round(d / 1000)} km)`, ids: [nodeId, unitId] });
}

onScheduled("smallDelivery", (c, e) => {
  const job = state(c).jobs.find((j) => j.id === e.key);
  if (!job || job.state !== "outbound") return;
  // Near the enemy a robot or drone is sometimes lost on the way.
  const enemyClose = Object.values(c.world.tracks[job.side]).some((t) => t.domain === "land" && distanceM(t.at, c.fe(job.unitId)?.position ?? t.at) < 3000);
  if (!c.landAlive(job.unitId) || (enemyClose && c.chance(wfig("world/domains/logistics.SMALL_LOSS")))) {
    setJob(c, job.id, { state: "failed" });
    c.log({ side: job.side, domain: "logistics", kind: "lost", text: `the ${job.means === "ugv" ? "robot" : "drone"} for ${job.unitId} never arrives`, ids: [job.unitId] });
    return;
  }
  const cargoT = job.means === "ugv" ? (recordFor("generic/ugv").carried.cargoT ?? 0.3) : wfig("world/domains/logistics.DRONE_CARGO_T");
  const share = deliver(c, job.nodeId, job.unitId, cargoT);
  setJob(c, job.id, { state: "done", deliveredAt: c.time });
  c.log({ side: job.side, domain: "logistics", kind: "delivered", text: `${job.unitId} gets ${Math.round(share * 100)}% of what it needed by ${job.means === "ugv" ? "robot" : "drone"}`, ids: [job.unitId] });
});

/**
 * Refill a unit from a node's stocks, as far as `cargoT` tonnes go: fuel and
 * each ammunition family it is short of, in proportion. Returns the share of
 * its need that was met.
 */
export function deliver(c: WorldCtx, nodeId: string, unitId: string, cargoT: number): number {
  const comps = c.comps(unitId);
  const node = c.entity(nodeId);
  if (!comps || !node || !hasRecord(comps.archetype)) return 0;
  const rec = recordFor(comps.archetype);
  const fuelFull = rec.stores.fuelL * rec.body.count;
  const fuelShort = Math.max(0, fuelFull - comps.stores.fuelL);
  const ammoShort = Object.fromEntries((Object.keys(rec.stores.ammo) as AmmoFamily[]).map((f) => [f, Math.max(0, (rec.stores.ammo[f] ?? 0) - (comps.stores.ammo[f] ?? 0))])) as Partial<Record<AmmoFamily, number>>;
  const needT = (fuelShort * FUEL_KG_PER_L + Object.entries(ammoShort).reduce((s, [f, n]) => s + (n ?? 0) * KG[f as AmmoFamily], 0)) / 1000;
  if (needT <= 0) return 1;
  const share = Math.min(1, cargoT / needT);
  // What the node has of each.
  const fuel = Math.min(fuelShort * share, node.stores.fuelL);
  const ammo = { ...comps.stores.ammo };
  const nodeAmmo = { ...node.stores.ammo };
  for (const [f, n] of Object.entries(ammoShort) as [AmmoFamily, number][]) {
    const give = Math.min(Math.floor(n * share), nodeAmmo[f] ?? 0);
    ammo[f] = (ammo[f] ?? 0) + give;
    nodeAmmo[f] = (nodeAmmo[f] ?? 0) - give;
  }
  c.setEntity(nodeId, { stores: { ...node.stores, fuelL: node.stores.fuelL - fuel, ammo: nodeAmmo } });
  c.setLand(unitId, {
    stores: { ...comps.stores, fuelL: comps.stores.fuelL + fuel, ammo },
    mounts: comps.mounts.map((m) => {
      const w = rec.weapons.find((x) => x.id === m.weaponId);
      return w ? { ...m, rounds: Math.min(w.magazine, ammo[w.family] ?? w.magazine) } : m;
    }),
    stocksLowAt: null,
  });
  resumeStalled(c, unitId);
  return share;
}

// ── Convoys on the road ──────────────────────────────────────────────────

/** Distance from a point to the segment a→b, m (flat approximation). */
function offLineM(p: LatLng, a: LatLng, b: LatLng): number {
  const k = 111_320;
  const cos = Math.cos((a.lat * Math.PI) / 180);
  const bx = (b.lng - a.lng) * k * cos, by = (b.lat - a.lat) * k;
  const px = (p.lng - a.lng) * k * cos, py = (p.lat - a.lat) * k;
  const len = bx * bx + by * by;
  const t = len === 0 ? 0 : Math.max(0, Math.min(1, (px * bx + py * by) / len));
  return Math.hypot(px - bx * t, py - by * t);
}

/** A known enemy land unit close to the road from `from` to `to`, if any. */
function routeThreat(c: WorldCtx, side: Side, from: LatLng, to: LatLng): LatLng | undefined {
  const radius = wfig("world/domains/logistics.ROUTE_THREAT_M");
  const near = (p: LatLng) => distanceM(p, to) > 1500 && offLineM(p, from, to) < radius;
  const t = Object.values(c.world.tracks[side]).find((t) => t.domain === "land" && c.time - t.seenAt < 900 && near(t.at));
  if (t) return t.at;
  // What the side's own units on the ground have seen (the land engine's picture).
  const own = Object.keys(c.game.forceElements).filter((id) => c.fe(id)!.side === side && c.landAlive(id));
  for (const enemy of Object.keys(c.game.forceElements).sort()) {
    const fe = c.fe(enemy)!;
    if (fe.side === side || !c.landAlive(enemy) || !near(fe.position)) continue;
    if (own.some((id) => c.land.known(id, enemy) !== "none")) return fe.position;
  }
  return undefined;
}

function convoys(c: WorldCtx): void {
  const reach = wfig("world/domains/logistics.HANDOVER_M");
  for (const job of state(c).jobs) {
    if (job.means !== "convoy" || job.state === "done" || job.state === "failed") continue;
    const v = job.vehicleId!;
    if (!c.landAlive(v)) {
      setJob(c, job.id, { state: "failed" });
      c.log({ side: job.side, domain: "logistics", kind: "convoyLost", text: `the convoy for ${job.unitId} (${v}) is lost on the road`, ids: [v, job.unitId] });
      // The unit is still short: it asks again.
      if (c.landAlive(job.unitId)) setState(c, { requests: [...state(c).requests, { unitId: job.unitId, side: job.side, at: c.time, need: needOf(c, job.unitId) }] });
      continue;
    }
    const here = c.fe(v)!.position;
    if (job.state === "waiting") {
      const dark = !job.waitForDark || isNight(c.world.env, c.time) || c.time - job.startedAt > wfig("world/domains/logistics.MAX_WAIT_S");
      if (dark && (job.holdUntil ?? 0) <= c.time) {
        setJob(c, job.id, { state: "outbound", waitForDark: false });
        drive(c, v, job.via ?? c.fe(job.unitId)?.position ?? here);
      }
      continue;
    }
    if (job.state === "outbound") {
      if (!c.landAlive(job.unitId)) {
        setJob(c, job.id, { state: "returning" });
        drive(c, v, c.entity(job.nodeId)?.pos ?? here);
        continue;
      }
      const target = c.fe(job.unitId)!.position;
      if (job.via) {
        if (distanceM(here, job.via) < reach) {
          setJob(c, job.id, { via: undefined });
          drive(c, v, target);
        }
      } else if (c.time % 60 === 0) drive(c, v, target);
      if (distanceM(here, target) <= reach) {
        const share = deliver(c, job.nodeId, job.unitId, recordFor("generic/convoyTruck").carried.cargoT! * (c.land.units[v].vehicles.fit / Math.max(1, c.land.units[v].vehicles.total)));
        setJob(c, job.id, { state: "returning", deliveredAt: c.time });
        c.stat((s) => ({ ...s, resupplyS: [...(s.resupplyS ?? []), c.time - job.startedAt] }));
        c.log({ side: job.side, domain: "logistics", kind: "delivered", text: `${v} resupplies ${job.unitId} (${Math.round(share * 100)}% of what it needed) after ${Math.round((c.time - job.startedAt) / 60)} min`, ids: [v, job.unitId] });
        drive(c, v, c.entity(job.nodeId)?.pos ?? here);
        continue;
      }
      // The road ahead: asked once per job if a known enemy is near it.
      if (c.time % 30 === 0 && c.world.memo[`routeAsked:${job.id}`] == null) {
        const threat = routeThreat(c, job.side, here, job.via ?? target);
        if (threat) {
          c.world.memo[`routeAsked:${job.id}`] = c.time;
          raiseQuestion(c, { agentId: v, side: job.side, trigger: "routeThreatened", about: { jobId: job.id, lat: threat.lat, lng: threat.lng }, quality: c.fe(v)!.troopQuality });
        }
      }
      // A drone diving on the convoy.
      const diving = Object.values(c.world.entities).find((e) => e.owner === opposing(job.side) && e.status === "active" && e.task.kind === "strike" && e.task.targetId === v && distanceM(e.pos, here) < 1500);
      if (diving && c.world.memo[`droneAsked:${diving.id}`] == null) {
        c.world.memo[`droneAsked:${diving.id}`] = c.time;
        raiseQuestion(c, { agentId: v, side: job.side, trigger: "droneAttackOnConvoy", about: { jobId: job.id, droneId: diving.id }, quality: c.fe(v)!.troopQuality });
      }
      continue;
    }
    if (job.state === "returning") {
      const home = c.entity(job.nodeId)?.pos ?? c.world.startLines[job.side];
      if (distanceM(here, home) <= reach * 2) {
        setJob(c, job.id, { state: "done" });
        const s = state(c);
        setState(c, { pool: { ...s.pool, [job.side]: [...s.pool[job.side], v] } });
        // The trucks reload at the node: nothing to carry, the node's stocks are drawn on delivery.
        c.land.order(v, { kind: "hold" });
      } else if (c.time % 120 === 0) drive(c, v, home);
    }
  }
}

// ── Maintenance and recovery ─────────────────────────────────────────────

function maintenance(c: WorldCtx): void {
  if (c.time % 60 !== 0) return;
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (comps.disabled <= 0 || !c.landAlive(id)) continue;
    const key = `repair:${id}`;
    if (c.world.memo[key] == null) {
      c.world.memo[key] = c.time;
      continue;
    }
    const unit = c.land.units[id];
    // Recovery needs the unit to hold its ground long enough to drag the wreck back.
    if (unit.lastMovedAt > c.time - 60 && unit.order.kind !== "hold") {
      c.world.memo[key] = c.time;
      continue;
    }
    if (c.time - c.world.memo[key] < wfig("world/domains/logistics.REPAIR_S")) continue;
    delete c.world.memo[key];
    const cannibalise = comps.stores.spares <= 0;
    if (cannibalise && comps.disabled < 2) {
      c.setLand(id, { disabled: 0 });
      continue;
    }
    const fe = c.fe(id)!;
    const next = { ...unit.vehicles, fit: Math.min(unit.vehicles.total, unit.vehicles.fit + 1) };
    if (next.fit === unit.vehicles.fit) {
      c.setLand(id, { disabled: 0 });
      continue;
    }
    c.land.setUnit(id, { vehicles: next });
    c.land.setFe(id, { combatStrength: strengthFor(fe, next) });
    c.setLand(id, { disabled: comps.disabled - (cannibalise ? 2 : 1), stores: { ...comps.stores, spares: Math.max(0, comps.stores.spares - (cannibalise ? 0 : 1)) } });
    c.log({ side: fe.side, domain: "logistics", kind: "repaired", text: `${id} gets a vehicle back from recovery and repair${cannibalise ? " (another cannibalised for parts)" : ""}: ${next.fit} of ${next.total}`, ids: [id] });
  }
}

// ── Medical ──────────────────────────────────────────────────────────────

/** Chance a casualty dies of wounds, by how long evacuation took (a declared curve: the golden hour). */
export function diedOfWoundsP(evacS: number): number {
  const h = evacS / 3600;
  const base = wfig("world/domains/logistics.DOW_BASE");
  return Math.min(0.9, h <= 1 ? base : base + wfig("world/domains/logistics.DOW_PER_HOUR") * (h - 1));
}

function finishEvac(c: WorldCtx, ev: Evacuation): void {
  const times = ev.hitAt.map((t) => c.time - t);
  let died = 0;
  for (const t of times) if (c.chance(diedOfWoundsP(t))) died++;
  setEvac(c, ev.id, { state: "done", doneAt: c.time });
  c.stat((s) => ({ ...s, evacuationS: [...s.evacuationS, ...times], diedOfWounds: (s.diedOfWounds ?? 0) + died }));
  const comps = c.comps(ev.unitId);
  if (comps && died) c.setLand(ev.unitId, { killed: comps.killed + died });
  const mean = Math.round(times.reduce((s, t) => s + t, 0) / Math.max(1, times.length) / 60);
  c.log({ side: ev.side, domain: "medical", kind: "treated", text: `${ev.hitAt.length} casualties from ${ev.unitId} reach treatment after ${mean} min${died ? `; ${died} die of wounds` : ""}`, ids: [ev.unitId] });
}

function medical(c: WorldCtx): void {
  // Wounded waiting at units: asked once a batch has gathered (or after a while).
  if (c.time % 30 === 0) {
    for (const id of Object.keys(c.world.land).sort()) {
      const comps = c.world.land[id];
      if (!comps.wounded.length || !c.fe(id)) continue;
      if (state(c).evacs.some((e) => e.unitId === id && e.state !== "done")) continue;
      const oldest = Math.min(...comps.wounded);
      if (c.time - oldest < wfig("world/domains/logistics.CCP_S")) continue;
      const side = c.fe(id)!.side;
      const hitAt = [...comps.wounded];
      c.setLand(id, { wounded: [] });
      const from = c.fe(id)!.position;
      const to = treatmentAt(c, side, from);
      // Carried back from the point of injury to the collection point, out of direct fire.
      const ccp = offsetBy(from, bearingDeg(from, to), Math.min(distanceM(from, to) / 2, wfig("world/domains/logistics.CCP_BACK_M")));
      const ev: Evacuation = { id: nextId(c, "evac"), side, unitId: id, hitAt, means: "abstract", state: "waiting", to, ccp };
      setState(c, { evacs: [...state(c).evacs, ev] });
      c.world.memo[`evacAsked:${ev.id}`] = c.time;
      if (!c.simulated("medical")) {
        // Abstracted: each batch takes a drawn time, from the reference band.
        const lo = wfig("world/domains/logistics.EVAC_ABS_MIN_S");
        const hi = wfig("world/domains/logistics.EVAC_ABS_MAX_S");
        c.schedule(oldest + lo + c.rng.int(Math.max(1, hi - lo)), "evacDone", ev.id);
        continue;
      }
      if (!c.landAlive(id)) {
        // Nobody left to decide: the medical chain comes for them.
        dispatchAmbulance(c, ev.id);
        continue;
      }
      const underFire = c.time - (c.land.units[id]?.lastIncomingAt ?? -Infinity) < 120;
      raiseQuestion(c, { agentId: id, side, trigger: "casualtiesWaiting", about: { evacId: ev.id, count: hitAt.length, underFire }, quality: c.fe(id)!.troopQuality });
    }
  }
  if (!c.simulated("medical")) return;
  for (const ev of state(c).evacs) {
    // A batch nobody decided about (its unit busy or gone): the ambulance comes anyway.
    if (ev.state === "waiting" && ev.means === "abstract" && c.time - (c.world.memo[`evacAsked:${ev.id}`] ?? c.time) > wfig("world/domains/logistics.CCP_S")) {
      dispatchAmbulance(c, ev.id);
      continue;
    }
    if (ev.state === "done" || ev.means !== "ambulance") continue;
    const v = ev.vehicleId!;
    if (!c.landAlive(v)) {
      // The ambulance is lost: the casualties wait again, now later.
      setEvac(c, ev.id, { state: "done", doneAt: c.time });
      const comps = c.comps(ev.unitId);
      if (comps) c.setLand(ev.unitId, { wounded: [...comps.wounded, ...ev.hitAt] });
      continue;
    }
    const here = c.fe(v)!.position;
    if (ev.state === "waiting") {
      if (!ev.waitForDark || isNight(c.world.env, c.time) || c.time - Math.min(...ev.hitAt) > wfig("world/domains/logistics.MAX_WAIT_S")) {
        setEvac(c, ev.id, { state: "inbound", waitForDark: false });
        drive(c, v, ev.ccp);
      }
    } else if (ev.state === "inbound") {
      const at = ev.ccp;
      if (c.time % 60 === 0) drive(c, v, at);
      if (distanceM(here, at) <= wfig("world/domains/logistics.HANDOVER_M")) {
        setEvac(c, ev.id, { state: "carrying" });
        drive(c, v, ev.to);
      }
    } else if (ev.state === "carrying" && distanceM(here, ev.to) <= wfig("world/domains/logistics.HANDOVER_M") * 2) {
      finishEvac(c, ev);
      c.land.order(v, { kind: "hold" });
      const s = state(c);
      setState(c, { pool: { ...s.pool, [ev.side]: [...s.pool[ev.side], v] } });
    } else if (ev.state === "carrying" && c.time % 120 === 0) drive(c, v, ev.to);
  }
}

onScheduled("evacDone", (c, e) => {
  const ev = state(c).evacs.find((x) => x.id === e.key);
  if (ev && ev.state !== "done") finishEvac(c, ev);
});

/** An ambulance for this evacuation: a free one, or one from the treatment point. */
function ambulance(c: WorldCtx, ev: Evacuation): string {
  const s = state(c);
  const free = s.pool[ev.side].find((id) => c.comps(id)?.archetype === "generic/medevac" && c.landAlive(id));
  if (free) {
    setState(c, { pool: { ...s.pool, [ev.side]: s.pool[ev.side].filter((x) => x !== free) } });
    return free;
  }
  const placed = Object.keys(c.world.land)
    .filter((id) => c.world.land[id].archetype === "generic/medevac" && c.fe(id)?.side === ev.side && c.landAlive(id) && !state(c).evacs.some((e) => e.vehicleId === id && e.state !== "done"))
    .sort()[0];
  return placed ?? spawn(c, ev.side, "generic/medevac", ev.to, "Ambulance section");
}

function dispatchAmbulance(c: WorldCtx, evacId: string): void {
  const ev = state(c).evacs.find((e) => e.id === evacId);
  if (!ev) return;
  const v = ambulance(c, ev);
  setEvac(c, ev.id, { means: "ambulance", vehicleId: v, state: "inbound" });
  delete c.world.memo[`evacAsked:${ev.id}`];
  drive(c, v, ev.ccp);
}

// ── Handlers and rules ───────────────────────────────────────────────────

const jobOf = (c: WorldCtx, q: { about: Record<string, unknown> }) => state(c).jobs.find((j) => j.id === q.about.jobId);
const evacOf = (c: WorldCtx, q: { about: Record<string, unknown> }) => state(c).evacs.find((e) => e.id === q.about.evacId);
const reachOf = (q: { about: Record<string, unknown> }) => (q.about.distanceKm as number) * 1000;

registerHandlers({
  "logistics.convoy": {
    available: (c, q) => c.landAlive(q.about.unitId as string),
    apply: (c, q) => sendConvoy(c, q.side, q.about.unitId as string, q.about.nodeId as string),
  },
  "logistics.ugv": {
    available: (c, q) => c.landAlive(q.about.unitId as string) && reachOf(q) <= wfig("world/domains/logistics.UGV_REACH_M"),
    apply: (c, q) => sendSmall(c, q.side, q.about.unitId as string, q.about.nodeId as string, "ugv"),
  },
  "logistics.drone": {
    available: (c, q) => c.landAlive(q.about.unitId as string) && reachOf(q) <= wfig("world/domains/logistics.DRONE_REACH_M") && c.mode("drones") !== "off",
    apply: (c, q) => sendSmall(c, q.side, q.about.unitId as string, q.about.nodeId as string, "drone"),
  },
  "logistics.waitForDark": {
    available: (c, q) => c.landAlive(q.about.unitId as string) && !isNight(c.world.env, c.time),
    apply: (c, q) => sendConvoy(c, q.side, q.about.unitId as string, q.about.nodeId as string, true),
  },
  "logistics.pushOn": { apply: () => {} },
  "logistics.alternateRoute": {
    available: (c, q) => jobOf(c, q) != null,
    apply(c, q) {
      const job = jobOf(c, q)!;
      const threat = { lat: q.about.lat as number, lng: q.about.lng as number };
      const v = c.fe(job.vehicleId ?? "")?.position;
      if (!v) return;
      // Round the threat on the far side from it, keeping the general direction.
      const to = c.fe(job.unitId)?.position ?? threat;
      const side = (bearingDeg(v, to) + (bearingDeg(v, threat) > bearingDeg(v, to) ? -90 : 90) + 360) % 360;
      const via = offsetBy(threat, side, wfig("world/domains/logistics.ROUTE_THREAT_M") * 2.5);
      setJob(c, job.id, { via });
      drive(c, job.vehicleId!, via);
    },
  },
  "logistics.scatter": {
    available: (c, q) => jobOf(c, q) != null,
    apply(c, q) {
      const job = jobOf(c, q)!;
      // Vehicles spread off the road and stop: harder to find and to hit.
      c.mark(`hiding:${job.vehicleId}`);
      c.land.order(job.vehicleId!, { kind: "hold" });
      setJob(c, job.id, { state: "waiting", holdUntil: c.time + wfig("world/domains/logistics.SCATTER_S") });
      c.schedule(c.time + wfig("world/domains/logistics.SCATTER_S"), "unhide", job.vehicleId!);
    },
  },
  "logistics.takeCover": {
    available: (c, q) => jobOf(c, q) != null,
    apply(c, q) {
      const job = jobOf(c, q)!;
      const fe = c.fe(job.vehicleId!)!;
      const at = fe.position;
      const cover = nearestCover(fe, c.config, 400);
      c.land.order(job.vehicleId!, { kind: "move", to: cover ?? at, mode: "march" });
      c.mark(`hiding:${job.vehicleId}`);
      setJob(c, job.id, { state: "waiting", holdUntil: c.time + wfig("world/domains/logistics.SCATTER_S") });
      c.schedule(c.time + wfig("world/domains/logistics.SCATTER_S"), "unhide", job.vehicleId!);
    },
  },
  "logistics.relocateNode": {
    available: (c, q) => c.entity(q.about.nodeId as string)?.hp != null,
    apply(c, q) {
      const node = c.entity(q.about.nodeId as string)!;
      const away = bearingDeg(c.world.startLines[opposing(q.side)], node.pos);
      c.setEntity(node.id, { pos: offsetBy(node.pos, away, wfig("world/domains/logistics.RELOCATE_M")) });
      // Out of action while it moves.
      c.world.memo[`nodeDown:${node.id}`] = c.time + wfig("world/domains/logistics.RELOCATE_S");
      c.log({ side: q.side, domain: "logistics", kind: "relocate", text: `${node.label} moves ${Math.round(wfig("world/domains/logistics.RELOCATE_M") / 1000)} km back; out of action for ${Math.round(wfig("world/domains/logistics.RELOCATE_S") / 3600)} h`, ids: [node.id] });
    },
  },
  "logistics.disperseStocks": {
    apply(c, q) {
      c.mark(`dispersed:${q.about.nodeId}`);
      c.log({ side: q.side, domain: "logistics", kind: "disperse", text: `${q.about.nodeId} disperses its stocks: later hits cost less`, ids: [q.about.nodeId as string] });
    },
  },
  "logistics.repairAndStay": {
    apply(c, q) {
      const node = c.entity(q.about.nodeId as string);
      if (node) c.schedule(c.time + wfig("world/domains/logistics.NODE_REPAIR_S"), "nodeRepair", node.id);
    },
  },
  "medical.evacuateNow": {
    available: (c, q) => evacOf(c, q) != null,
    apply: (c, q) => dispatchAmbulance(c, q.about.evacId as string),
  },
  "medical.waitForDark": {
    available: (c, q) => evacOf(c, q) != null && !isNight(c.world.env, c.time),
    apply(c, q) {
      const ev = evacOf(c, q)!;
      setEvac(c, ev.id, { means: "ambulance", vehicleId: ambulance(c, ev), state: "waiting", waitForDark: true });
      delete c.world.memo[`evacAsked:${ev.id}`];
    },
  },
  "medical.requestUgv": {
    available: (c, q) => {
      const ev = evacOf(c, q);
      return ev != null && distanceM(ev.ccp, ev.to) <= wfig("world/domains/logistics.UGV_REACH_M");
    },
    apply(c, q) {
      // A robot to the unit and back: slow, but nobody else exposed.
      const ev = evacOf(c, q)!;
      const d = distanceM(ev.ccp, ev.to);
      setEvac(c, ev.id, { means: "ugv" });
      delete c.world.memo[`evacAsked:${ev.id}`];
      c.schedule(c.time + Math.round((2 * d) / (recordFor("generic/ugv").mover.cruiseKmh / 3.6)), "evacDone", ev.id);
    },
  },
});

onScheduled("unhide", (c, e) => {
  delete c.world.memo[`hiding:${e.key}`];
});
onScheduled("nodeRepair", (c, e) => {
  const node = c.entity(e.key);
  if (!node || node.hp <= 0) return;
  const full = recordFor(node.archetype).body.stayingPower;
  c.setEntity(node.id, { hp: Math.min(full, node.hp + 1) });
  c.world.memo[`nodeHp:${node.id}`] = Math.min(full, node.hp + 1);
});

registerRules({
  resupplyRequests: (c, q, options) => {
    // Robots or drones in the drone-filled last kilometres [ref R7, P19]; convoys otherwise; by day under threat, wait for dark.
    const dronesNear = Object.values(c.world.tracks[q.side]).some((t) => t.domain === "air" && t.truthId != null && c.entity(t.truthId)?.owner === opposing(q.side) && distanceM(t.at, c.fe(q.about.unitId as string)?.position ?? t.at) < 5000);
    if (dronesNear) return prefer(options, "logistics.ugv", "logistics.drone", "logistics.waitForDark", "logistics.convoy");
    if (q.about.threat && (q.about.need as number) > 15) return prefer(options, "logistics.waitForDark", "logistics.convoy");
    return prefer(options, "logistics.convoy");
  },
  routeThreatened: (_c, _q, options) => prefer(options, "logistics.alternateRoute", "logistics.pushOn"),
  droneAttackOnConvoy: (c, q, options) => {
    const v = jobOf(c, q)?.vehicleId;
    const covered = v && c.fe(v) ? inCover(c.config.terrain, c.fe(v)!.position) : false;
    return covered ? prefer(options, "logistics.takeCover", "logistics.scatter") : prefer(options, "logistics.scatter", "logistics.pushOn");
  },
  casualtiesWaiting: (c, q, options) => {
    // The golden hour: evacuate now unless the unit is under fire by day and a robot can do it.
    if (q.about.underFire && !isNight(c.world.env, c.time)) return prefer(options, "medical.requestUgv", "medical.evacuateNow");
    return prefer(options, "medical.evacuateNow");
  },
  nodeStruck: (c, q, options) => {
    const node = c.entity(q.about.nodeId as string);
    const full = node && hasRecord(node.archetype) ? recordFor(node.archetype).body.stayingPower : 1;
    // Hit hard: move it back. Hit once: disperse and repair.
    return node && node.hp < full / 2 ? prefer(options, "logistics.relocateNode", "logistics.disperseStocks") : prefer(options, "logistics.disperseStocks", "logistics.repairAndStay");
  },
});

registerSystem({
  name: "logistics",
  order: 88,
  when: (c) => c.simulated("logistics") || medicalOn(c),
  run(c) {
    if (!c.world.logistics) c.world.logistics = state(c);
    if (simulatedLog(c)) {
      rail(c);
      nodeHits(c);
      logCycle(c);
      convoys(c);
      maintenance(c);
    }
    if (medicalOn(c)) medical(c);
  },
});
