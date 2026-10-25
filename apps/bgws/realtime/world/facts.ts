// ── realtime/world/facts.ts ──────────────────────────────────────────────
// The facts a world template's conditions test (behaviour/actions/world.ts
// WORLD_FACT_INFO), worked out for one agent — a land unit or an entity —
// at the moment it is asked. Every fact is always supplied; one that does
// not apply is null, which no ordering comparison is true of.

import { distanceM } from "../../lib/board";
import { inCover } from "../../lib/proceduralTerrain";
import type { FactTable, FactValue } from "../behaviour/types";
import type { WorldFact } from "../behaviour/actions/world";
import { recordFor, hasRecord } from "../catalogue";
import type { RtConfig, RtState } from "../engine/types";
import { isNight } from "./environment";
import { pntQualityAt } from "./pnt";
import type { WorldState } from "./types";

export interface FactCtx {
  state: Pick<RtState, "game" | "units" | "time">;
  world: WorldState;
  config: RtConfig;
  agentId: string;
  about: Record<string, string | number | boolean>;
}

function pct(have: number, of: number): number | null {
  return of > 0 ? Math.round((100 * have) / of) : null;
}

/** The fraction of a basic load still carried, over every family it started with. */
function ammoPct(have: Partial<Record<string, number>>, archetype: string): number | null {
  if (!hasRecord(archetype)) return null;
  const load = recordFor(archetype).stores.ammo;
  const families = Object.keys(load).filter((f) => (load as Record<string, number>)[f] > 0);
  if (!families.length) return null;
  const share = families.map((f) => (have[f] ?? 0) / (load as Record<string, number>)[f]);
  return Math.round(100 * Math.min(...share));
}

export const WORLD_FACT_TABLE: FactTable<FactCtx> & Record<WorldFact, (c: FactCtx) => FactValue> = {
  "agent.domain": ({ world, agentId }) => world.entities[agentId]?.domain ?? (world.land[agentId] ? "land" : null),
  "agent.archetype": ({ world, agentId }) => world.entities[agentId]?.archetype ?? world.land[agentId]?.archetype ?? null,
  "agent.strengthPct": ({ state, world, agentId }) => {
    const fe = state.game.forceElements[agentId];
    if (fe) return pct(fe.combatStrength, fe.combatStrengthStart);
    const e = world.entities[agentId];
    return e && hasRecord(e.archetype) ? pct(e.fit, recordFor(e.archetype).body.count) : null;
  },
  "agent.ammoPct": ({ world, agentId }) => {
    const land = world.land[agentId];
    if (land) return ammoPct(land.stores.ammo, land.archetype);
    const e = world.entities[agentId];
    return e ? ammoPct(e.stores.ammo, e.archetype) : null;
  },
  "agent.fuelPct": ({ world, agentId }) => {
    const owner = world.land[agentId] ?? world.entities[agentId];
    if (!owner || !hasRecord(owner.archetype)) return null;
    return pct(owner.stores.fuelL, recordFor(owner.archetype).stores.fuelL);
  },
  "agent.batteryPct": ({ world, agentId }) => {
    const e = world.entities[agentId];
    if (e?.enduranceS != null && hasRecord(e.archetype)) {
      const full = (recordFor(e.archetype).mover.enduranceH ?? 0) * 3600;
      return pct(e.enduranceS, full);
    }
    const land = world.land[agentId];
    return land && hasRecord(land.archetype) ? pct(land.stores.batteryWh, recordFor(land.archetype).stores.batteryWh) : null;
  },
  "agent.magazinePct": ({ world, agentId }) => {
    const mounts = world.entities[agentId]?.mounts ?? world.land[agentId]?.mounts ?? [];
    const archetype = world.entities[agentId]?.archetype ?? world.land[agentId]?.archetype;
    if (!mounts.length || !archetype || !hasRecord(archetype)) return null;
    const rec = recordFor(archetype);
    const full = mounts.reduce((sum, m) => sum + (rec.weapons.find((w) => w.id === m.weaponId)?.magazine ?? 0), 0);
    return pct(mounts.reduce((sum, m) => sum + m.rounds, 0), full);
  },
  "agent.onAirS": ({ world, agentId }) => world.land[agentId]?.onAirS ?? null,
  "agent.inCover": ({ state, config, agentId }) => {
    const fe = state.game.forceElements[agentId];
    return fe ? inCover(config.terrain, fe.position) : null;
  },
  "agent.carriesDismounts": ({ world, agentId }) => (world.land[agentId]?.dismounts.carried ?? 0) > 0,
  "agent.hasFibreDrones": ({ world, agentId }) => world.land[agentId]?.archetype === "generic/fpvTeamFibre",
  "agent.armed": ({ world, agentId }) => {
    const owner = world.entities[agentId] ?? world.land[agentId];
    return owner ? owner.mounts.some((m) => m.rounds > 0) || (hasRecord(owner.archetype) && recordFor(owner.archetype).weapons.length > 0) : null;
  },
  "agent.canAutonomous": ({ world, agentId }) => {
    const e = world.entities[agentId];
    return e ? e.archetype === "generic/loiter" || e.archetype === "generic/owaDrone" : null;
  },
  "link.quality": ({ world, agentId }) => world.entities[agentId]?.link?.quality ?? null,
  "pnt.quality": ({ state, world, agentId }) => {
    const at = state.game.forceElements[agentId]?.position ?? world.entities[agentId]?.pos;
    const side = state.game.forceElements[agentId]?.side ?? world.entities[agentId]?.owner;
    return at && (side === "blue" || side === "red") ? Math.round(100 * pntQualityAt(world, side, at)) / 100 : null;
  },
  "target.rangeKm": ({ state, world, agentId, about }) => {
    const from = state.game.forceElements[agentId]?.position ?? world.entities[agentId]?.pos;
    const to = targetAt(state, world, about);
    return from && to ? Math.round(distanceM(from, to) / 100) / 10 : null;
  },
  "target.inRange": ({ about }) => (typeof about.inRange === "boolean" ? about.inRange : null),
  "target.identified": ({ about }) => (typeof about.identified === "boolean" ? about.identified : null),
  "target.costUsd": ({ about }) => (typeof about.targetCostUsd === "number" ? about.targetCostUsd : null),
  "threat.adWithinKm": ({ about }) => (typeof about.adWithinKm === "number" ? about.adWithinKm : null),
  "threat.located": ({ about }) => (typeof about.located === "boolean" ? about.located : null),
  "env.night": ({ world, state }) => isNight(world.env, state.time),
  "env.badWeather": ({ world }) => world.env.visibilityKm < 3 || world.env.precipitation > 0.5 || world.env.windMs > 12,
  "roe.weaponsFree": ({ world, state, agentId }) => {
    const side = state.game.forceElements[agentId]?.side ?? world.entities[agentId]?.owner;
    const hq = Object.values(world.hqs).find((h) => h.side === side && h.guidance);
    return hq?.guidance?.roe.weaponsFree ?? true;
  },
};

function targetAt(state: Pick<RtState, "game">, world: WorldState, about: FactCtx["about"]) {
  if (typeof about.lat === "number" && typeof about.lng === "number") return { lat: about.lat, lng: about.lng };
  const id = typeof about.targetId === "string" ? about.targetId : undefined;
  if (!id) return undefined;
  return state.game.forceElements[id]?.position ?? world.entities[id]?.pos;
}
