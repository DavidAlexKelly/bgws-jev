// ── bgws/realtime/engine/comms.ts ──────────────────────────────────────────
// How information travels between units on one side: the radio net.
//
// docs/REALTIME_COMMS.md. Game logic, every second, no model calls:
//
//   REPORTS go out automatically — contact, under fire, situation — as a
//   drill, like the react-to-contact drill. Nobody decides to send them.
//   REQUESTS ("cover me", "fire on X") and REPLIES are sent when a leader's
//   choice at a decision point carries one (decisions.ts); game logic routes
//   them.
//
// Two modes (RtConfig.comms):
//
//   perfect   every message reaches its recipients `reportDelayS` after it is
//             sent, whatever the range: the old side-wide picture.
//   radio     a unit composes its message (slower under fire; a pinned crew
//             sends only "under fire"), then waits for the net — one talker
//             at a time, most urgent first. It goes to its HQ, which passes
//             it on to everyone in its range: two hops. With no HQ in reach,
//             it goes straight to whoever is in range, a little slower.
//
// Messages are structured, not free text, so they are cheap and replayable:
// every delay is a fixed, declared figure, and nothing here rolls a die.

import { bearingDeg, distanceM } from "../../lib/board";
import type { ForceElement, GameState, Side, SightingLevel } from "../../lib/state";
import { compassWord } from "./knowledge";
import { PINNED_AT, SUPPRESSED_AT } from "./timing";
import type { CommsState, MessageKind, ReportLevel, RtConfig, RtEvent, RtMessage, RtUnit, Track } from "./types";

/** DECLARED. Seconds to compose a report, and to pass one on at an HQ. */
export const COMPOSE_S = 10;
export const RELAY_S = 5;
/** DECLARED. Seconds a transmission holds the net. */
export const TRANSMIT_S = 4;
/** DECLARED. Radio range, metres: a troop set in rolling country. */
export const RADIO_RANGE_M = 5000;
/** DECLARED. A report's position error when made (own sight), and what passing it on adds. */
export const SIGHTED_ERROR_M = 50;
/** Re-report a contact still in sight this often; not at all if a friend reported it this recently. */
export const REREPORT_S = 60;
export const ALREADY_REPORTED_S = 30;
/** At most one "under fire" report a unit this often; a situation report this often. */
export const UNDER_FIRE_REPORT_S = 20;
export const SITREP_S = 120;
/** Friends within this may be asked to help, and hear a friend's trouble as theirs to act on (D9). */
export const HELP_RADIUS_M = 1500;
const HELP_INTERVAL_S = 60;
/** The last few messages a unit remembers hearing. */
const HEARD_LENGTH = 6;

/** Most urgent first. */
const PRIORITY: Record<MessageKind, number> = { underFire: 0, request: 1, reply: 1, contact: 2, sitrep: 3 };
const RANK: Record<SightingLevel, number> = { none: 0, veryPartial: 1, partial: 2, full: 3 };

export function emptyComms(): CommsState {
  return { pending: [], nextId: 1, busyUntil: { blue: 0, red: 0 } };
}

export { trackErrorM } from "./knowledge";

/** What the net needs from the tick it runs in. */
export interface CommsContext {
  time: number;
  game: GameState;
  units: Record<string, RtUnit>;
  config: RtConfig;
  setUnit: (id: string, patch: Partial<RtUnit>) => void;
  emit: (
    unitId: string,
    kind: RtEvent["kind"],
    detail: string,
    severe?: boolean,
    extra?: Partial<Pick<RtEvent, "about" | "located" | "bearingDeg" | "info" | "requestId" | "reported">>,
  ) => void;
  known: (id: string, enemyId: string) => SightingLevel;
}

/** One message delivered: for the feed and the stream. */
export interface Delivery {
  time: number;
  message: RtMessage;
  to: string[];
}

const alive = (ctx: CommsContext, id: string) =>
  (ctx.game.forceElements[id]?.combatStrength ?? 0) > 0 && ctx.units[id] != null;

function friendsOf(ctx: CommsContext, side: Side): ForceElement[] {
  return Object.values(ctx.game.forceElements).filter((fe) => fe.side === side && alive(ctx, fe.id));
}

/** The HQ a unit reports to: the nearest steady-enough HQ in radio range, not itself. */
export function hqFor(ctx: Pick<CommsContext, "game" | "units">, id: string): string | undefined {
  const self = ctx.game.forceElements[id];
  if (!self) return undefined;
  return Object.values(ctx.game.forceElements)
    .filter((fe) => fe.side === self.side && fe.id !== id && (fe.commandRating ?? 0) > 0 && fe.combatStrength > 0)
    .filter((fe) => ctx.units[fe.id] != null && ctx.units[fe.id].cohesion !== "broken")
    .filter((fe) => distanceM(fe.position, self.position) <= RADIO_RANGE_M)
    .sort((a, b) => distanceM(a.position, self.position) - distanceM(b.position, self.position))[0]?.id;
}

/** A message before it is on the net. */
export type MessageDraft = Pick<RtMessage, "kind" | "from" | "to" | "text" | "contact" | "underFire" | "request" | "reply">;
type Draft = MessageDraft;

/**
 * Put a message on the net. A pinned crew sends only "under fire"; under
 * fire, composing takes twice as long; with no HQ to pass it on, a little
 * longer. In "perfect" comms it simply arrives `reportDelayS` later.
 */
export function queueMessage(comms: CommsState, ctx: CommsContext, draft: Draft): RtMessage | null {
  const self = ctx.game.forceElements[draft.from];
  const unit = ctx.units[draft.from];
  if (!self || !unit) return null;
  const id = comms.nextId++;
  const base = { ...draft, id, side: self.side, sender: draft.from, sentAt: ctx.time, hop: 1 as const };
  if ((ctx.config.comms ?? "perfect") === "perfect") {
    const message: RtMessage = { ...base, readyAt: ctx.time, dueAt: ctx.time + ctx.config.timing.reportDelayS };
    comms.pending.push(message);
    return message;
  }
  // A pinned crew gets out only the short calls: "under fire", and "help".
  if (unit.suppression >= PINNED_AT && draft.kind !== "underFire" && draft.kind !== "request") return null;
  const underFire = ctx.time - unit.lastIncomingAt <= 10 || unit.suppression >= SUPPRESSED_AT;
  const addressed = draft.to !== "all" ? ctx.game.forceElements[draft.to] : undefined;
  const direct = addressed != null && distanceM(addressed.position, self.position) <= RADIO_RANGE_M;
  const via = direct || (self.commandRating ?? 0) > 0 ? undefined : hqFor(ctx, draft.from);
  const compose = COMPOSE_S * (underFire ? 2 : 1) * (via || direct || (self.commandRating ?? 0) > 0 ? 1 : 1.5);
  const message: RtMessage = { ...base, via, readyAt: ctx.time + compose };
  comms.pending.push(message);
  return message;
}

/** Who hears this transmission. */
function recipients(ctx: CommsContext, message: RtMessage): string[] {
  const perfect = (ctx.config.comms ?? "perfect") === "perfect";
  const sender = ctx.game.forceElements[message.sender];
  const inRange = (id: string) =>
    perfect || (sender != null && distanceM(ctx.game.forceElements[id].position, sender.position) <= RADIO_RANGE_M);
  // First hop by way of an HQ: only the HQ hears it.
  if (!perfect && message.hop === 1 && message.via) return alive(ctx, message.via) ? [message.via] : [];
  if (message.to !== "all") return alive(ctx, message.to) && inRange(message.to) ? [message.to] : [];
  return friendsOf(ctx, message.side)
    .map((fe) => fe.id)
    .filter((id) => id !== message.from && id !== message.sender && inRange(id));
}

/**
 * One second of the net: deliver what has arrived, pass on what an HQ
 * received, and put the next message on the air on each side's net.
 */
export function stepComms(comms: CommsState, ctx: CommsContext): Delivery[] {
  const perfect = (ctx.config.comms ?? "perfect") === "perfect";
  const delivered: Delivery[] = [];
  const due = comms.pending.filter((m) => m.dueAt != null && m.dueAt <= ctx.time).sort((a, b) => a.id - b.id);
  comms.pending = comms.pending.filter((m) => !(m.dueAt != null && m.dueAt <= ctx.time));
  for (const message of due) {
    if (!alive(ctx, message.sender) && message.sender !== message.from) continue;
    const to = recipients(ctx, message);
    // An HQ passing on a message addressed to someone else only relays it.
    const relayOnly = (id: string) => message.hop === 1 && id === message.via && message.to !== "all" && message.to !== id;
    for (const id of to) if (!relayOnly(id)) receive(ctx, id, message);
    if (to.length) delivered.push({ time: ctx.time, message, to });
    // The HQ passes it on to everyone in its range: the second hop.
    if (!perfect && message.hop === 1 && message.via && to.includes(message.via)) {
      comms.pending.push({ ...message, id: comms.nextId++, sender: message.via, hop: 2, readyAt: ctx.time + RELAY_S, dueAt: undefined });
    }
  }
  if (!perfect) {
    for (const side of ["blue", "red"] as const) {
      if (comms.busyUntil[side] > ctx.time) continue;
      const next = comms.pending
        .filter((m) => m.side === side && m.dueAt == null && m.readyAt <= ctx.time)
        .filter((m) => alive(ctx, m.sender))
        .sort((a, b) => PRIORITY[a.kind] - PRIORITY[b.kind] || a.readyAt - b.readyAt || a.id - b.id)[0];
      if (!next) continue;
      next.dueAt = ctx.time + TRANSMIT_S;
      comms.busyUntil[side] = next.dueAt;
    }
    // A sender that is gone takes its unsent messages with it.
    comms.pending = comms.pending.filter((m) => m.dueAt != null || alive(ctx, m.sender));
  }
  return delivered;
}

/** What hearing a message does to the unit that hears it. */
function receive(ctx: CommsContext, id: string, message: RtMessage): void {
  const unit = ctx.units[id];
  const self = ctx.game.forceElements[id];
  if (!unit || !self) return;
  const hear = () =>
    ctx.setUnit(id, {
      heard: [...ctx.units[id].heard, { time: ctx.time, from: message.from, kind: message.kind, text: message.text }].slice(-HEARD_LENGTH),
    });
  switch (message.kind) {
    case "contact": {
      const c = message.contact!;
      if (!alive(ctx, c.enemyId)) return;
      const own = unit.ownSeen[c.enemyId];
      if (own && own.time >= c.seenAt) return;
      const had = unit.picture[c.enemyId];
      const knewBefore = ctx.known(id, c.enemyId);
      if (had && had.seenAt >= c.seenAt && RANK[had.level] >= RANK[c.level]) return;
      const level: ReportLevel = had && RANK[had.level] > RANK[c.level] ? had.level : c.level;
      const track: Track = {
        level,
        at: c.at,
        seenAt: c.seenAt,
        receivedAt: ctx.time,
        from: message.from,
        ...(message.via ? { via: message.via } : {}),
        errorM: c.errorM,
        moving: c.moving,
        ...(c.label ?? had?.label ? { label: c.label ?? had?.label } : {}),
      };
      ctx.setUnit(id, { picture: { ...ctx.units[id].picture, [c.enemyId]: track } });
      // News of an enemy it knew nothing of: a new contact (D1).
      if (knewBefore === "none") {
        ctx.emit(
          id,
          "sighted",
          `${message.from} reports ${level === "full" && c.label ? c.label : "a contact"} (${c.enemyId}) ${Math.round(
            distanceM(self.position, c.at),
          )} m ${compassWord(bearingDeg(self.position, c.at))}`,
          false,
          { about: c.enemyId, reported: true },
        );
      }
      return;
    }
    case "sitrep":
      ctx.setUnit(id, { friendStatus: { ...ctx.units[id].friendStatus, [message.from]: { time: ctx.time, text: message.text } } });
      return;
    case "underFire": {
      hear();
      // A friend close by in trouble, from an attacker this unit knows of: D9.
      const friend = ctx.game.forceElements[message.from];
      const shooter = message.underFire?.shooterId;
      const u = ctx.units[id];
      if (!friend || !shooter || u.cohesion !== "steady" || ctx.time - u.lastShotAt <= 30) return;
      if (distanceM(friend.position, self.position) > HELP_RADIUS_M) return;
      if (ctx.time - (u.helpAt[message.from] ?? -Infinity) < HELP_INTERVAL_S) return;
      if (ctx.known(id, shooter) === "none") return;
      ctx.setUnit(id, { helpAt: { ...u.helpAt, [message.from]: ctx.time } });
      ctx.emit(id, "friendNeedsHelp", message.text, (message.underFire?.lostNow ?? 0) > 0, { about: shooter });
      return;
    }
    case "request": {
      hear();
      const r = message.request!;
      ctx.setUnit(id, {
        requests: [...ctx.units[id].requests.filter((q) => ctx.time - q.time < 120), { ...r, time: ctx.time, from: message.from }],
      });
      ctx.emit(id, "request", message.text, true, { ...(r.enemyId ? { about: r.enemyId } : {}), requestId: r.id });
      return;
    }
    case "reply":
      hear();
      ctx.emit(id, "reply", message.text, false, { info: true });
      return;
  }
}
