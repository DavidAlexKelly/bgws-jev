// ── world/domains/ew.ts ──────────────────────────────────────────────────
// Electronic warfare and PNT (MULTI_DOMAIN_PLAN.md W6): the spectrum as
// terrain. Emitting is useful and dangerous; jamming is spatial and
// indiscriminate; GNSS loss degrades precision.
//
//   JAMMERS     an EW team's jammer is a footprint over its bands, on unless
//               it is silent. It touches radios (net.ts, jamming.ts), drone
//               links (drones.ts) and GNSS (pnt.ts) of BOTH sides in it.
//   ESM         listeners hear emissions in range and over the horizon: each
//               scan that hears one adds a bearing; bearings from different
//               places (or a moving listener) shrink the error ellipse to a
//               fix. A fix good enough to shoot at goes to the fires net —
//               the time from emission to strike is not set anywhere, it
//               emerges from the kill chain.
//   EXPOSURE    a unit that stays on the air is asked whether to go silent
//               or move; an EW team is asked where to jam.
//   PNT         a unit's own position error grows where GNSS is jammed, which
//               spoils its calls for fire; GNSS-guided rounds lose accuracy
//               (pnt.ts), and a gun about to fire one is asked (fires.ts).

import { hasRecord, recordFor } from "../../catalogue";
import type { Side } from "../../../lib/state";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { bearingDeg, distanceM, groundAt, horizonM, offsetBy, scatter } from "../geo";
import { guidedCepM, pntQualityAt } from "../pnt";
import { raiseQuestion } from "../questions";
import { registerSystem } from "../step";
import { requestFire } from "./fires";
import type { Emission, EmitterFix, JammerState } from "../types";

// ── Jammers ──────────────────────────────────────────────────────────────

function syncJammers(c: WorldCtx): void {
  const next: JammerState[] = [];
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!hasRecord(comps.archetype)) continue;
    const jam = recordFor(comps.archetype).emitters.find((e) => e.kind === "jammer");
    if (!jam || !c.landAlive(id)) continue;
    const fe = c.fe(id)!;
    const focus = c.world.memo[`jamOff:${id}`] != null;
    const on = comps.emcon !== "silent" && !comps.damage.comms && !focus;
    next.push({ id, side: fe.side, at: fe.position, radiusM: jam.jamRadiusM ?? 10_000, ...(jam.jamRadiiM ? { radii: { ...jam.jamRadiiM } as Record<string, number> } : {}), bands: [...(jam.jams ?? [])], on });
    if (on) c.world.emissions.push({ emitterId: id, side: fe.side, kind: "jammer", at: fe.position, time: c.time });
  }
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.status !== "active" || !hasRecord(e.archetype) || e.owner === "neutral") continue;
    const jam = recordFor(e.archetype).emitters.find((x) => x.kind === "jammer");
    if (!jam) continue;
    const on = e.emcon !== "silent" && !e.damage.comms;
    next.push({ id, side: e.owner, at: e.pos, radiusM: jam.jamRadiusM ?? 20_000, ...(jam.jamRadiiM ? { radii: { ...jam.jamRadiiM } as Record<string, number> } : {}), bands: [...(jam.jams ?? [])], on });
    if (on) c.world.emissions.push({ emitterId: id, side: e.owner, kind: "jammer", at: e.pos, time: c.time });
  }
  c.world.jammers.length = 0;
  c.world.jammers.push(...next);
}

// ── ESM ──────────────────────────────────────────────────────────────────

interface Listener {
  id: string;
  side: Side;
  at: { lat: number; lng: number };
  altM: number;
  rangeM: number;
  scanS: number;
}

function listeners(c: WorldCtx): Listener[] {
  const out: Listener[] = [];
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id) || comps.damage.sensor || !hasRecord(comps.archetype)) continue;
    const esm = recordFor(comps.archetype).sensors.find((s) => s.kind === "esm");
    if (!esm) continue;
    const fe = c.fe(id)!;
    out.push({ id, side: fe.side, at: fe.position, altM: groundAt(c.config.terrain, fe.position) + esm.heightM, rangeM: esm.rangeM, scanS: esm.scanS });
  }
  for (const id of Object.keys(c.world.entities).sort()) {
    const e = c.world.entities[id];
    if (e.status !== "active" || e.owner === "neutral" || e.damage.sensor || !hasRecord(e.archetype)) continue;
    const esm = recordFor(e.archetype).sensors.find((s) => s.kind === "esm" || s.kind === "rwr");
    if (!esm) continue;
    out.push({ id, side: e.owner, at: e.pos, altM: e.altM + esm.heightM, rangeM: esm.rangeM, scanS: esm.scanS });
  }
  return out;
}

/** The land radio net's transmissions this second are emissions too: every unit that talks can be heard. */
function landRadioEmissions(c: WorldCtx): void {
  for (const m of c.land.prev.comms?.pending ?? []) {
    if (m.dueAt == null || m.dueAt - wfig("world/domains/ew.LAND_TRANSMIT_S") > c.time || m.dueAt <= c.time) continue;
    const at = c.fe(m.sender)?.position;
    if (at) c.world.emissions.push({ emitterId: m.sender, side: m.side, kind: "radio", at, time: c.time });
  }
}

/** What was last heard of each emitter, by each side: bearings and where they were taken from. */
function esm(c: WorldCtx): void {
  if (!c.world.emissions.length) return;
  for (const l of listeners(c)) {
    const scan = Math.max(1, Math.round(l.scanS));
    if ((c.time + l.id.length) % scan !== 0) continue;
    // Everything heard since its last look, once per emitter.
    const heardNow = new Map<string, Emission>();
    for (const e of c.world.emissions) if (e.time > c.time - scan && !heardNow.has(`${e.side}:${e.emitterId}`)) heardNow.set(`${e.side}:${e.emitterId}`, e);
    for (const e of heardNow.values()) {
      if (e.side === l.side) continue;
      const range = distanceM(l.at, e.at);
      if (range > l.rangeM) continue;
      if (range > horizonM(Math.max(2, l.altM - groundAt(c.config.terrain, l.at)), 5)) continue;
      // A higher duty cycle, more time on the air: more chances to take a bearing.
      if (!c.chance(wfig("world/domains/ew.BEARING_CHANCE"))) continue;
      takeBearing(c, l, e, range);
    }
  }
}

function takeBearing(c: WorldCtx, l: Listener, e: Emission, range: number): void {
  const fixes = c.world.fixes[l.side];
  const had = fixes[e.emitterId];
  const fresh = !had || c.time - had.time > wfig("world/domains/ew.FIX_FORGET_S");
  // A bearing from somewhere new halves the error; the same place again helps a little.
  const fromKey = `bearingFrom:${l.side}:${e.emitterId}`;
  const lastFromLat = c.world.memo[`${fromKey}:lat`];
  const lastFromLng = c.world.memo[`${fromKey}:lng`];
  const newPlace = lastFromLat == null || distanceM({ lat: lastFromLat, lng: lastFromLng! }, l.at) > wfig("world/domains/ew.BASELINE_M");
  c.world.memo[`${fromKey}:lat`] = l.at.lat;
  c.world.memo[`${fromKey}:lng`] = l.at.lng;
  const first = range * wfig("world/domains/ew.FIRST_BEARING_ERROR");
  const errorM = fresh ? first : Math.max(wfig("world/domains/ew.BEST_FIX_M"), had.errorM * (newPlace ? 0.5 : 0.92));
  const bearings = fresh ? 1 : had.bearings + 1;
  const at = scatter(e.at, errorM * 0.5, (n) => c.rng.int(n));
  const fix: EmitterFix = { emitterId: e.emitterId, at, errorM, bearings, time: c.time, kind: e.kind };
  fixes[e.emitterId] = fix;
  if (fresh) c.log({ side: l.side, domain: "ew", kind: "intercept", text: `${l.id} hears a ${e.kind} ${Math.round(range / 1000)} km to the ${Math.round(bearingDeg(l.at, e.at))}°`, ids: [l.id] });
  // Good enough to shoot at: the EW team decides; with none to decide, it is reported.
  if (errorM <= wfig("world/domains/ew.FIX_TARGET_M") && c.world.memo[`fixed:${l.side}:${e.emitterId}`] == null) {
    c.mark(`fixed:${l.side}:${e.emitterId}`);
    c.log({ side: l.side, domain: "ew", kind: "fix", text: `fix on a ${e.kind} (${e.emitterId}) to ${Math.round(errorM)} m after ${bearings} bearings`, ids: [l.id, e.emitterId], at });
    const isTeam = c.comps(l.id) != null;
    const q = isTeam ? raiseQuestion(c, { agentId: l.id, side: l.side, trigger: "emitterDetected", about: { emitterId: e.emitterId, lat: at.lat, lng: at.lng, errorM: Math.round(errorM) }, quality: c.fe(l.id)?.troopQuality ?? 4 }) : null;
    if (!q) reportFix(c, l.side, l.id, fix);
  }
}

/** A fix goes to the fires net as a call for fire. */
function reportFix(c: WorldCtx, side: Side, by: string, fix: EmitterFix): void {
  if (c.mode("fires") === "off") return;
  const truth = c.landAlive(fix.emitterId) ? fix.emitterId : undefined;
  requestFire(c, { side, requestedBy: by, targetAt: fix.at, ...(truth ? { targetId: truth } : {}), priority: 3, found: fix.time, reason: `emitter fix (${fix.kind})` });
}

// ── Exposure, jamming orders, PNT ────────────────────────────────────────

function exposure(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const id of Object.keys(c.world.land).sort()) {
    const comps = c.world.land[id];
    if (!c.landAlive(id)) continue;
    const unit = c.land.units[id];
    if (unit.lastMovedAt >= c.time - 10 && comps.onAirS > 0) {
      c.setLand(id, { onAirS: 0 });
      continue;
    }
    // Command posts decide through c2's emittingTooLong, drone teams through operatorLocated.
    if (comps.archetype.startsWith("generic/cp-") || /drone|fpv|loiter/i.test(comps.archetype)) continue;
    if (comps.onAirS < wfig("world/domains/ew.EXPOSE_S") || c.time - c.last(`exposed:${id}`) < 900 || comps.emcon === "silent") continue;
    c.mark(`exposed:${id}`);
    raiseQuestion(c, { agentId: id, side: c.fe(id)!.side, trigger: "ownEmissionsExposed", about: { onAirS: Math.round(comps.onAirS) }, quality: c.fe(id)!.troopQuality });
  }
}

function jammingOrders(c: WorldCtx): void {
  for (const j of c.world.jammers) {
    if (!c.comps(j.id)) continue;
    if (c.time !== 1 && c.time - c.last(`jamOrder:${j.id}`) < wfig("world/domains/ew.JAM_REVIEW_S")) continue;
    c.mark(`jamOrder:${j.id}`);
    raiseQuestion(c, { agentId: j.id, side: j.side, trigger: "jammingOrder", quality: c.fe(j.id)?.troopQuality ?? 4 });
  }
}

function positionErrors(c: WorldCtx): void {
  if (c.time % 10 !== 0) return;
  for (const id of Object.keys(c.world.land).sort()) {
    if (!c.landAlive(id)) continue;
    const fe = c.fe(id)!;
    const q = pntQualityAt(c.world, fe.side, fe.position);
    const err = Math.round(guidedCepM(wfig("world/domains/ew.GOOD_POSITION_M"), q));
    if (err !== c.world.land[id].positionErrorM) c.setLand(id, { positionErrorM: err });
  }
}

// ── Handlers ─────────────────────────────────────────────────────────────

const fixOf = (c: WorldCtx, q: { side: Side; about: Record<string, unknown> }) => c.world.fixes[q.side][q.about.emitterId as string];
const isJammer = (c: WorldCtx, id: string) => {
  const a = c.comps(id)?.archetype;
  return a != null && hasRecord(a) && recordFor(a).emitters.some((e) => e.kind === "jammer");
};

registerHandlers({
  "ew.locate": { apply: (c, q) => void delete c.world.memo[`fixed:${q.side}:${q.about.emitterId}`] },
  "ew.jam": {
    available: (c, q) => isJammer(c, q.agentId),
    apply(c, q) {
      delete c.world.memo[`jamOff:${q.agentId}`];
      c.setLand(q.agentId, { emcon: "free" });
      const fix = fixOf(c, q);
      const fe = c.fe(q.agentId);
      const jam = fe && recordFor(c.comps(q.agentId)!.archetype).emitters.find((e) => e.kind === "jammer");
      // Out of reach: move up until the emitter is well inside the footprint.
      if (fix && fe && jam?.jamRadiusM && distanceM(fe.position, fix.at) > jam.jamRadiusM * 0.6) {
        c.land.order(q.agentId, { kind: "move", to: offsetBy(fix.at, bearingDeg(fix.at, fe.position), jam.jamRadiusM * 0.5), mode: "tactical" });
      }
    },
  },
  "ew.reportForStrike": {
    available: (c, q) => fixOf(c, q) != null && c.mode("fires") !== "off",
    apply(c, q) {
      const fix = fixOf(c, q);
      if (fix) reportFix(c, q.side, q.agentId, fix);
    },
  },
  "ew.ignore": { apply: () => {} },
  "ew.goSilent": {
    apply(c, q) {
      if (c.comps(q.agentId)) c.setLand(q.agentId, { emcon: "silent", onAirS: 0 });
      else if (c.entity(q.agentId)) c.setEntity(q.agentId, { emcon: "silent" });
      if (isJammer(c, q.agentId)) c.mark(`jamOff:${q.agentId}`);
    },
  },
  "ew.relocateAfter": {
    available: (c, q) => c.fe(q.agentId) != null,
    apply(c, q) {
      const fe = c.fe(q.agentId)!;
      const away = (bearingDeg(c.world.startLines[fe.side], fe.position) + 180 + (c.rng.int(90) - 45)) % 360;
      c.land.order(q.agentId, { kind: "move", to: offsetBy(fe.position, away, wfig("world/domains/ew.RELOCATE_M")), mode: "march" });
      c.setLand(q.agentId, { onAirS: 0 });
    },
  },
  "ew.carryOn": { apply: () => {} },
  "ew.switchBearer": {
    apply(c, q) {
      const was = c.world.structured[q.agentId];
      c.world.structured[q.agentId] = { ...(was ?? { mission: "", intent: "", phaseTriggers: [], branches: [], measures: [], weaponControl: "tight", emcon: "limited", issuedAt: c.time, by: "rules" }), pace: ["fibre", "mesh", "courier", "satcomLeo", "vhf"] };
      if (c.comps(q.agentId)) c.setLand(q.agentId, { emcon: "limited", onAirS: 0 });
    },
  },
  "ew.jamSector": {
    apply(c, q) {
      delete c.world.memo[`jamOff:${q.agentId}`];
      if (c.comps(q.agentId)) c.setLand(q.agentId, { emcon: "free" });
    },
  },
  "ew.protectAdvance": {
    available: (c, q) => c.fe(q.agentId) != null,
    apply(c, q) {
      delete c.world.memo[`jamOff:${q.agentId}`];
      const fe = c.fe(q.agentId)!;
      const objective = c.game.objectives?.[fe.side];
      // Up behind the lead: the friend nearest the objective, a few kilometres back (Kursk, R5.3).
      const lead = Object.keys(c.game.forceElements)
        .filter((id) => id !== q.agentId && c.landAlive(id) && c.fe(id)!.side === fe.side)
        .sort((a, b) => (objective ? distanceM(c.fe(a)!.position, objective) - distanceM(c.fe(b)!.position, objective) : 0))[0];
      if (!lead) return;
      const at = c.fe(lead)!.position;
      c.land.order(q.agentId, { kind: "move", to: offsetBy(at, bearingDeg(at, c.world.startLines[fe.side]), wfig("world/domains/ew.BEHIND_LEAD_M")), mode: "tactical" });
    },
  },
});

registerRules({
  emitterDetected: (c, _q, options) => (c.mode("fires") !== "off" ? prefer(options, "ew.reportForStrike", "ew.jam") : prefer(options, "ew.jam", "ew.locate")),
  ownEmissionsExposed: (_c, q, options) => ((q.about.onAirS as number) > wfig("world/domains/ew.EXPOSE_S") * 2 ? prefer(options, "ew.relocateAfter", "ew.goSilent") : prefer(options, "ew.relocateAfter", "ew.carryOn")),
  jammingOrder: (c, q, options) => {
    // With the force advancing, move up behind it; otherwise jam where it is.
    const fe = c.fe(q.agentId);
    const objective = fe && c.game.objectives?.[fe.side];
    const advancing = objective && Object.keys(c.game.forceElements).some((id) => c.fe(id)!.side === fe.side && c.land.units[id]?.order.kind === "move");
    return advancing ? prefer(options, "ew.protectAdvance", "ew.jamSector") : prefer(options, "ew.jamSector");
  },
});

registerSystem({
  name: "ew",
  order: 45,
  when: (c) => c.mode("ew") !== "off",
  run(c) {
    syncJammers(c);
    landRadioEmissions(c);
    esm(c);
    exposure(c);
    if (c.simulated("ew")) jammingOrders(c);
    positionErrors(c);
  },
});
