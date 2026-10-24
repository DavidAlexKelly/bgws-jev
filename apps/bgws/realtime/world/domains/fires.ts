// ── world/domains/fires.ts ───────────────────────────────────────────────
// Indirect fire (MULTI_DOMAIN_PLAN.md W3) and the fire-mission chain the
// kill web measures (W11):
//
//   request → fires cell → shooter assigned → its decision (fire now, lay
//   carefully, shoot and scoot, refuse) → lay → fire → time of flight →
//   impact → assessment
//
// WHO ASKS. A steady land unit that has found an enemy itself, out of reach
// of its own weapons or worth more than its own fire, calls for fire —
// once per enemy per couple of minutes. Drone teams and recce call too
// (drones.ts), and so does the counter-battery radar, at the gun that fired.
// The target is where the CALLER believes it is, off by the caller's own
// position error (which GNSS jamming grows, W6).
//
// WHO FIRES. Shooters are the side's indirect-fire units with rounds left,
// halted, in range. The cell takes requests by priority and gives each the
// nearest shooter that can reach; the shooter's leader (its template) then
// decides how. When the fires domain is ABSTRACTED there are no shooters on
// the board: a support service answers after its response time from off the
// board (§5.2), and its rounds still fly as entities.
//
// COUNTER-BATTERY. A counter-battery radar that tracks a shell back-plots
// the gun's position and calls fire on it; a gun that has fired from the same
// place too long decides whether to move (counterBatteryThreat).

import { opposing, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import type { WeaponSpec } from "../../catalogue/schema";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { bearingDeg, distanceM, offsetBy, scatter } from "../geo";
import { launch, onImpact } from "../munitions";
import { raiseQuestion } from "../questions";
import { onDetection } from "../sensing";
import { registerSystem } from "../step";
import type { FireMission } from "../types";
import type { LatLng } from "../../../lib/board";

const INDIRECT = new Set(["mortar", "howitzer", "rocket"]);

/** An indirect-fire weapon this unit or entity carries, with rounds, that reaches `to`. */
function mountFor(c: WorldCtx, shooterId: string, to: LatLng, preferGuided = false): { weapon: WeaponSpec; rounds: number } | undefined {
  const mounts = c.comps(shooterId)?.mounts ?? c.entity(shooterId)?.mounts ?? [];
  const from = c.where(shooterId);
  if (!from) return undefined;
  const range = distanceM(from, to);
  const usable = mounts
    .map((m) => ({ weapon: weaponSpec(m.weaponId), rounds: m.rounds }))
    .filter((m) => (INDIRECT.has(m.weapon.kind) || m.weapon.id === "navalGun") && m.rounds > 0 && range >= m.weapon.minRangeM && range <= m.weapon.maxRangeM);
  // Unguided first unless a precise round is wanted: precision rounds are few and dear.
  usable.sort((a, b) => (preferGuided ? Number(b.weapon.guidance !== "none") - Number(a.weapon.guidance !== "none") : Number(a.weapon.guidance !== "none") - Number(b.weapon.guidance !== "none")));
  return usable[0];
}

/** The side's indirect-fire shooters: land units with indirect mounts, steady and alive; ships with guns. */
export function shootersOf(c: WorldCtx, side: Side): string[] {
  const out: string[] = [];
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id) || c.fe(id)!.side !== side) continue;
    if (c.land.units[id].cohesion !== "steady" || comps.damage.firepower) continue;
    if (comps.mounts.some((m) => INDIRECT.has(weaponSpec(m.weaponId).kind) && m.rounds > 0)) out.push(id);
  }
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.owner !== side || e.status !== "active" || e.damage.firepower) continue;
    if (e.mounts.some((m) => m.weaponId === "navalGun" && m.rounds > 0)) out.push(id);
  }
  return out;
}

function busy(c: WorldCtx, shooterId: string): boolean {
  return c.world.missions.some((m) => m.shooterId === shooterId && (m.state === "assigned" || m.state === "laying" || m.state === "firing"));
}

/** Put in a call for fire. It reaches the fires cell at once, or when its message arrives (c2.ts). */
export function requestFire(
  c: WorldCtx,
  r: { side: Side; requestedBy: string; targetAt: LatLng; targetId?: string; priority: number; rounds?: number; found?: number; chainId?: string; heardAt?: number; reason?: string },
): FireMission {
  const mission: FireMission = {
    id: c.newId("fm"),
    side: r.side,
    requestedBy: r.requestedBy,
    requestedAt: c.time,
    targetAt: r.targetAt,
    ...(r.targetId ? { targetId: r.targetId } : {}),
    rounds: r.rounds ?? 0,
    state: "requested",
    times: { requested: c.time, ...(r.found != null ? { found: r.found } : {}) },
    priority: r.priority,
    ...(r.chainId ? { chainId: r.chainId } : {}),
    ...(r.reason ? { reason: r.reason } : {}),
  };
  (mission as FireMission & { heardAt?: number }).heardAt = r.heardAt ?? c.time + (ROUTE_DELAY ? ROUTE_DELAY(c, mission) : 0);
  c.world.missions.push(mission);
  c.log({ side: r.side, domain: "fires", kind: "request", text: `${r.requestedBy} calls for fire${r.targetId ? ` on ${r.targetId}` : ""}${r.reason ? ` (${r.reason})` : ""}`, ids: [r.requestedBy, ...(r.targetId ? [r.targetId] : [])], at: r.targetAt });
  return mission;
}

/** How long a call for fire takes to reach the fires cell: set by the C2 domain once it is loaded. */
let ROUTE_DELAY: ((c: WorldCtx, m: FireMission) => number) | null = null;
export function setFireRouting(delay: (c: WorldCtx, m: FireMission) => number): void {
  ROUTE_DELAY = delay;
}

function setMission(c: WorldCtx, id: string, patch: Partial<FireMission> & Record<string, unknown>): void {
  const i = c.world.missions.findIndex((m) => m.id === id);
  if (i >= 0) c.world.missions[i] = { ...c.world.missions[i], ...patch };
}

type Live = FireMission & { heardAt?: number; fireAt?: number; careful?: boolean; scoot?: boolean; left?: number; nextAt?: number; weaponId?: string; lastImpactAt?: number };

// ── Calls for fire ───────────────────────────────────────────────────────

function callsForFire(c: WorldCtx): void {
  for (const side of ["blue", "red"] as const) {
    const shooters = shootersOf(c, side);
    const abstracted = c.mode("fires") === "abstracted";
    if (!shooters.length && !abstracted) continue;
    const open = new Set(c.world.missions.filter((m) => m.side === side && m.state !== "complete" && m.state !== "refused" && m.targetId).map((m) => m.targetId!));
    for (const id of Object.keys(c.game.forceElements).sort()) {
      const fe = c.fe(id)!;
      if (fe.side !== side || !c.landAlive(id) || shooters.includes(id)) continue;
      const unit = c.land.units[id];
      if (unit.cohesion !== "steady") continue;
      for (const [enemyId, seen] of Object.entries(unit.ownSeen)) {
        if (seen.time !== c.time || open.has(enemyId) || !c.landAlive(enemyId)) continue;
        const key = `${id}:${enemyId}`;
        if (c.time - (c.last(key)) < wfig("world/domains/fires.RECALL_S")) continue;
        const range = distanceM(fe.position, c.fe(enemyId)!.position);
        // Worth calling for: beyond its own reach, or something firing on it.
        const ownReach = Math.max(0, ...fe.capabilities.map((cap) => cap.maxRangeM));
        const firingOnMe = c.time - (unit.attackers[enemyId] ?? -Infinity) < 60;
        if (range <= ownReach * 0.6 && !firingOnMe) continue;
        const aim = seen.at ?? c.fe(enemyId)!.position;
        if (!abstracted && !shooters.some((s) => mountFor(c, s, aim))) continue;
        c.mark(key);
        open.add(enemyId);
        const error = c.comps(id)?.positionErrorM ?? 10;
        requestFire(c, {
          side,
          requestedBy: id,
          targetAt: scatter(aim, error, (n) => c.rng.int(n)),
          targetId: enemyId,
          priority: (firingOnMe ? 3 : 1) + (c.fe(enemyId)!.targetClass === "armoured_vehicle" ? 1 : 0),
          found: seen.time,
          reason: firingOnMe ? "it is firing on us" : "seen beyond our reach",
        });
      }
    }
  }
}

// ── The fires cell: requests to shooters ─────────────────────────────────

function assign(c: WorldCtx): void {
  for (const side of ["blue", "red"] as const) {
    const waiting = (c.world.missions as Live[])
      .filter((m) => m.side === side && m.state === "requested" && (m.heardAt ?? 0) <= c.time)
      .sort((a, b) => b.priority - a.priority || a.requestedAt - b.requestedAt);
    if (!waiting.length) continue;
    if (c.mode("fires") === "abstracted") {
      for (const m of waiting) supportFire(c, m);
      continue;
    }
    const shooters = shootersOf(c, side).filter((s) => !busy(c, s) && !c.world.questions.some((q) => q.agentId === s && !q.answer));
    for (const m of waiting) {
      const candidates = shooters
        .filter((s) => mountFor(c, s, m.targetAt))
        .filter((s) => !(c.fe(s) && c.land.units[s].lastMovedAt === c.time))
        .sort((a, b) => distanceM(c.where(a)!, m.targetAt) - distanceM(c.where(b)!, m.targetAt));
      const shooter = candidates[0];
      if (!shooter) {
        if (c.time - m.requestedAt > wfig("world/domains/fires.GIVE_UP_S")) {
          setMission(c, m.id, { state: "refused", result: "no shooter could take it" });
          c.log({ side, domain: "fires", kind: "refused", text: `no shooter for ${m.requestedBy}'s call for fire`, ids: [m.requestedBy] });
        }
        continue;
      }
      shooters.splice(shooters.indexOf(shooter), 1);
      setMission(c, m.id, { state: "assigned", shooterId: shooter, times: { ...m.times, assigned: c.time } });
      const fe = c.fe(shooter);
      raiseQuestion(c, {
        agentId: shooter,
        side,
        trigger: "fireRequestReceived",
        about: { missionId: m.id, inRange: true, ...(m.targetId ? { targetId: m.targetId } : {}), lat: m.targetAt.lat, lng: m.targetAt.lng },
        quality: fe?.troopQuality ?? 4,
      });
    }
  }
}

/** Abstracted fires: an off-board battery answers after the service's response time. */
function supportFire(c: WorldCtx, m: Live): void {
  const service = c.world.support.find((s) => s.side === m.side && s.domain === "fires");
  if (!service) return;
  const hour = Math.floor(c.time / 3600);
  const svc = service.hour === hour ? service : { ...service, hour, used: 0 };
  if (svc.used >= svc.perHour) {
    if (c.time - m.requestedAt > wfig("world/domains/fires.GIVE_UP_S")) setMission(c, m.id, { state: "refused", result: "support used up this hour" });
    return;
  }
  const i = c.world.support.indexOf(service);
  c.world.support[i] = { ...svc, used: svc.used + 1, pending: [...svc.pending, { at: c.time, dueAt: c.time + svc.responseMin * 60, requestedBy: m.requestedBy, ...(m.targetId ? { targetId: m.targetId } : {}) }] };
  setMission(c, m.id, { state: "laying", shooterId: `support:${service.id}`, fireAt: c.time + svc.responseMin * 60, left: 4, weaponId: "howitzer155", times: { ...m.times, assigned: c.time } });
}

// ── Laying and firing ────────────────────────────────────────────────────

function fire(c: WorldCtx): void {
  for (const raw of c.world.missions as Live[]) {
    if (raw.state !== "laying" && raw.state !== "firing") continue;
    const m = c.world.missions.find((x) => x.id === raw.id) as Live;
    const shooter = m.shooterId!;
    const offBoard = shooter.startsWith("support:");
    if (!offBoard && !(c.landAlive(shooter) || c.entity(shooter)?.status === "active")) {
      setMission(c, m.id, { state: "refused", result: "the shooter was lost" });
      continue;
    }
    if ((m.fireAt ?? Infinity) > c.time || (m.nextAt ?? 0) > c.time) continue;
    // A gun on the move does not fire.
    if (!offBoard && c.fe(shooter) && c.land.units[shooter].lastMovedAt === c.time) continue;
    const from = offBoard ? offBoardPoint(c, m.side, m.targetAt) : c.where(shooter)!;
    const mount = offBoard ? { weapon: weaponSpec("howitzer155"), rounds: 99 } : mountFor(c, shooter, m.targetAt, m.priority >= 4);
    if (!mount) {
      setMission(c, m.id, { state: m.left != null && m.left < (m.rounds || Infinity) ? "inFlight" : "refused", result: "out of range or out of rounds" });
      continue;
    }
    const w = mount.weapon;
    const tubes = offBoard ? 4 : c.fe(shooter) ? c.land.units[shooter].vehicles.fit : 1;
    const left = m.left ?? (m.rounds || w.salvo * wfig("world/domains/fires.SALVOS")) * Math.max(1, tubes);
    const salvo = Math.min(left, Math.max(1, tubes) * (offBoard ? 1 : 1), mount.rounds);
    const careful = m.careful ? wfig("world/domains/fires.CAREFUL_CEP") : 1;
    for (let k = 0; k < salvo; k++) {
      // Each tube's round is aimed at the believed target; a carefully laid mission spreads less.
      const aim = careful < 1 ? m.targetAt : scatter(m.targetAt, w.cepM * 0.2, (n) => c.rng.int(n));
      launch(c, { launcherId: offBoard ? shooter : shooter, owner: m.side, weaponId: w.id, from, fromAltM: 2, aimAt: aim, ...(m.targetId && w.guidance !== "none" ? { targetId: undefined } : {}), missionId: m.id, ...(m.chainId ? { chainId: m.chainId } : {}) });
    }
    if (!offBoard) spend(c, shooter, w.id, salvo);
    const remaining = left - salvo;
    setMission(c, m.id, {
      state: remaining > 0 ? "firing" : "inFlight",
      left: remaining,
      weaponId: w.id,
      nextAt: c.time + w.reloadS,
      times: { ...m.times, fired: m.times.fired ?? c.time },
      lastImpactAt: c.time + distanceM(from, m.targetAt) / Math.max(1, w.speedMs),
    });
    if (!offBoard && c.fe(shooter)) c.land.setUnit(shooter, { lastShotAt: c.time });
    if (m.times.fired == null) {
      c.log({ side: m.side, domain: "fires", kind: "fire", text: `${shooter} fires ${w.id} for ${m.requestedBy}${m.targetId ? ` on ${m.targetId}` : ""}`, ids: [shooter, m.requestedBy], at: m.targetAt });
    }
  }
  // Missions whose last round has landed are complete; a gun that fired decides whether to move.
  for (const raw of c.world.missions as Live[]) {
    if (raw.state !== "inFlight" || (raw.lastImpactAt ?? Infinity) > c.time) continue;
    setMission(c, raw.id, { state: "complete", times: { ...raw.times, impact: raw.lastImpactAt } });
    const shooter = raw.shooterId!;
    if (shooter.startsWith("support:") || !c.landAlive(shooter)) continue;
    if (raw.scoot) {
      displace(c, shooter, "shoot and scoot");
    } else if (c.world.land[shooter]?.mounts.some((x) => x.rounds > 0)) {
      raiseQuestion(c, { agentId: shooter, side: raw.side, trigger: "counterBatteryThreat", about: { missionId: raw.id, located: firedFromHere(c, shooter) >= 2 }, quality: c.fe(shooter)!.troopQuality });
    }
  }
}

/** Missions this gun has fired since it last moved. */
function firedFromHere(c: WorldCtx, id: string): number {
  const lastMoved = c.land.units[id]?.lastMovedAt ?? -Infinity;
  return c.world.missions.filter((m) => m.shooterId === id && (m.times.fired ?? -Infinity) > lastMoved).length;
}

function offBoardPoint(c: WorldCtx, side: Side, target: LatLng): LatLng {
  const start = c.world.startLines[side];
  const away = bearingDeg(target, start);
  return offsetBy(start, away, wfig("world/domains/fires.OFF_BOARD_M"));
}

function spend(c: WorldCtx, id: string, weaponId: string, rounds: number): void {
  const comps = c.comps(id);
  const family = weaponSpec(weaponId).family;
  if (comps) {
    c.setLand(id, {
      mounts: comps.mounts.map((m) => (m.weaponId === weaponId ? { ...m, rounds: Math.max(0, m.rounds - rounds) } : m)),
      stores: { ...comps.stores, ammo: { ...comps.stores.ammo, [family]: Math.max(0, (comps.stores.ammo[family] ?? 0) - rounds) } },
    });
    return;
  }
  const e = c.entity(id);
  if (e) c.setEntity(id, { mounts: e.mounts.map((m) => (m.weaponId === weaponId ? { ...m, rounds: Math.max(0, m.rounds - rounds) } : m)) });
}

/** Move a gun a few hundred metres to a new position. */
function displace(c: WorldCtx, id: string, why: string): void {
  const fe = c.fe(id);
  if (!fe) return;
  const away = (c.rng.int(8) * 45 + 180 + (fe.facing ?? 0)) % 360;
  c.land.order(id, { kind: "move", to: offsetBy(fe.position, away, wfig("world/domains/fires.DISPLACE_M")), mode: "march" });
  c.log({ side: fe.side, domain: "fires", kind: "displace", text: `${id} displaces (${why})`, ids: [id], at: fe.position });
}

// ── Counter-battery: a radar tracks a shell back to its gun ─────────────

onDetection((c, o, t, _track, isNew) => {
  if (t.domain !== "munition" || !isNew) return;
  const rec = c.comps(o.id)?.archetype;
  if (!rec || !hasRecord(rec) || !recordFor(rec).sensors.some((s) => s.kind === "cbr")) return;
  const m = c.entity(t.id)?.munition;
  if (!m) return;
  const w = weaponSpec(m.weaponId);
  if (!INDIRECT.has(w.kind)) return;
  const key = `cb:${o.side}:${m.launcherId}`;
  if (c.time - (c.last(key)) < wfig("world/domains/fires.RECALL_S")) return;
  c.mark(key);
  const gunAt = scatter(m.from, wfig("world/domains/fires.CB_ERROR_M"), (n) => c.rng.int(n));
  c.log({ side: o.side, domain: "fires", kind: "counterBattery", text: `${o.id} tracks a ${w.id} round back to a gun ${Math.round(distanceM(o.at, gunAt) / 100) / 10} km away`, ids: [o.id], at: gunAt });
  if (c.simulated("fires") || c.mode("fires") === "abstracted") {
    requestFire(c, { side: o.side, requestedBy: o.id, targetAt: gunAt, ...(c.landAlive(m.launcherId) ? { targetId: m.launcherId } : {}), priority: 4, found: c.time, reason: "counter-battery" });
  }
});

// Rounds landing near one of the side's own guns: it knows it has been found.
onImpact((c, munition, outcome) => {
  if (!munition.munition) return;
  const enemy = munition.owner === "blue" ? "red" : munition.owner === "red" ? "blue" : null;
  if (!enemy) return;
  for (const id of shootersOf(c, enemy)) {
    const at = c.where(id);
    if (!at || !c.fe(id) || distanceM(at, outcome.at) > wfig("world/domains/fires.NEAR_MISS_M")) continue;
    raiseQuestion(c, { agentId: id, side: enemy, trigger: "counterBatteryThreat", about: { located: true }, quality: c.fe(id)!.troopQuality });
  }
});

// ── Handlers ─────────────────────────────────────────────────────────────

function missionOf(c: WorldCtx, q: { about: Record<string, unknown> }): Live | undefined {
  return c.world.missions.find((m) => m.id === q.about.missionId) as Live | undefined;
}

function layS(c: WorldCtx, id: string): number {
  const mounts = c.comps(id)?.mounts ?? [];
  const kind = mounts.map((m) => weaponSpec(m.weaponId).kind)[0];
  return kind === "mortar" ? wfig("world/domains/fires.LAY_MORTAR_S") : wfig("world/domains/fires.LAY_GUN_S");
}

registerHandlers({
  "fires.fireNow": {
    available: (c, q) => missionOf(c, q) != null,
    describe: (c, q) => {
      const m = missionOf(c, q);
      return m ? `fire now for ${m.requestedBy} (${Math.round(distanceM(c.where(q.agentId)!, m.targetAt) / 100) / 10} km)` : "fire now";
    },
    apply(c, q) {
      const m = missionOf(c, q);
      if (m) setMission(c, m.id, { state: "laying", fireAt: c.time + layS(c, q.agentId) });
    },
  },
  "fires.fireWhenLaid": {
    available: (c, q) => missionOf(c, q) != null,
    apply(c, q) {
      const m = missionOf(c, q);
      if (m) setMission(c, m.id, { state: "laying", fireAt: c.time + 2 * layS(c, q.agentId), careful: true });
    },
  },
  "fires.fireThenScoot": {
    available: (c, q) => missionOf(c, q) != null && c.fe(q.agentId) != null,
    apply(c, q) {
      const m = missionOf(c, q);
      if (m) setMission(c, m.id, { state: "laying", fireAt: c.time + layS(c, q.agentId), scoot: true });
    },
  },
  "fires.refuse": {
    describe: () => "refuse the mission",
    apply(c, q) {
      const m = missionOf(c, q);
      if (!m) return;
      setMission(c, m.id, { state: "requested", shooterId: undefined, requestedAt: m.requestedAt });
      c.log({ side: q.side, domain: "fires", kind: "refuse", text: `${q.agentId} refuses ${m.requestedBy}'s mission`, ids: [q.agentId] });
    },
  },
  "fires.displaceNow": { available: (c, q) => c.fe(q.agentId) != null, apply: (c, q) => displace(c, q.agentId, "counter-battery threat") },
  "fires.finishThenDisplace": {
    apply(c, q) {
      // The next mission it fires ends with a move.
      const next = (c.world.missions as Live[]).find((m) => m.shooterId === q.agentId && m.state !== "complete" && m.state !== "refused");
      if (next) setMission(c, next.id, { scoot: true });
      else displace(c, q.agentId, "finished its missions");
    },
  },
  "fires.stay": { apply: () => {} },
});

registerRules({
  fireRequestReceived: (c, q, options) => {
    const m = missionOf(c, q);
    if (!m || !mountFor(c, q.agentId, m.targetAt)) return prefer(options, "fires.refuse");
    // A gun that has already fired from here twice shoots and scoots.
    if (firedFromHere(c, q.agentId) >= 2) return prefer(options, "fires.fireThenScoot", "fires.fireNow");
    return prefer(options, "fires.fireNow");
  },
  counterBatteryThreat: (_c, q, options) => (q.about.located ? prefer(options, "fires.displaceNow") : prefer(options, "fires.finishThenDisplace", "fires.stay")),
});

registerSystem({
  name: "fires",
  order: 60,
  when: (c) => c.mode("fires") !== "off",
  run(c) {
    callsForFire(c);
    assign(c);
    fire(c);
  },
});

void opposing;
