// ── realtime/world/net.ts ────────────────────────────────────────────────
// Networks for command and control (MULTI_DOMAIN_PLAN.md §4.5, W4): messages
// over BEARERS, each with a range model, a delay, a loss model under
// jamming, and an RF signature (every transmission is an emission ESM can
// hear, W6).
//
//   vhf        combat net radio: 5–10 km on high power, more with a mast;
//              terrain cuts it [ref R2, P4]; frequency hopping resists jamming
//   hf         NVIS, 35–450 km, slow [ref R2, P5]
//   uhf        air–ground, line of sight to the horizon
//   datalink   Link-16-like: fast and precise, line of sight or relayed by an
//              aircraft; 31.6–115.2 kbps [ref R2, P6]
//   satcomMil  GEO: anywhere in the footprint, while the satellite and the
//              ground segment work
//   satcomLeo  commercial LEO: 25–100 ms [ref R2, P7]; a provider can cut it
//   mesh       hop by hop through friends close by
//   fibre      between fixed posts
//   courier    a vehicle: slow, sure, never jammed
//
// PACE: a sender tries its bearers in order (primary, alternate,
// contingency, emergency) and falls back as a rule, not a decision, when one
// cannot reach. The land engine's own radio net (comms.ts) still carries the
// platoon-level chatter; this carries what HQs and staff cells send.

import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import { hasRecord, recordFor } from "../catalogue";
import type { BearerId } from "../catalogue/schema";
import type { WorldCtx } from "./context";
import { wfig, type WorldFigureKey } from "./figures";
import { distanceM, groundAt, horizonM, seesBetween } from "./geo";
import type { NetMessage } from "./types";

export const DEFAULT_PACE: readonly BearerId[] = ["vhf", "datalink", "hf", "satcomLeo", "satcomMil", "mesh", "courier"];

interface BearerModel {
  /** Can a message go from `a` to `b` on this bearer directly? */
  reach: (c: WorldCtx, a: Node, b: Node) => boolean;
  /** Compose + transmit + latency, s. */
  delayS: (c: WorldCtx, a: Node, b: Node) => number;
  /** The band jammers must cover to touch it, and how well it resists (0 = not at all, 1 = immune). */
  band: string | null;
  resistance: () => number;
  /** Is it an emission (radio), and of what kind. */
  emits: boolean;
}

/** One end of a link: a land unit or an entity. */
interface Node {
  id: string;
  at: LatLng;
  altM: number;
  bearers: readonly BearerId[];
  /** A command post with a mast and an amplifier. */
  post: boolean;
  /** Has not moved for a while: fibre can be laid to it. */
  fixed: boolean;
}

const f = (k: WorldFigureKey) => wfig(k);

function los(c: WorldCtx, a: Node, b: Node, mastM: number): boolean {
  return seesBetween(c.config.terrain, a.at, a.altM + mastM, b.at, b.altM + mastM, 16);
}

export const BEARERS: Record<BearerId, BearerModel> = {
  vhf: {
    reach: (c, a, b) => {
      const range = (a.post || b.post ? f("world/net.VHF_POST_RANGE_M") : f("world/net.VHF_RANGE_M")) * (los(c, a, b, 3) ? 1 : f("world/net.VHF_NO_LOS"));
      return distanceM(a.at, b.at) <= range;
    },
    delayS: () => f("world/net.VOICE_COMPOSE_S") + f("world/net.VOICE_TRANSMIT_S"),
    band: "vhf",
    resistance: () => f("world/net.VHF_HOP_RESIST"),
    emits: true,
  },
  hf: {
    reach: (_c, a, b) => distanceM(a.at, b.at) <= f("world/net.HF_RANGE_M"),
    delayS: () => f("world/net.VOICE_COMPOSE_S") + f("world/net.HF_TRANSMIT_S"),
    band: "hf",
    resistance: () => 0.3,
    emits: true,
  },
  uhf: {
    reach: (c, a, b) => distanceM(a.at, b.at) <= Math.min(horizonM(Math.max(2, a.altM), Math.max(2, b.altM)), 400_000) && los(c, a, b, 2),
    delayS: () => f("world/net.VOICE_COMPOSE_S") + 5,
    band: "uhf",
    resistance: () => 0.4,
    emits: true,
  },
  datalink: {
    reach: (c, a, b) => distanceM(a.at, b.at) <= Math.min(horizonM(Math.max(5, a.altM + 5), Math.max(5, b.altM + 5)), f("world/net.DATALINK_RANGE_M")) || relayedByAir(c, a, b),
    delayS: () => f("world/net.DATALINK_S"),
    band: "l",
    resistance: () => f("world/net.DATALINK_RESIST"),
    emits: true,
  },
  satcomMil: {
    reach: (c, a) => satelliteUp(c, "generic/satcomGeo") && !outaged(c, "satcomMil", a.id),
    delayS: () => f("world/net.SATCOM_MIL_S"),
    band: "ku",
    resistance: () => 0.7,
    emits: true,
  },
  satcomLeo: {
    reach: (c, a) => !outaged(c, "satcomLeo", a.id),
    delayS: () => f("world/net.SATCOM_LEO_S"),
    band: "ku",
    resistance: () => 0.5,
    emits: true,
  },
  mesh: {
    reach: (c, a, b) => meshPath(c, a, b) != null,
    delayS: (c, a, b) => (meshPath(c, a, b)?.length ?? 1) * f("world/net.MESH_HOP_S"),
    band: "uhf",
    resistance: () => 0.5,
    emits: true,
  },
  fibre: {
    reach: (_c, a, b) => a.fixed && b.fixed && a.post && b.post,
    delayS: () => 1,
    band: null,
    resistance: () => 1,
    emits: false,
  },
  courier: {
    reach: () => true,
    delayS: (_c, a, b) => f("world/net.COURIER_SETUP_S") + distanceM(a.at, b.at) / f("world/net.COURIER_MS"),
    band: null,
    resistance: () => 1,
    emits: false,
  },
};

/** In a space-simulated game a SATCOM bearer needs its satellite; otherwise it is assumed. */
/** A bearer or system cut, for everyone or for this node's side (cyber, counterspace, a provider): until when it is out. */
export function outaged(c: WorldCtx, key: string, nodeId?: string): boolean {
  const side = nodeId ? c.sideOf(nodeId) : undefined;
  const until = Math.max(c.world.outages[key] ?? -Infinity, side ? (c.world.outages[`${key}:${side}`] ?? -Infinity) : -Infinity);
  return until > c.time;
}

function satelliteUp(c: WorldCtx, archetype: string): boolean {
  if (!c.simulated("space")) return true;
  return Object.values(c.world.entities).some((e) => e.archetype === archetype && e.status === "active" && !e.damage.comms);
}

/** An airborne friend (AEW, any aircraft with a data link) that sees both ends. */
function relayedByAir(c: WorldCtx, a: Node, b: Node): boolean {
  const side = c.sideOf(a.id);
  return Object.values(c.world.entities).some((e) => {
    if (e.owner !== side || e.domain !== "air" || e.status !== "active" || !hasRecord(e.archetype)) return false;
    if (!recordFor(e.archetype).bearers.includes("datalink")) return false;
    const hop = horizonM(e.altM, 5);
    return distanceM(e.pos, a.at) <= hop && distanceM(e.pos, b.at) <= hop;
  });
}

/** Hop by hop through friends within mesh range: the shortest path, or null. */
function meshPath(c: WorldCtx, a: Node, b: Node): string[] | null {
  const side = c.sideOf(a.id);
  const hop = f("world/net.MESH_HOP_M");
  const friends = Object.keys(c.game.forceElements).filter((id) => c.landAlive(id) && c.fe(id)!.side === side).map((id) => ({ id, at: c.fe(id)!.position }));
  const seen = new Set([a.id]);
  let frontier = [{ id: a.id, at: a.at, path: [] as string[] }];
  for (let depth = 0; depth < 6 && frontier.length; depth++) {
    const next: typeof frontier = [];
    for (const n of frontier) {
      if (distanceM(n.at, b.at) <= hop) return [...n.path, b.id];
      for (const fr of friends) {
        if (seen.has(fr.id) || distanceM(n.at, fr.at) > hop) continue;
        seen.add(fr.id);
        next.push({ id: fr.id, at: fr.at, path: [...n.path, fr.id] });
      }
    }
    frontier = next;
  }
  return null;
}

/** The network's view of a unit or entity. */
export function nodeOf(c: WorldCtx, id: string): Node | undefined {
  const fe = c.fe(id);
  if (fe) {
    if (!c.landAlive(id)) return undefined;
    const comps = c.comps(id);
    const rec = comps && hasRecord(comps.archetype) ? recordFor(comps.archetype) : undefined;
    const unit = c.land.units[id];
    return {
      id,
      at: fe.position,
      altM: groundAt(c.config.terrain, fe.position),
      bearers: comps?.damage.comms ? ["courier"] : [...(rec?.bearers ?? ["vhf"]), "mesh", "courier"],
      post: (rec?.id.startsWith("generic/cp-") ?? false) || (fe.commandRating ?? 0) > 0,
      fixed: c.time - (unit?.lastMovedAt ?? -Infinity) > 600,
    };
  }
  const e = c.entity(id);
  if (!e || e.status === "destroyed" || e.status === "expended") return undefined;
  const rec = hasRecord(e.archetype) ? recordFor(e.archetype) : undefined;
  return { id, at: e.pos, altM: e.altM, bearers: e.damage.comms ? [] : [...(rec?.bearers ?? []), ...(e.domain === "node" ? (["courier"] as BearerId[]) : [])], post: e.domain === "node", fixed: e.domain === "node" };
}

/** How badly jamming touches a transmission on this bearer between these two points: 0 (not) to 1 (lost). */
export function jamLoss(c: WorldCtx, bearer: BearerId, a: LatLng, b: LatLng, override?: { band: string; resistance: number; side?: string; ownFactor?: number }): number {
  const model = override ? { band: override.band, resistance: () => override.resistance } : BEARERS[bearer];
  if (!model.band) return 0;
  let worst = 0;
  for (const j of c.world.jammers) {
    if (!j.on || !j.bands.includes(model.band)) continue;
    // The receiving end is what jamming denies; a sender in the footprint suffers too.
    const radius = j.radii?.[model.band] ?? j.radiusM;
    const d = Math.min(distanceM(j.at, b), distanceM(j.at, a) * 1.5);
    if (d >= radius) continue;
    // A side's own jammer can be deconflicted from some of its own links (drone frequency plans): weaker on them.
    const own = override?.side != null && j.side === override.side ? (override.ownFactor ?? 1) : 1;
    worst = Math.max(worst, ((1 - d / radius) * (1 - model.resistance()) + 0.15) * own);
  }
  // A cyber effect on the bearer degrades it everywhere.
  if ((c.world.outages[`degrade:${bearer}`] ?? 0) > c.time) worst = Math.max(worst, 0.6);
  return Math.min(0.98, worst);
}

export interface SendSpec {
  side: Side;
  kind: NetMessage["kind"];
  from: string;
  to: string;
  text: string;
  body?: Record<string, unknown>;
  /** Bearers in PACE order; default: the sender's, in the standard order. */
  pace?: readonly BearerId[];
}

/**
 * Send a message: the first bearer in the PACE plan both ends have that can
 * reach goes; if jamming loses it, the next is tried, as a rule. Returns the
 * message (marked lost if nothing could carry it).
 */
export function sendNet(c: WorldCtx, spec: SendSpec): NetMessage {
  const a = nodeOf(c, spec.from);
  const b = nodeOf(c, spec.to);
  const id = c.newNumber();
  const base: NetMessage = { id, side: spec.side, kind: spec.kind, from: spec.from, to: spec.to, bearer: "courier", sentAt: c.time, dueAt: Infinity, text: spec.text, body: spec.body ?? {}, path: [spec.from] };
  if (!a || !b) {
    const lost = { ...base, lost: true };
    c.world.messages.push(lost);
    return lost;
  }
  const pace = (spec.pace ?? DEFAULT_PACE).filter((bearer) => a.bearers.includes(bearer) && b.bearers.includes(bearer));
  let delay = 0;
  for (const bearer of pace) {
    const model = BEARERS[bearer];
    if (outaged(c, bearer, a.id) || !model.reach(c, a, b)) continue;
    if (model.emits && (c.comps(a.id)?.emcon === "silent" || c.entity(a.id)?.emcon === "silent") && bearer !== "fibre") continue;
    delay += model.delayS(c, a, b);
    if (model.emits) {
      c.world.emissions.push({ emitterId: a.id, side: spec.side, kind: bearer === "datalink" ? "datalink" : bearer.startsWith("satcom") ? "satcom" : "radio", at: a.at, time: c.time });
      const comps = c.comps(a.id);
      if (comps) c.setLand(a.id, { onAirS: comps.onAirS + BEARERS[bearer].delayS(c, a, b) });
    }
    if (c.chance(jamLoss(c, bearer, a.at, b.at))) {
      // Lost to jamming: the operator tries the next bearer after a while.
      delay += f("world/net.RETRY_S");
      continue;
    }
    const msg: NetMessage = { ...base, bearer, dueAt: c.time + delay, path: [spec.from, spec.to] };
    c.world.messages.push(msg);
    return msg;
  }
  const lost = { ...base, lost: true };
  c.world.messages.push(lost);
  return lost;
}

/** How long a message from `from` to `to` would take now, if it can go at all (for routing estimates). */
export function deliveryDelayS(c: WorldCtx, from: string, to: string): number | null {
  if (from === to) return 0;
  const a = nodeOf(c, from);
  const b = nodeOf(c, to);
  if (!a || !b) return null;
  for (const bearer of DEFAULT_PACE) {
    if (!a.bearers.includes(bearer) || !b.bearers.includes(bearer)) continue;
    const model: BearerModel = BEARERS[bearer];
    if (!outaged(c, bearer, a.id) && model.reach(c, a, b)) return model.delayS(c, a, b);
  }
  return null;
}

type DeliveryHandler = (c: WorldCtx, m: NetMessage) => void;
const ON_DELIVER = new Map<NetMessage["kind"], DeliveryHandler[]>();
/** What happens when a message of this kind arrives. */
export function onDelivered(kind: NetMessage["kind"], handler: DeliveryHandler): void {
  ON_DELIVER.set(kind, [...(ON_DELIVER.get(kind) ?? []), handler]);
}

/** Deliver every message that is due, in order; drop old delivered ones. */
export function stepNet(c: WorldCtx): void {
  const due = c.world.messages.filter((m) => !m.lost && !m.delivered && m.dueAt <= c.time).sort((x, y) => x.dueAt - y.dueAt || x.id - y.id);
  for (const m of due) {
    const i = c.world.messages.findIndex((x) => x.id === m.id);
    c.world.messages[i] = { ...m, delivered: true };
    // A receiver that is gone receives nothing.
    if (!nodeOf(c, m.to)) continue;
    for (const handler of ON_DELIVER.get(m.kind) ?? []) handler(c, c.world.messages[i]);
  }
  const keep = f("world/net.KEEP_S");
  if (c.world.messages.length > 200) {
    const kept = c.world.messages.filter((m) => !(m.delivered || m.lost) || c.time - m.sentAt < keep);
    c.world.messages.length = 0;
    c.world.messages.push(...kept);
  }
}
