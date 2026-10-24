// ── realtime/world/munitions.ts ──────────────────────────────────────────
// Munitions in flight as entities (MULTI_DOMAIN_PLAN.md §4.7, W2): every
// round with a flight time over a couple of seconds — shells, rockets,
// missiles, interceptors, glide bombs, torpedoes. In flight they can be seen
// by radar (counter-battery tracks shells back to the gun), intercepted,
// decoyed, jammed or spoofed, which is what makes air defence, naval salvo
// combat and EW work without special cases.
//
// HOW EACH FLIES
//   ballistic, glide  aimed at a point at launch, scattered by its CEP
//                     (grown by poor GNSS for a GNSS-guided round); arrives
//                     at a time fixed at launch. A high arc, for the map and
//                     for the radar that sees it.
//   pursuit, cruise,  steer each second at the target where it is now (if it
//   seaSkim,          still has it), or at the aim point. Hits are found by
//   underwater        closest approach within the second, so nothing fast
//                     tunnels through its target.
// A hit is then a roll against the weapon's kill probability, reduced by
// whatever the target does about it (hit modifiers: a defending aircraft,
// a ship's decoys, a vehicle's cage).

import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import { recordFor, hasRecord, weaponSpec } from "../catalogue";
import type { WeaponSpec } from "../catalogue/schema";
import { structuralHits } from "../catalogue/weapons";
import type { WorldCtx } from "./context";
import { wfig } from "./figures";
import { closestApproachM, distanceM, groundAt, scatter, step3d, bearingDeg } from "./geo";
import { impactOnLand } from "./landEffects";
import { guidedCepM, pntQualityAt } from "./pnt";
import { registerSystem } from "./step";
import { HIT_MODIFIERS, IMPACT_LISTENERS } from "./hooks";
import type { Entity } from "./types";

export interface LaunchSpec {
  launcherId: string;
  owner: Side;
  weaponId: string;
  from: LatLng;
  fromAltM: number;
  targetId?: string;
  aimAt: LatLng;
  aimAltM?: number;
  missionId?: string;
  decoy?: boolean;
  /** The kill chain this round is part of (W11). */
  chainId?: string;
}

export { registerHitModifier, onImpact, type HitModifier, type ImpactListener } from "./hooks";

/** Put a round in the air. Its magazine is the launcher's business. */
export function launch(c: WorldCtx, spec: LaunchSpec): Entity {
  const weapon = weaponSpec(spec.weaponId);
  const id = c.newId(`m-${weapon.id}`);
  const range = distanceM(spec.from, spec.aimAt);
  const gnss = weapon.guidance === "gnss";
  const pnt = gnss ? pntQualityAt(c.world, spec.owner, spec.from) : 1;
  const pntAtTarget = gnss ? pntQualityAt(c.world, spec.owner, spec.aimAt) : 1;
  const ballistic = weapon.flight === "ballistic" || weapon.flight === "glide";
  // Where it will come down: aimed, then scattered. A guided round against a
  // fixed point is scattered by its guided CEP, grown by jamming at the target.
  const cep = gnss ? guidedCepM(weapon.cepM, Math.min(pnt, pntAtTarget)) : weapon.guidance === "none" ? weapon.cepM * (0.5 + range / Math.max(1, weapon.maxRangeM)) : weapon.cepM;
  const lands = ballistic ? scatter(spec.aimAt, cep, (n) => c.rng.int(n)) : spec.aimAt;
  const flightS = Math.max(2, range / Math.max(1, weapon.speedMs));
  const entity: Entity = {
    id,
    owner: spec.owner,
    archetype: `munition/${weapon.id}`,
    label: weapon.id,
    domain: "munition",
    pos: spec.from,
    altM: spec.fromAltM,
    headingDeg: bearingDeg(spec.from, spec.aimAt),
    speedMs: weapon.speedMs,
    status: "active",
    fit: 1,
    hp: 1,
    damage: { mobility: false, firepower: false, sensor: false, comms: false },
    task: spec.targetId ? { kind: "strike", targetId: spec.targetId, aimAt: lands, ...(spec.aimAltM != null ? { altM: spec.aimAltM } : {}) } : { kind: "strike", aimAt: lands },
    stores: { fuelL: 0, ammo: {}, batteryWh: 0, medical: 0, spares: 0 },
    mounts: [],
    parent: spec.launcherId,
    template: "core/world",
    emcon: "free",
    weaponControl: "free",
    munition: {
      weaponId: weapon.id,
      launcherId: spec.launcherId,
      launchedAt: c.time,
      ...(spec.targetId ? { targetId: spec.targetId } : {}),
      aimAt: lands,
      from: spec.from,
      ...(ballistic ? { impactAt: c.time + flightS } : {}),
      ...(spec.missionId ? { missionId: spec.missionId } : {}),
      pntFactor: pnt,
      ...(spec.decoy ? { decoy: true } : {}),
    },
    costUsd: weapon.costUsd,
    lastFiredAt: c.time,
    history: [],
  };
  c.addEntity(entity);
  c.stat((s) => {
    const m = s.munitions[weapon.id] ?? { fired: 0, hits: 0, kills: 0, intercepted: 0 };
    return { ...s, spentUsd: { ...s.spentUsd, [spec.owner]: s.spentUsd[spec.owner] + weapon.costUsd }, munitions: { ...s.munitions, [weapon.id]: { ...m, fired: m.fired + 1 } } };
  });
  return entity;
}

/** Where a platform (land unit or entity) is, and how high above sea level. */
export function positionOf(c: WorldCtx, id: string): { at: LatLng; altM: number } | undefined {
  const fe = c.fe(id);
  if (fe) return fe.combatStrength > 0 ? { at: fe.position, altM: groundAt(c.config.terrain, fe.position) + 2 } : undefined;
  const e = c.entity(id);
  if (!e || e.status === "destroyed" || e.status === "expended") return undefined;
  return { at: e.pos, altM: e.altM };
}

/** What a hit does to an entity (§4.8): aircraft lost one by one, ships and installations by staying power. */
export function damageEntity(c: WorldCtx, targetId: string, weapon: WeaponSpec, by: string): { killed: boolean; text: string } {
  const t = c.entity(targetId);
  if (!t) return { killed: false, text: "no target" };
  const owner = t.owner;
  const lose = (usd: number) => {
    if (owner === "blue" || owner === "red") c.stat((s) => ({ ...s, lostUsd: { ...s.lostUsd, [owner]: s.lostUsd[owner] + usd } }));
  };
  if (t.domain === "munition" || (t.domain === "air" && (t.fit <= 1 || !hasRecord(t.archetype) || recordFor(t.archetype).body.count === 1))) {
    c.setEntity(targetId, { status: "destroyed", fit: 0 });
    lose(t.costUsd);
    return { killed: true, text: `${t.label} destroyed` };
  }
  if (t.domain === "air") {
    const fit = t.fit - 1;
    const unitCost = hasRecord(t.archetype) ? recordFor(t.archetype).costUsd : t.costUsd;
    lose(unitCost);
    c.setEntity(targetId, { fit, ...(fit <= 0 ? { status: "destroyed" as const } : {}) });
    return { killed: fit <= 0, text: fit <= 0 ? `${t.label}: last aircraft shot down` : `${t.label}: one aircraft shot down, ${fit} left` };
  }
  const hits = structuralHits(weapon);
  const hp = t.hp - hits;
  const full = hasRecord(t.archetype) ? recordFor(t.archetype).body.stayingPower : 1;
  const damage = { ...t.damage };
  // Below half its staying power a ship or site starts losing systems.
  if (hp < full / 2) {
    if (c.chance(0.4)) damage.sensor = true;
    if (c.chance(0.3)) damage.mobility = true;
    if (c.chance(0.3)) damage.firepower = true;
    if (c.chance(0.2)) damage.comms = true;
  }
  const killed = hp <= 0;
  c.setEntity(targetId, { hp, damage, ...(killed ? { status: "destroyed" as const } : {}) });
  if (killed) lose(t.costUsd);
  void by;
  return { killed, text: killed ? `${t.label} ${t.domain === "node" ? "out of action" : "sunk"}` : `${t.label} hit (${Math.max(0, Math.round((hp / full) * 100))}% staying power left)` };
}

function finish(c: WorldCtx, m: Entity, outcome: { hit: boolean; killed: boolean; targetId?: string; at: LatLng }): void {
  c.setEntity(m.id, { status: "expended", pos: outcome.at });
  if (outcome.hit) {
    c.stat((s) => {
      const k = m.munition!.weaponId;
      const rec = s.munitions[k] ?? { fired: 0, hits: 0, kills: 0, intercepted: 0 };
      return { ...s, munitions: { ...s.munitions, [k]: { ...rec, hits: rec.hits + 1, kills: rec.kills + (outcome.killed ? 1 : 0) } } };
    });
  }
  for (const l of IMPACT_LISTENERS) l(c, c.entity(m.id)!, outcome);
}

/** Resolve a munition reaching its target or aim point. */
function arrive(c: WorldCtx, m: Entity, weapon: WeaponSpec, at: LatLng, targetId: string | undefined): void {
  const owner = m.owner === "neutral" ? "blue" : m.owner;
  if (m.munition?.decoy) return finish(c, m, { hit: false, killed: false, at });
  const target = targetId ? c.entity(targetId) : undefined;
  const intercept = target != null && (target.domain === "air" || target.domain === "munition");
  if (intercept) c.stat((s) => ({ ...s, interceptsTried: s.interceptsTried + 1 }));
  if (target) {
    let p = weapon.pk;
    for (const mod of HIT_MODIFIERS) p *= mod(c, m, weapon, targetId!);
    if (!c.chance(p)) {
      c.log({ side: owner, domain: "munitions", kind: "miss", text: `${weapon.id} from ${m.munition!.launcherId} missed ${target.label}`, ids: [m.munition!.launcherId, targetId!], at });
      return finish(c, m, { hit: false, killed: false, targetId, at });
    }
    const result = damageEntity(c, targetId!, weapon, m.munition!.launcherId);
    if (intercept && result.killed) c.stat((s) => ({ ...s, interceptsMade: s.interceptsMade + 1 }));
    if (intercept && target.domain === "munition") {
      c.stat((s) => {
        const k = target.munition?.weaponId ?? target.label;
        const rec = s.munitions[k] ?? { fired: 0, hits: 0, kills: 0, intercepted: 0 };
        return { ...s, munitions: { ...s.munitions, [k]: { ...rec, intercepted: rec.intercepted + (result.killed ? 1 : 0) } } };
      });
    }
    c.log({ side: owner, domain: "munitions", kind: "hit", text: `${weapon.id} from ${m.munition!.launcherId}: ${result.text}`, ids: [m.munition!.launcherId, targetId!], at });
    return finish(c, m, { hit: true, killed: result.killed, targetId, at });
  }
  // A land target (guided onto a unit) or a point on the ground.
  if (weapon.targets.includes("land")) {
    const direct = targetId && c.fe(targetId) ? targetId : undefined;
    let pk = 1;
    if (direct) for (const mod of HIT_MODIFIERS) pk *= mod(c, m, weapon, direct);
    const effective = pk < 1 && direct && !c.chance(pk) ? undefined : direct;
    const results = impactOnLand(c, at, weapon, { firerSide: owner, firerAt: m.munition!.from, targetId: effective, label: `${weapon.id} from ${m.munition!.launcherId}`, rounds: 1 });
    const knocked = results.reduce((s, r) => s + r.knocked, 0);
    return finish(c, m, { hit: knocked > 0 || results.length > 0, killed: knocked > 0, ...(direct ? { targetId: direct } : {}), at });
  }
  finish(c, m, { hit: false, killed: false, at });
}

/** One second of every munition in flight. */
export function stepMunitions(c: WorldCtx): void {
  const dt = c.config.timing.tickS;
  const ids = Object.keys(c.world.entities).filter((id) => {
    const e = c.world.entities[id];
    return e.domain === "munition" && e.status === "active";
  });
  for (const id of ids) {
    const m = c.entity(id);
    if (!m || m.status !== "active" || !m.munition) continue;
    const weapon = weaponSpec(m.munition.weaponId);
    const task = m.task.kind === "strike" ? m.task : undefined;
    const ballistic = weapon.flight === "ballistic" || weapon.flight === "glide";
    if (ballistic && m.munition.impactAt != null) {
      const total = Math.max(1, m.munition.impactAt - m.munition.launchedAt);
      const f = Math.min(1, (c.time - m.munition.launchedAt) / total);
      const from = m.munition.from;
      const to = m.munition.aimAt;
      const range = distanceM(from, to);
      const apex = weapon.flight === "glide" ? 8000 : Math.min(30_000, range / 4);
      const pos = { lat: from.lat + (to.lat - from.lat) * f, lng: from.lng + (to.lng - from.lng) * f };
      c.setEntity(id, { pos, altM: 4 * apex * f * (1 - f) });
      if (c.time >= m.munition.impactAt) arrive(c, c.entity(id)!, weapon, to, m.munition.targetId);
      continue;
    }
    // Guided: steer at the target where it is now, or at the aim point.
    const targetId = m.munition.targetId;
    const target = targetId ? positionOf(c, targetId) : undefined;
    // An anti-radiation missile needs its target emitting, or it flies on to where it last was.
    const seekerHas = target != null && (weapon.guidance !== "antiRadiation" || emitting(c, targetId!));
    const goal = seekerHas ? target!.at : (task?.aimAt ?? m.munition.aimAt);
    const goalAlt = seekerHas ? target!.altM : (task?.altM ?? groundAt(c.config.terrain, goal));
    const cruiseAlt = weapon.flight === "seaSkim" ? 5 : weapon.flight === "underwater" ? -30 : weapon.flight === "cruise" ? 60 : m.altM;
    const toAlt = distanceM(m.pos, goal) > weapon.speedMs * 8 ? Math.max(cruiseAlt, Math.min(goalAlt, cruiseAlt)) : goalAlt;
    const before = { at: m.pos, altM: m.altM };
    const next = step3d(m.pos, m.altM, goal, weapon.flight === "pursuit" ? goalAlt : toAlt, weapon.speedMs, dt);
    const flown = distanceM(m.munition.from, next.at);
    c.setEntity(id, { pos: next.at, altM: next.altM, headingDeg: bearingDeg(m.pos, goal) });
    // Did it pass its target inside this second?
    if (seekerHas) {
      const t0 = target!;
      const miss = closestApproachM(before, { at: next.at, altM: next.altM }, t0, t0);
      const reach = Math.max(weapon.lethalRadiusM, wfig("world/munitions.FUZE_M"));
      if (miss <= reach || next.arrived) {
        arrive(c, c.entity(id)!, weapon, next.at, targetId);
        continue;
      }
    } else if (next.arrived) {
      // Lost its target: it comes down at the aim point, or goes into the sea.
      arrive(c, c.entity(id)!, weapon, next.at, undefined);
      continue;
    }
    if (flown > weapon.maxRangeM * 1.2) {
      c.log({ side: m.owner, domain: "munitions", kind: "expended", text: `${weapon.id} from ${m.munition.launcherId} ran out of energy`, ids: [m.munition.launcherId] });
      finish(c, m, { hit: false, killed: false, at: next.at });
    }
  }
  // Spent rounds leave the world after a while: the map keeps a trace, the state does not grow for ever.
  for (const id of Object.keys(c.world.entities)) {
    const e = c.world.entities[id];
    if (e.domain === "munition" && e.status !== "active" && c.time - (e.munition?.impactAt ?? e.lastFiredAt) > wfig("world/munitions.KEEP_SPENT_S")) {
      delete c.world.entities[id];
    }
  }
}

/** Is this platform emitting now (radar on, transmitting)? For anti-radiation seekers and ESM. */
export function emitting(c: WorldCtx, id: string): boolean {
  if (c.world.emissions.some((e) => e.emitterId === id && (e.kind === "radar" || e.kind === "jammer"))) return true;
  const e = c.entity(id);
  const land = c.comps(id);
  const emcon = e?.emcon ?? land?.emcon ?? "free";
  if (emcon === "silent") return false;
  const archetype = e?.archetype ?? land?.archetype;
  return archetype != null && hasRecord(archetype) && recordFor(archetype).sensors.some((s) => s.active) && emcon === "free";
}

registerSystem({
  name: "munitions",
  order: 80,
  when: (c) => Object.values(c.world.entities).some((e) => e.domain === "munition"),
  run: stepMunitions,
});
