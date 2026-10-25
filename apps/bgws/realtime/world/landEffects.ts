// ── realtime/world/landEffects.ts ────────────────────────────────────────
// What a world weapon does to land units: a shell, a rocket, a glide bomb,
// an FPV, a loitering munition, a cruise missile.
//
// The land engine resolves direct fire itself (engine.ts §5–6). Everything
// that arrives from elsewhere lands here, and does to a land unit exactly
// what the engine's own fire does: vehicles knocked out one by one, strength
// following the vehicles, suppression, and the same events — "under fire"
// from a shooter it has NOT located (so its leader faces D3: take cover,
// search, pull back), "hit", and a friend lost. Casualties are split into
// killed and wounded; the wounded wait for evacuation (W10).

import { bearingDeg, distanceM, type LatLng } from "../../lib/board";
import { inCover } from "../../lib/proceduralTerrain";
import { applyEffects } from "../../rules/apply";
import { strengthFor } from "../engine/engine";
import { compassWord } from "../engine/knowledge";
import { recordFor } from "../catalogue";
import type { WeaponSpec } from "../catalogue/schema";
import type { WorldCtx } from "./context";
import { wfig } from "./figures";

/** Who suffers from an impact, and how. */
export interface ImpactResult {
  unitId: string;
  rangeM: number;
  knocked: number;
  suppressed: number;
}

/** How hard a warhead is on this kind of target: 1 is a soft vehicle in the open. */
export function vulnerabilityOf(weapon: WeaponSpec, targetClass: string, armourMm: number | undefined, direct: boolean): number {
  const pen = weapon.penetrationMm;
  const armour = armourMm ?? (targetClass === "armoured_vehicle" ? 500 : 0);
  if (targetClass === "armoured_vehicle") {
    // A shaped charge from above meets a roof about a sixth as thick as the front.
    const facing = weapon.topAttack ? armour / 6 : armour;
    if (pen > 0 && pen >= facing) return 1;
    return direct ? wfig("world/landEffects.BLAST_VS_ARMOUR") * 2 : wfig("world/landEffects.BLAST_VS_ARMOUR");
  }
  if (targetClass === "soft_skin") return 1;
  // Foot, and light vehicles counted as soft_skin above; dispersed men are hard to kill wholesale.
  return targetClass === "foot" ? 0.8 : wfig("world/landEffects.BLAST_VS_LIGHT");
}

/**
 * A warhead arrives at `at`. `targetId` is the unit it was guided onto, if
 * any (an FPV, an ATGM, a loitering munition): that unit is struck directly;
 * everyone near is caught by blast as area fire is.
 */
export function impactOnLand(
  ctx: WorldCtx,
  at: LatLng,
  weapon: WeaponSpec,
  options: { firerSide: "blue" | "red" | "neutral"; firerAt?: LatLng; targetId?: string; label: string; rounds?: number },
): ImpactResult[] {
  const results: ImpactResult[] = [];
  const rounds = options.rounds ?? 1;
  const spread = wfig("world/landEffects.SPREAD_M");
  const reach = weapon.lethalRadiusM * 3 + spread + 50;
  const game = ctx.game;
  const ids = Object.keys(game.forceElements).sort();
  for (const id of ids) {
    const fe = game.forceElements[id];
    if (!ctx.landAlive(id)) continue;
    // A section carried in its vehicle shares its fate: hit with the carrier, not on its own.
    if (ctx.comps(id)?.carrier && ctx.comps(id)?.aggregated == null && isCarried(ctx, id)) continue;
    const d = distanceM(fe.position, at);
    const direct = options.targetId === id;
    if (d > reach && !direct) continue;
    const unit = ctx.land.units[id];
    const comps = ctx.comps(id);
    const covered = inCover(ctx.config.terrain, fe.position) ? wfig("world/landEffects.COVER_FACTOR") : 1;
    const dug = 1 - wfig("world/landEffects.DUG_IN_FACTOR") * (comps?.dugIn ?? 0);
    const vuln = vulnerabilityOf(weapon, fe.targetClass, fe.armourMm, direct);
    let knocked = 0;
    const fit = unit.vehicles.fit;
    if (direct) {
      // Guided onto one vehicle: it is hit or it is not; blast takes no one else in the unit.
      for (let r = 0; r < rounds && knocked < fit; r++) if (ctx.chance(weapon.pk * vuln * (fe.targetClass === "foot" ? covered * dug : 1))) knocked++;
    } else {
      const radius = weapon.lethalRadiusM + spread;
      const pVehicle = weapon.pk * vuln * covered * dug * Math.exp(-((d / radius) ** 2));
      for (let r = 0; r < rounds; r++) {
        for (let v = 0; v < fit - knocked; v++) if (ctx.chance(pVehicle / Math.max(1, fit))) knocked++;
      }
    }
    knocked = Math.min(knocked, fit);
    const supp = wfig("world/landEffects.SUPPRESSION_NEAR") * Math.exp(-d / wfig("world/landEffects.SUPPRESSION_FALLOFF_M")) * Math.min(3, rounds) * covered;
    if (supp < 2 && knocked === 0) continue;
    applyHit(ctx, id, knocked, supp, { label: options.label, from: options.firerAt, firerSide: options.firerSide });
    results.push({ unitId: id, rangeM: d, knocked, suppressed: supp });
  }
  return results;
}

function isCarried(ctx: WorldCtx, id: string): boolean {
  const carrier = ctx.comps(id)?.carrier;
  if (!carrier) return false;
  const c = ctx.comps(carrier);
  return c != null && !c.dismounts.out.includes(id);
}

/**
 * Vehicles knocked out and suppression on one land unit, with everything the
 * engine's own fire does alongside: strength, losses, casualties, events and
 * the under-fire report on the net.
 */
export function applyHit(
  ctx: WorldCtx,
  id: string,
  knocked: number,
  suppression: number,
  why: { label: string; from?: LatLng; firerSide: "blue" | "red" | "neutral" },
): void {
  const fe = ctx.fe(id);
  const unit = ctx.land.units[id];
  if (!fe || !unit || fe.combatStrength <= 0) return;
  const time = ctx.time;
  const vehicles = unit.vehicles;
  const lost = Math.min(knocked, vehicles.fit);
  const quality = Math.max(0.5, 1.2 - fe.troopQuality * 0.05);
  ctx.land.setUnit(id, {
    suppression: Math.min(100, unit.suppression + suppression * quality),
    lastIncomingAt: time,
  });
  const bearing = why.from ? bearingDeg(fe.position, why.from) : undefined;
  const fromWhere = bearing != null ? ` from the ${compassWord(bearing)}` : "";
  let lostNow = 0;
  if (lost > 0) {
    const next = { ...vehicles, fit: vehicles.fit - lost };
    const strength = strengthFor(fe, next);
    lostNow = fe.combatStrength - strength;
    ctx.land.setUnit(id, { vehicles: next, losses: [...ctx.land.units[id].losses, ...Array(lost).fill(time)] });
    const game = applyEffects(ctx.game, [
      { kind: "combatStrength", feId: id, delta: -lostNow },
      ...(strength <= 0 ? [{ kind: "eliminated" as const, feId: id }] : []),
    ]);
    // Write back through setFe so the engine's own `game` follows.
    ctx.land.setFe(id, { combatStrength: game.forceElements[id].combatStrength, ...(strength <= 0 ? { morale: game.forceElements[id].morale } : {}) });
    casualties(ctx, id, lost);
    const side = fe.side;
    const start = ctx.world.startLines[side];
    if (start) {
      const depthKm = distanceM({ lat: fe.position.lat, lng: start.lng }, start) / 1000;
      ctx.stat((s) => ({ ...s, landLossDepthKm: [...s.landLossDepthKm, Math.round(depthKm * 10) / 10] }));
    }
    try {
      const archetype = ctx.comps(id)?.archetype;
      if (archetype) {
        const cost = recordFor(archetype).costUsd * lost;
        ctx.stat((s) => ({ ...s, lostUsd: { ...s.lostUsd, [side]: s.lostUsd[side] + cost } }));
      }
    } catch {
      // An archetype the catalogue does not hold costs nothing in the report.
    }
  }
  const after = ctx.fe(id)!;
  if (after.combatStrength <= 0) {
    for (const friend of Object.keys(ctx.game.forceElements).sort()) {
      const f = ctx.fe(friend)!;
      if (friend === id || f.side !== fe.side || !ctx.landAlive(friend)) continue;
      if (distanceM(f.position, fe.position) > 1000) continue;
      ctx.land.setUnit(friend, { lastFriendLostAt: time });
      ctx.land.emit(friend, "friendLost", `${id} destroyed by ${why.label} ${Math.round(distanceM(f.position, fe.position))} m away`, true);
    }
    ctx.log({ side: fe.side, domain: "land", kind: "destroyed", text: `${fe.label} (${id}) destroyed by ${why.label}`, ids: [id], at: fe.position });
    return;
  }
  ctx.land.emit(
    id,
    "underFire",
    `${why.label}${fromWhere}: shooter not located${lost ? `; ${lost} knocked out` : ""}`,
    lost > 0 || suppression >= 20,
    bearing != null ? { located: false, bearingDeg: bearing } : { located: false, bearingDeg: ((fe.facing ?? 0) + 180) % 360 },
  );
  if (lost > 0) {
    const v = ctx.land.units[id].vehicles;
    ctx.land.emit(id, "hit", `${lost} knocked out by ${why.label}: ${v.fit} of ${v.total} still fighting`, true, {
      located: false,
      ...(bearing != null ? { bearingDeg: bearing } : {}),
    });
    ctx.log({ side: fe.side, domain: "land", kind: "hit", text: `${fe.label} (${id}) lost ${lost} to ${why.label}`, ids: [id], at: fe.position });
  }
  if (lostNow > 0 || suppression >= 20) {
    ctx.land.radio({
      kind: "underFire",
      from: id,
      to: "all",
      text: `${id} under fire (${why.label})${lost ? `, ${ctx.land.units[id].vehicles.fit} of ${ctx.land.units[id].vehicles.total} left` : ""}`,
      underFire: { ...(bearing != null ? { bearingDeg: bearing } : {}), lostNow },
    });
  }
}

/** People hurt when vehicles or teams are knocked out: killed now, wounded waiting for evacuation. */
export function casualties(ctx: WorldCtx, id: string, lost: number): void {
  const comps = ctx.comps(id);
  if (!comps) return;
  let crew = 3;
  try {
    crew = recordFor(comps.archetype).body.crew;
  } catch {
    // Default crew.
  }
  const foot = ctx.fe(id)?.targetClass === "foot";
  const hurt = Math.max(1, Math.round(lost * crew * (foot ? 1 : wfig("world/landEffects.CREW_HURT_SHARE"))));
  const killed = Math.round(hurt * wfig("world/landEffects.KILLED_SHARE"));
  const wounded = hurt - killed;
  ctx.setLand(id, {
    killed: comps.killed + killed,
    wounded: [...comps.wounded, ...Array(wounded).fill(ctx.time)],
    ...(foot ? {} : { disabled: comps.disabled + Math.round(lost * 0.3) }),
  });
}
