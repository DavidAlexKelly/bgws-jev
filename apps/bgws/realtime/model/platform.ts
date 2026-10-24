// ── realtime/model/platform.ts ───────────────────────────────────────────
// The platform made of components (MULTI_DOMAIN_PLAN.md §4.1, W2): one shape
// for every domain — a tank troop, a drone, a destroyer, a satellite — read
// from the land engine's ForceElement and RtUnit, the world's components and
// the catalogue record behind it.
//
// THE ADAPTER COMES FIRST. The land engine keeps fighting ForceElements; this
// is a read-only view over them, so the golden tests see nothing change. The
// inspector, the HQ console and the world systems read platforms through it,
// so a land unit and an aircraft are described the same way.
//
//   Today's ForceElement / RtUnit   Component
//   vehicles {total, fit}           body
//   armour                          body vulnerability
//   capabilities                    weapons
//   speedKmh, moveType              mover
//   detection multipliers           signatures

import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import { hasRecord, recordFor } from "../catalogue";
import type { BearerId, BodySpec, EmitterSpec, MoverSpec, PlatformDomain, SensorSpec, SignatureSpec, WeaponSpec } from "../catalogue/schema";
import type { RtState } from "../engine/types";
import type { DamageState, Emcon, MountState, Owner, Stores } from "../world/types";
import { archetypeOfSnapshot } from "./archetypes";

export interface Platform {
  id: string;
  side: Owner;
  label: string;
  sidc: string;
  archetype: string;
  domain: PlatformDomain;
  /** Its HQ in the command tree, if any. */
  parent?: string;
  kin: { at: LatLng; altM: number; headingDeg: number; speedMs: number };
  mover: MoverSpec;
  body: BodySpec & { fit: number; damage: DamageState };
  signatures: SignatureSpec;
  sensors: SensorSpec[];
  weapons: (WeaponSpec & { rounds: number | null })[];
  bearers: BearerId[];
  emitters: (EmitterSpec & { on: boolean })[];
  stores: Stores;
  carried: { dismounts: number; drones: number; ids: string[] };
  crew: { quality: number; suppression: number; cohesion: string; wounded: number; killed: number };
  emcon: Emcon;
  activity: string;
  template: string;
}

const NO_DAMAGE: DamageState = { mobility: false, firepower: false, sensor: false, comms: false };
const EMPTY_STORES: Stores = { fuelL: 0, ammo: {}, batteryWh: 0, medical: 0, spares: 0 };

function weaponsWithRounds(weapons: readonly WeaponSpec[], mounts: readonly MountState[]): Platform["weapons"] {
  return weapons.map((w) => ({ ...w, rounds: mounts.find((m) => m.weaponId === w.id)?.rounds ?? null }));
}

/** The component view of a land unit or a world entity, or undefined if there is none by that id. */
export function toPlatform(state: RtState, id: string): Platform | undefined {
  const fe = state.game.forceElements[id];
  const world = state.world;
  if (fe) {
    const unit = state.units[id];
    const comps = world?.land[id];
    const archetype = comps?.archetype ?? archetypeOfSnapshot(id, { targetClass: fe.targetClass, moveType: fe.moveType, capabilities: fe.capabilities, armourMm: fe.armourMm, displayName: fe.label });
    const rec = hasRecord(archetype) ? recordFor(archetype) : undefined;
    const vulnerability = fe.targetClass === "foot" ? "personnel" : fe.targetClass === "armoured_vehicle" ? "armoured" : "soft";
    return {
      id,
      side: fe.side,
      label: fe.label,
      sidc: fe.sidc,
      archetype,
      domain: "land",
      ...(comps?.hq ? { parent: comps.hq } : {}),
      kin: { at: fe.position, altM: 0, headingDeg: fe.facing ?? 0, speedMs: unit?.lastMovedAt === state.time ? ((fe.speedKmh ?? 30) / 3.6) * 0.6 : 0 },
      mover: rec?.mover ?? { kind: fe.moveType === "F" ? "foot" : fe.moveType === "W" ? "wheeled" : "tracked", speedKmh: fe.speedKmh ?? 40, cruiseKmh: (fe.speedKmh ?? 40) / 2 },
      body: {
        count: unit?.vehicles.total ?? fe.platformCount ?? 1,
        fit: unit?.vehicles.fit ?? 1,
        vulnerability,
        armourMm: fe.armourMm ?? 0,
        stayingPower: 1,
        crew: rec?.body.crew ?? 3,
        damage: comps?.damage ?? NO_DAMAGE,
      },
      signatures: rec?.signatures ?? { visual: fe.targetClass === "foot" ? 0.4 : 1, ir: 1, rcsM2: 10, acousticDb: 0 },
      sensors: rec?.sensors ?? [],
      // The land engine's own direct-fire capabilities, as weapons, then the world's mounts.
      weapons: [
        ...fe.capabilities.map(
          (cap): Platform["weapons"][number] => ({
            id: `land.${cap.kind}`,
            kind: cap.kind === "atm" ? "atgm" : "directFire",
            minRangeM: 0,
            maxRangeM: cap.maxRangeM,
            salvo: 1,
            magazine: 0,
            reloadS: 0,
            flight: "none",
            speedMs: 0,
            guidance: "none",
            pk: 0,
            cepM: 0,
            lethalRadiusM: 0,
            penetrationMm: cap.penetrationMm ?? 0,
            topAttack: cap.topAttack ?? false,
            targets: ["land"],
            family: cap.kind === "atm" ? "atgm" : "smallArms",
            costUsd: 0,
            rounds: null,
          }),
        ),
        ...(rec ? weaponsWithRounds(rec.weapons.filter((w) => comps?.mounts.some((m) => m.weaponId === w.id)), comps?.mounts ?? []) : []),
      ],
      bearers: rec?.bearers ?? ["vhf"],
      emitters: (rec?.emitters ?? []).map((e) => ({ ...e, on: (comps?.emcon ?? "free") !== "silent" })),
      stores: comps?.stores ?? EMPTY_STORES,
      carried: { dismounts: comps?.dismounts.carried ?? 0, drones: comps?.drones ?? 0, ids: comps?.dismounts.out ?? [] },
      crew: {
        quality: fe.troopQuality,
        suppression: unit?.suppression ?? 0,
        cohesion: unit?.cohesion ?? "steady",
        wounded: comps?.wounded.length ?? 0,
        killed: comps?.killed ?? 0,
      },
      emcon: comps?.emcon ?? "free",
      activity: unit?.order.kind ?? "hold",
      template: rec?.template ?? "core/default",
    };
  }
  const e = world?.entities[id];
  if (!e) return undefined;
  const rec = hasRecord(e.archetype) ? recordFor(e.archetype) : undefined;
  const side: Side | undefined = e.owner === "neutral" ? undefined : e.owner;
  return {
    id,
    side: e.owner,
    label: e.label,
    sidc: rec ? (side === "red" ? rec.sidc.red : rec.sidc.blue) : "SUAPWM---------",
    archetype: e.archetype,
    domain: e.domain,
    ...(e.parent ? { parent: e.parent } : {}),
    kin: { at: e.pos, altM: e.altM, headingDeg: e.headingDeg, speedMs: e.speedMs },
    mover: rec?.mover ?? { kind: "pursuit", speedKmh: e.speedMs * 3.6, cruiseKmh: e.speedMs * 3.6 },
    body: { ...(rec?.body ?? { count: 1, vulnerability: "missile", armourMm: 0, stayingPower: 1, crew: 0 }), fit: e.fit, damage: e.damage },
    signatures: rec?.signatures ?? { visual: 0.3, ir: 1.5, rcsM2: 0.1, acousticDb: 0 },
    sensors: rec?.sensors ?? [],
    weapons: rec ? weaponsWithRounds(rec.weapons, e.mounts) : [],
    bearers: rec?.bearers ?? [],
    emitters: (rec?.emitters ?? []).map((em) => ({ ...em, on: e.emcon !== "silent" })),
    stores: e.stores,
    carried: { dismounts: 0, drones: 0, ids: [] },
    crew: { quality: rec?.crewQuality ?? 4, suppression: 0, cohesion: "steady", wounded: 0, killed: 0 },
    emcon: e.emcon,
    activity: e.task.kind,
    template: e.template,
  };
}

/** Every platform in the game: land units, then entities, in id order. */
export function allPlatforms(state: RtState): Platform[] {
  const ids = [...Object.keys(state.game.forceElements).sort(), ...Object.keys(state.world?.entities ?? {}).sort()];
  return ids.map((id) => toPlatform(state, id)).filter((p): p is Platform => p != null);
}
