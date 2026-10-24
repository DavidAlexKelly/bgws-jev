// ── world/domains/killweb.ts ─────────────────────────────────────────────
// The joint kill web (MULTI_DOMAIN_PLAN.md W11): any sensor, any shooter,
// through real networks and real staff decisions. Its speed is measured,
// not set.
//
//   NOMINATION   a track in an HQ's picture that is on the high-payoff target
//                list and meets the target selection standards (location
//                error, age) becomes a nomination. Over a data link the HQ
//                sees it as the sensor does and its cell decides in a minute
//                or two; up and down the voice nets each echelon's staff
//                handles it in turn [ref R3, P2: minutes against hours].
//   OPTIONS      every shooter the cells can task — guns, rockets, drone
//                teams, aircraft, ships and submarines — in range, with
//                ammunition, gets four figures by rule: time to effect,
//                expected effect, cost and risk. The cell's choice (Jev or
//                the rules: "cheapest effective") tasks it; or it defers.
//   TRACKING     the shooter aims where the track says the target is. A
//                target that moved while the chain ran is missed: slow chains
//                are punished by moving targets.
//   BDA          after the strike a sensor must look. Until something of the
//                side's sees the spot again, the HQ does not know; after a
//                wait the cell is asked (bdaUnclear): strike again, look
//                again, or move on — both re-striking the dead and leaving
//                the living are possible.
//   MEASUREMENT  each chain's step times (find → fix → target → engage →
//                assess) go to the AAR and to stats.killChainS.

import type { LatLng } from "../../../lib/board";
import { SIDES, opposing, type Side } from "../../../lib/state";
import { hasRecord, recordFor, weaponSpec } from "../../catalogue";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { distanceM, offsetBy, bearingDeg } from "../geo";
import { onImpact } from "../hooks";
import { deliveredImagery } from "../imagery";
import { launch } from "../munitions";
import { raiseQuestion } from "../questions";
import { onScheduled, registerSystem } from "../step";
import type { Headquarters, KillChain, WorldTrack } from "../types";
import { assignMission, isAircraft } from "./air";
import { launchDrone } from "./drones";
import { requestFire, shootersOf } from "./fires";

/** What is worth a strike by default, by archetype: command, fires, air defence, sensors, strike systems, sites. */
const DEFAULT_HPTL = [/cp-/, /sph155|mlrs|mortar/, /sam|shorad/, /cbRadar|ewJammer/, /coastalAsm/, /depot|railhead|airbase|port/];

const truthArchetype = (c: WorldCtx, id: string | undefined) => (id ? (c.comps(id)?.archetype ?? c.entity(id)?.archetype ?? "") : "");
const isFixed = (archetype: string) => hasRecord(archetype) && recordFor(archetype).mover.kind === "static";
const alive = (c: WorldCtx, id: string) => c.landAlive(id) || ["active", "landed"].includes(c.entity(id)?.status ?? "") && (c.entity(id)?.hp ?? 1) > 0;

function network(c: WorldCtx, side: Side): "voice" | "datalink" {
  return c.world.network?.[side] ?? "datalink";
}

/** How many HQs a nomination climbs to reach the top one, where strikes are approved on voice nets. */
function echelonsAbove(c: WorldCtx, hq: Headquarters): number {
  let n = 0;
  for (let h: Headquarters | undefined = hq; h?.parent && n < 6; h = c.world.hqs[h.parent]) n++;
  return n;
}

/** The staff time to turn a track into a nomination, by network: the reference contrast lives here. */
function staffDelayS(c: WorldCtx, hq: Headquarters): number {
  if (network(c, hq.side) === "datalink") return wfig("world/domains/killweb.DATALINK_STAFF_S");
  return wfig("world/domains/killweb.VOICE_STAFF_S") * (1 + echelonsAbove(c, hq));
}

function setChain(c: WorldCtx, id: string, patch: Partial<KillChain>): void {
  c.world.chains = c.world.chains.map((k) => (k.id === id ? { ...k, ...patch } : k));
  const k = c.world.chains.find((x) => x.id === id);
  if (k && patch.result && k.truthId) c.mark(`renominate:${k.side}:${k.truthId}`);
}

/** Does this track meet the target selection standards: fresh and precise enough to strike on? */
function meetsTss(c: WorldCtx, t: WorldTrack): boolean {
  const fixed = isFixed(truthArchetype(c, t.truthId));
  const maxAge = fixed ? wfig("world/domains/killweb.TSS_AGE_FIXED_S") : wfig("world/domains/killweb.TSS_AGE_S");
  return t.errorM <= wfig("world/domains/killweb.TSS_ERROR_M") && c.time - t.seenAt <= maxAge;
}

function onHptl(c: WorldCtx, t: WorldTrack): boolean {
  const a = truthArchetype(c, t.truthId);
  return a !== "" && DEFAULT_HPTL.some((p) => p.test(a));
}

// ── Nominations ──────────────────────────────────────────────────────────

function nominations(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const side of SIDES) {
    // The HQs that run targeting: those with no HQ above them, and battalions and up.
    const cells = Object.values(c.world.hqs)
      .filter((h) => h.side === side && !h.down && c.landAlive(h.postId) && (!h.parent || ["battalion", "brigade", "division", "corps", "component", "joint"].includes(h.echelon)))
      .sort((a, b) => a.id.localeCompare(b.id));
    for (const hq of cells) {
      for (const t of Object.values(hq.cop)) {
        if (!t.truthId || !alive(c, t.truthId)) continue;
        const firstKey = `found:${side}:${t.truthId}`;
        if (c.world.memo[firstKey] == null) c.world.memo[firstKey] = t.seenAt;
        if (!onHptl(c, t) || !meetsTss(c, t)) continue;
        if (c.world.chains.some((k) => k.side === side && k.truthId === t.truthId && k.result == null)) continue;
        // A target deferred, or just struck and missed, waits before it is nominated again.
        if (c.time - c.last(`renominate:${side}:${t.truthId}`) < wfig("world/domains/killweb.RENOMINATE_S")) continue;
        // A chain is open from the moment the target was first found; fixed now; nominated after the staff's time.
        const chain: KillChain = { id: `kc-${c.world.nextId}`, side, targetTrackId: t.id, truthId: t.truthId, found: c.world.memo[firstKey], fixed: c.time, network: network(c, side), costUsd: 0 };
        (c.world as { nextId: number }).nextId = c.world.nextId + 1;
        c.world.chains = [...c.world.chains, chain];
        c.schedule(c.time + staffDelayS(c, hq), "nominate", chain.id, { hq: hq.id });
      }
    }
  }
}

onScheduled("nominate", (c, e) => {
  const chain = c.world.chains.find((k) => k.id === e.key);
  const hq = c.world.hqs[String(e.data?.hq)];
  if (!chain || chain.result != null || !hq || hq.down || !c.landAlive(hq.postId)) {
    if (chain && chain.result == null) setChain(c, chain.id, { result: "deferred" });
    return;
  }
  if (!alive(c, chain.truthId!)) {
    setChain(c, chain.id, { result: "stale" });
    return;
  }
  setChain(c, chain.id, { targeted: c.time });
  const t = trackOf(c, chain.side, chain.truthId!);
  raiseQuestion(c, { agentId: hq.postId, side: chain.side, trigger: "nomination", about: { chainId: chain.id, targetId: chain.truthId!, thought: t?.thought ?? "target", lat: t?.at.lat ?? 0, lng: t?.at.lng ?? 0 }, quality: c.fe(hq.postId)?.troopQuality ?? 4 });
});

/** The freshest the side knows of a target: its tracks, its HQs' pictures, its imagery. */
function trackOf(c: WorldCtx, side: Side, truthId: string): WorldTrack | undefined {
  const all = [
    ...Object.values(c.world.tracks[side]),
    ...Object.values(c.world.hqs).filter((h) => h.side === side).flatMap((h) => Object.values(h.cop)),
    ...deliveredImagery(c, side),
  ].filter((t) => t.truthId === truthId);
  return all.sort((a, b) => b.seenAt - a.seenAt)[0];
}

// ── Shooter options ──────────────────────────────────────────────────────

export interface ShooterOption {
  shooterId: string;
  means: "fires" | "drone" | "air" | "naval";
  weaponId: string;
  tteS: number;
  pEffect: number;
  costUsd: number;
  risk: number;
}

/** Every shooter the side can task against this target now, with the four figures (W11). */
export function shooterOptions(c: WorldCtx, side: Side, truthId: string, at: LatLng): ShooterOption[] {
  const out: ShooterOption[] = [];
  const archetype = truthArchetype(c, truthId);
  const hard = hasRecord(archetype) && ["armoured", "ship", "structure"].includes(recordFor(archetype).body.vulnerability);
  const adNear = Object.values(c.world.tracks[side]).some((t) => /sam|shorad/.test(truthArchetype(c, t.truthId)) && distanceM(t.at, at) < 40_000 && alive(c, t.truthId ?? ""));
  // Guns and rockets.
  if (c.mode("fires") !== "off") {
    for (const id of shootersOf(c, side)) {
      const from = c.where(id);
      if (!from) continue;
      const mounts = c.comps(id)?.mounts ?? c.entity(id)?.mounts ?? [];
      const w = mounts.map((m) => weaponSpec(m.weaponId)).find((x) => ["mortar", "howitzer", "rocket"].includes(x.kind) && distanceM(from, at) <= x.maxRangeM && distanceM(from, at) >= x.minRangeM);
      if (!w) continue;
      out.push({ shooterId: id, means: "fires", weaponId: w.id, tteS: wfig("world/domains/killweb.FIRES_SETUP_S") + distanceM(from, at) / Math.max(1, w.speedMs), pEffect: hard ? 0.25 : 0.5, costUsd: w.costUsd * 6, risk: 0.2 });
    }
  }
  // Drone teams with strike drones that reach.
  if (c.simulated("drones")) {
    for (const id of Object.keys(c.world.land).sort()) {
      const comps = c.world.land[id];
      if (!c.landAlive(id) || c.fe(id)!.side !== side || comps.drones <= 0) continue;
      if (!["generic/fpvTeamRf", "generic/fpvTeamFibre", "generic/loiterTeam"].includes(comps.archetype)) continue;
      const rec = recordFor(comps.archetype);
      const drone = rec.carried.drones?.archetype;
      if (!drone) continue;
      const dr = recordFor(drone);
      const reach = (dr.mover.cruiseKmh / 3.6) * (dr.mover.enduranceH ?? 0.3) * 3600 * 0.45;
      const d = distanceM(c.fe(id)!.position, at);
      if (d > reach) continue;
      out.push({ shooterId: id, means: "drone", weaponId: dr.weapons[0]?.id ?? "fpv", tteS: 60 + d / (dr.mover.cruiseKmh / 3.6), pEffect: comps.archetype === "generic/loiterTeam" ? 0.5 : hard ? 0.2 : 0.3, costUsd: dr.costUsd, risk: 0 });
    }
  }
  // Aircraft: idle strike-capable flights, on the ground or up.
  if (c.simulated("air")) {
    for (const f of Object.values(c.world.entities)) {
      if (!isAircraft(f) || f.owner !== side || f.fit <= 0 || f.air?.mission || (f.status !== "active" && f.status !== "landed")) continue;
      // Air defence is suppressed with anti-radiation missiles (SEAD); the rest is struck with bombs or missiles.
      const ad = /sam|shorad/.test(archetype);
      const mount = (ad ? f.mounts.find((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === "arm") : undefined) ?? f.mounts.find((m) => m.rounds > 0 && ["glideBomb", "agm"].includes(weaponSpec(m.weaponId).kind));
      if (!mount) continue;
      const speed = recordFor(f.archetype).mover.cruiseKmh / 3.6;
      const ready = f.status === "landed" ? Math.max(0, (f.air?.readyAt ?? 0) - c.time) + 300 : 0;
      out.push({ shooterId: f.id, means: "air", weaponId: mount.weaponId, tteS: ready + distanceM(f.pos, at) / speed, pEffect: weaponSpec(mount.weaponId).kind === "arm" ? 0.5 : 0.6, costUsd: weaponSpec(mount.weaponId).costUsd * 2, risk: adNear ? 0.6 : 0.1 });
    }
  }
  // Ships and submarines: cruise missiles at land targets, anti-ship missiles at ships.
  if (c.simulated("naval")) {
    const atSea = c.entity(truthId)?.domain === "surface";
    for (const s of Object.values(c.world.entities)) {
      if (s.owner !== side || s.status !== "active" || (s.domain !== "surface" && s.domain !== "subsurface")) continue;
      const mount = s.mounts.find((m) => m.rounds > 0 && weaponSpec(m.weaponId).kind === (atSea ? "ascm" : "cruise"));
      if (!mount) continue;
      const w = weaponSpec(mount.weaponId);
      const d = distanceM(s.pos, at);
      if (d > w.maxRangeM) continue;
      out.push({ shooterId: s.id, means: "naval", weaponId: w.id, tteS: 60 + d / w.speedMs, pEffect: 0.7, costUsd: w.costUsd * 2, risk: 0 });
    }
  }
  return out.sort((a, b) => a.costUsd - b.costUsd || a.tteS - b.tteS || a.shooterId.localeCompare(b.shooterId));
}

/** Task the chosen shooter: aimed where the side believes the target is. */
function engage(c: WorldCtx, chain: KillChain, o: ShooterOption, at: LatLng): void {
  const side = chain.side;
  const target = chain.truthId!;
  switch (o.means) {
    case "fires":
      requestFire(c, { side, requestedBy: o.shooterId, targetAt: at, targetId: target, priority: 4, found: chain.found, chainId: chain.id, heardAt: c.time, reason: `kill chain ${chain.id}` });
      break;
    case "drone":
      launchDrone(c, o.shooterId, { kind: "strike", targetId: target, aimAt: at, altM: 100 }, `kill chain ${chain.id}`);
      break;
    case "air": {
      const f = c.entity(o.shooterId)!;
      const w = weaponSpec(o.weaponId);
      const sead = w.kind === "arm";
      assignMission(c, f, { kind: sead ? "sead" : "strike", at, radiusM: sead ? 5000 : 2000, targetId: target, altM: w.kind === "agm" ? 300 : sead ? 8000 : 9000, until: c.time + wfig("world/domains/air.SORTIE_S"), releaseAt: offsetBy(at, bearingDeg(at, f.pos), w.maxRangeM * (sead ? 0.6 : 0.8)) });
      break;
    }
    case "naval": {
      const s = c.entity(o.shooterId)!;
      const w = weaponSpec(o.weaponId);
      for (let i = 0; i < 2; i++) launch(c, { launcherId: s.id, owner: side, weaponId: w.id, from: s.pos, fromAltM: s.domain === "subsurface" ? -20 : 10, targetId: target, aimAt: at, chainId: chain.id });
      c.setEntity(s.id, { mounts: s.mounts.map((m) => (m.weaponId === w.id ? { ...m, rounds: m.rounds - 2 } : m)), lastFiredAt: c.time });
      break;
    }
  }
  setChain(c, chain.id, { engaged: c.time, effectAt: c.time + o.tteS, shooterId: o.shooterId, means: o.means, costUsd: chain.costUsd + o.costUsd, tracked: trackOf(c, side, target)?.seenAt ?? chain.fixed });
  c.log({ side, domain: "killweb", kind: "engage", text: `kill chain ${chain.id}: ${o.shooterId} (${o.means}) on ${target}, ${Math.round(o.tteS / 60)} min to effect, $${Math.round(o.costUsd / 1000)}k`, ids: [o.shooterId, target] });
}

// ── Battle damage assessment ─────────────────────────────────────────────

/** Has anything of the side's looked at this spot since `since`: a fresh track of the target, a sensor close by, an image? */
function looked(c: WorldCtx, side: Side, truthId: string, at: LatLng, since: number): boolean {
  const t = trackOf(c, side, truthId);
  if (t && t.seenAt > since) return true;
  const eyes = wfig("world/domains/killweb.LOOK_M");
  if (Object.values(c.world.entities).some((e) => e.owner === side && e.status === "active" && (e.domain === "air" || e.domain === "surface") && e.domain !== undefined && distanceM(e.pos, at) < eyes && hasRecord(e.archetype) && recordFor(e.archetype).sensors.length > 0)) return true;
  if (Object.keys(c.game.forceElements).some((id) => c.fe(id)!.side === side && c.landAlive(id) && distanceM(c.fe(id)!.position, at) < eyes / 3)) return true;
  return (c.world.imagery ?? []).some((b) => b.side === side && b.takenAt > since && b.deliveredAt <= c.time && distanceM(b.at, at) < 15_000);
}

function assess(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const k of c.world.chains) {
    if (k.engaged == null || k.result != null) continue;
    const t = trackOf(c, k.side, k.truthId!);
    const at = t?.at ?? { lat: 0, lng: 0 };
    // Not before the effect is due, nor while the chain's rounds or missions are still under way.
    const since = Math.max(k.engaged, k.effectAt ?? k.engaged, c.world.memo[`impact:${k.id}`] ?? -Infinity) + wfig("world/domains/killweb.EFFECT_WAIT_S");
    if (c.time < since) continue;
    if (Object.values(c.world.entities).some((m) => m.status === "active" && m.munition?.chainId === k.id)) continue;
    if (c.world.missions.some((m) => m.chainId === k.id && m.state !== "complete" && m.state !== "refused")) continue;
    if (k.means === "air" && Object.values(c.world.entities).some((f) => f.air?.mission?.targetId === k.truthId && !f.air?.winchester)) continue;
    if (looked(c, k.side, k.truthId!, at, since)) {
      const dead = !alive(c, k.truthId!);
      const result = dead ? "destroyed" : "missed";
      setChain(c, k.id, { assessed: c.time, result });
      const done = { ...k, assessed: c.time };
      c.stat((s) => ({ ...s, killChainS: { blue: s.killChainS?.blue ?? [], red: s.killChainS?.red ?? [], [k.side]: [...(s.killChainS?.[k.side] ?? []), done.engaged! - done.found] } }));
      c.log({ side: k.side, domain: "killweb", kind: "assessed", text: `kill chain ${k.id} on ${k.truthId}: ${result} — found→fix ${mins(k.fixed! - k.found)}, fix→target ${mins((k.targeted ?? k.fixed!) - k.fixed!)}, target→engage ${mins(k.engaged - (k.targeted ?? k.fixed!))}, engage→assess ${mins(c.time - k.engaged)}`, ids: [k.truthId!] });
      continue;
    }
    if (c.time - since > wfig("world/domains/killweb.BDA_WAIT_S") && c.world.memo[`bda:${k.id}`] == null) {
      c.mark(`bda:${k.id}`);
      const hq = Object.values(c.world.hqs).find((h) => h.side === k.side && !h.down && c.landAlive(h.postId) && h.cop[k.truthId!] != null) ?? Object.values(c.world.hqs).find((h) => h.side === k.side && !h.down && c.landAlive(h.postId));
      if (hq) raiseQuestion(c, { agentId: hq.postId, side: k.side, trigger: "bdaUnclear", about: { chainId: k.id, targetId: k.truthId!, lat: at.lat, lng: at.lng }, quality: c.fe(hq.postId)?.troopQuality ?? 4 });
    }
  }
}

const mins = (s: number) => `${Math.max(0, Math.round(s / 60))} min`;

/** A missile of the chain hitting its target: the engagement's effect time, for the AAR. */
onImpact((c, m, outcome) => {
  const chainId = m.munition?.chainId;
  if (!chainId) return;
  c.world.memo[`impact:${chainId}`] = c.time;
  c.log({ side: m.owner, domain: "killweb", kind: outcome.hit ? "impact" : "miss", text: `kill chain ${chainId}: ${outcome.hit ? "impact" : "miss"}${outcome.killed ? ", target destroyed (not yet known)" : ""}`, ids: outcome.targetId ? [outcome.targetId] : [] });
});

// ── Handlers ─────────────────────────────────────────────────────────────

const chainOf = (c: WorldCtx, q: { about: Record<string, unknown> }) => c.world.chains.find((k) => k.id === q.about.chainId);

registerHandlers({
  "killweb.useShooter": {
    available: (c, q) => {
      const k = chainOf(c, q);
      return k != null && alive(c, k.truthId!) && shooterOptions(c, q.side, k.truthId!, { lat: q.about.lat as number, lng: q.about.lng as number }).length > 0;
    },
    // One option per shooter, with its four figures in the words.
    expand(c, q) {
      const k = chainOf(c, q);
      if (!k) return [];
      const opts = shooterOptions(c, q.side, k.truthId!, { lat: q.about.lat as number, lng: q.about.lng as number }).slice(0, 6);
      return opts.map((o) => ({ key: o.shooterId, summary: `${o.shooterId} (${o.means}): ${Math.round(o.tteS / 60)} min, ${Math.round(o.pEffect * 100)}% effect, $${Math.round(o.costUsd / 1000)}k, risk ${Math.round(o.risk * 100)}%`, params: { shooterId: o.shooterId, means: o.means, weaponId: o.weaponId, tteS: Math.round(o.tteS), pEffect: o.pEffect, costUsd: o.costUsd, risk: o.risk } }));
    },
    apply(c, q, option) {
      const k = chainOf(c, q);
      const params = option.params;
      if (!k || !params?.shooterId) return;
      const t = trackOf(c, q.side, k.truthId!);
      engage(c, k, params as unknown as ShooterOption, t?.at ?? { lat: q.about.lat as number, lng: q.about.lng as number });
    },
  },
  "killweb.defer": {
    apply(c, q) {
      const k = chainOf(c, q);
      if (k) setChain(c, k.id, { result: "deferred" });
    },
  },
  "killweb.restrike": {
    available: (c, q) => {
      const k = chainOf(c, q);
      return k != null && shooterOptions(c, q.side, k.truthId!, { lat: q.about.lat as number, lng: q.about.lng as number }).length > 0;
    },
    apply(c, q) {
      const k = chainOf(c, q);
      if (!k) return;
      const at = { lat: q.about.lat as number, lng: q.about.lng as number };
      const o = shooterOptions(c, q.side, k.truthId!, at).filter((x) => x.pEffect >= 0.3)[0];
      if (o) engage(c, k, o, at);
    },
  },
  "killweb.lookAgain": {
    apply(c, q) {
      const k = chainOf(c, q);
      if (!k) return;
      // An ISR drone sent over the spot, if a team can; the question comes again after another wait.
      const at = { lat: q.about.lat as number, lng: q.about.lng as number };
      const team = Object.keys(c.world.land).find((id) => c.world.land[id].archetype === "generic/droneIsrTeam" && c.landAlive(id) && c.fe(id)!.side === q.side && c.world.land[id].drones > 0 && distanceM(c.fe(id)!.position, at) < 10_000);
      if (team) launchDrone(c, team, { kind: "orbit", at, radiusM: 300, altM: 150, until: c.time + 900 }, `BDA for ${k.id}`);
      delete c.world.memo[`bda:${k.id}`];
      setChain(c, k.id, { effectAt: c.time });
    },
  },
  "killweb.moveOn": {
    apply(c, q) {
      const k = chainOf(c, q);
      if (k) setChain(c, k.id, { result: "unknown" });
    },
  },
});

registerRules({
  // Cheapest effective: the cheapest option likely enough to work, unless something far quicker is needed.
  nomination: (_c, _q, options) => {
    const shooters = options.filter((o) => o.action === "killweb.useShooter" && o.params);
    const effective = shooters.filter((o) => (o.params!.pEffect as number) >= 0.3 && (o.params!.risk as number) < 0.5);
    const pick = [...(effective.length ? effective : shooters)].sort((a, b) => (a.params!.costUsd as number) - (b.params!.costUsd as number) || (a.params!.tteS as number) - (b.params!.tteS as number))[0];
    return pick?.id ?? prefer(options, "killweb.defer");
  },
  bdaUnclear: (_c, _q, options) => prefer(options, "killweb.lookAgain", "killweb.moveOn"),
});

registerSystem({
  name: "killweb",
  order: 58,
  when: (c) => Object.keys(c.world.hqs).length > 0,
  run(c) {
    nominations(c);
    assess(c);
  },
});

void opposing;
