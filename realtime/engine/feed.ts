// ── bgws/realtime/engine/feed.ts ───────────────────────────────────────────
// The feed, in words and as rows.
//
// `describeEntry` is the line the screen shows. `streamRow` is the same
// entry as one row of the Foundry event stream (data/eventStream.ts): the
// shared columns, and a struct for whichever kind of entry it is — event,
// shot, decision or flag — with the others null. The row's `text` is the
// screen's line, so the stream reads exactly like the feed.

import type { RtLogEntry } from "./runner";
import { clock } from "./timing";
import type { RtState } from "./types";

/** Why a Jev call failed, from the trace: the error it reported, or that it did not answer. */
export function jevFailure(rationale: string | undefined): string {
  const text = rationale ?? "";
  const reported = /Jev unavailable \((.*)\) — the rules decided/.exec(text)?.[1];
  if (reported) return reported;
  if (/gave no answer/.test(text)) return "no answer for this unit in the reply";
  return "unknown";
}

/** One line of the feed, in words. */
export function describeEntry(entry: RtLogEntry): string {
  const at = clock(entry.time);
  if (entry.type === "shot") return `${at}  ${entry.shot.firerId} fires on ${entry.shot.targetId}: ${entry.shot.result}`;
  if (entry.type === "event") return `${at}  ${entry.event.unitId} ${entry.event.kind}: ${entry.event.detail}`;
  if (entry.type === "flag") return `${at}  ⚑ ${entry.flag.text}`;
  if (entry.type === "message") {
    const m = entry.delivery.message;
    const to = entry.delivery.to.length === 1 ? entry.delivery.to[0] : `${entry.delivery.to.length} units`;
    const relayed = m.sender !== m.from ? ` (passed on by ${m.sender})` : "";
    return `${at}  📻 ${m.from} → ${to}${relayed}: ${m.text}`;
  }
  const d = entry.decision;
  const p = d.trace.probabilities?.[d.optionId];
  const who =
    d.trace.chosenBy === "jev"
      ? `Jev${p != null ? ` ${Math.round(p * 100)}%` : ""}`
      : `rules${d.trace.fallback ? `, Jev ${d.trace.fallback === "error" ? `failed: ${jevFailure(d.trace.rationale)}` : d.trace.fallback}` : ""}`;
  const point = d.trace.question?.split(" ")[0] ?? "";
  return `${at}  ${d.unitId} ${point} → ${entry.summary} (${who}; asked ${clock(entry.askedAt)})`;
}

/** Who a stream belongs to: one run of one scenario. */
export interface StreamRun {
  /** New for every run, "Run scenario again" included. */
  runId: string;
  gameSeed: string;
}

/** One row of the event stream. Field names and types match the stream's schema. */
export interface StreamRow {
  runId: string;
  sequence: number;
  /** Wall-clock time, epoch milliseconds. */
  emittedAt: number;
  simTimeS: number;
  gameSeed: string;
  entryType: RtLogEntry["type"];
  side: string;
  unitId: string | null;
  unitLat: number | null;
  unitLng: number | null;
  text: string;
  event: {
    kind: string;
    detail: string;
    severe: boolean;
    info: boolean;
    aboutId: string | null;
    located: boolean | null;
    bearingDeg: number | null;
  } | null;
  shot: {
    firerId: string;
    targetId: string;
    targetSide: string;
    result: string;
    narrative: string | null;
    rangeM: number | null;
    rounds: number | null;
    hits: number | null;
    knockedOut: number | null;
    pHit: number | null;
  } | null;
  decision: {
    decisionPoint: string;
    question: string;
    optionId: string;
    summary: string;
    chosenBy: string;
    fallback: string | null;
    confidence: number | null;
    latencyMs: number | null;
    askedAtS: number;
    rationale: string | null;
    options: { id: string; summary: string; probability: number | null }[];
  } | null;
  flag: { kind: string; text: string } | null;
  message: {
    kind: string;
    fromId: string;
    senderId: string;
    toIds: string[];
    viaId: string | null;
    hop: number;
    sentAtS: number;
    enemyId: string | null;
    text: string;
  } | null;
}

/** A feed entry as a stream row. `state` is the game as it stands when the entry is made. */
export function streamRow(entry: RtLogEntry, state: RtState, run: StreamRun, sequence: number, now = Date.now()): StreamRow {
  const unitId =
    entry.type === "event"
      ? entry.event.unitId
      : entry.type === "shot"
        ? entry.shot.firerId
        : entry.type === "decision"
          ? entry.decision.unitId
          : entry.type === "message"
            ? entry.delivery.message.from
            : (entry.flag.unitId ?? null);
  const at = unitId ? state.game.forceElements[unitId]?.position : undefined;
  const base = {
    runId: run.runId,
    sequence,
    emittedAt: now,
    simTimeS: Math.round(entry.time),
    gameSeed: run.gameSeed,
    entryType: entry.type,
    side: entry.side,
    unitId,
    unitLat: at?.lat ?? null,
    unitLng: at?.lng ?? null,
    text: describeEntry(entry),
    event: null,
    shot: null,
    decision: null,
    flag: null,
    message: null,
  };
  switch (entry.type) {
    case "event": {
      const e = entry.event;
      return {
        ...base,
        event: {
          kind: e.kind,
          detail: e.detail,
          severe: e.severe,
          info: e.info ?? false,
          aboutId: e.about ?? null,
          located: e.located ?? null,
          bearingDeg: e.bearingDeg ?? null,
        },
      };
    }
    case "shot": {
      const s = entry.shot;
      return {
        ...base,
        shot: {
          firerId: s.firerId,
          targetId: s.targetId,
          targetSide: state.game.forceElements[s.targetId]?.side ?? "",
          result: s.result,
          narrative: s.narrative ?? null,
          rangeM: s.rangeM ?? null,
          rounds: s.rounds ?? null,
          hits: s.hits ?? null,
          knockedOut: s.knockedOut ?? null,
          pHit: s.pHit ?? null,
        },
      };
    }
    case "decision": {
      const d = entry.decision;
      const t = d.trace;
      return {
        ...base,
        decision: {
          decisionPoint: t.question?.split(" ")[0]?.replace(/:$/, "") ?? "",
          question: t.question ?? "",
          optionId: d.optionId,
          summary: entry.summary,
          chosenBy: t.chosenBy === "jev" ? "jev" : "rules",
          fallback: t.fallback ?? null,
          confidence: t.confidence ?? null,
          latencyMs: t.latencyMs ?? null,
          askedAtS: Math.round(entry.askedAt),
          rationale: t.rationale ?? null,
          options: t.options.map((option) => ({
            id: option.id,
            summary: option.summary,
            probability: t.probabilities?.[option.id] ?? null,
          })),
        },
      };
    }
    case "message": {
      const m = entry.delivery.message;
      return {
        ...base,
        message: {
          kind: m.kind,
          fromId: m.from,
          senderId: m.sender,
          toIds: entry.delivery.to,
          viaId: m.via ?? null,
          hop: m.hop,
          sentAtS: Math.round(m.sentAt),
          enemyId: m.contact?.enemyId ?? m.underFire?.shooterId ?? m.request?.enemyId ?? null,
          text: m.text,
        },
      };
    }
    case "flag":
      return { ...base, flag: { kind: entry.flag.kind, text: entry.flag.text } };
  }
}
