// ── picture.ts ───────────────────────────────────────────────────────────
// What a unit has been told — stored once per side (docs/plan/SIDE_PICTURE.md).
//
// Each side keeps one record of the reports that have reached its units:
// per enemy, a few VERSIONS of the report, each with the set of units that
// hold it (`heardBy`, a bitset by element order). Every unit holds at most
// one version per enemy. A unit's picture is not stored anywhere: it is
// "the version of each report this unit holds", read through `trackFor` and
// `trackedBy`, and written only through `ReportWriter`.
//
// Before, every unit kept its own copy of everything it had been told (about
// 190 tracks each at 600 units, ~13 MB), and one broadcast report copied the
// picture of every friend that heard it. Now a delivery is one version and a
// bit per listener, and forgetting stale reports is once per side.
//
// THE RULES ARE THOSE OF THE OLD PER-UNIT PICTURE, unit by unit: comms still
// decides, for each listener, whether it takes the report and at what level
// (its own sighting, what it held before), and the listener then holds
// exactly that track. Listeners who end up with the same track share one
// version. Differences from before, all deliberate:
//   - A dead unit's picture is forgotten with everyone else's (before, a
//     dead unit kept its last picture for ever; nothing reads it).
//   - `trackedBy` walks a unit's reports in the order the SIDE first heard
//     of each enemy, not the order that unit did. Readers that depend on
//     order (the HQ picture's tie-break) may differ in a tie.

import type { Side } from "../../lib/state";
import { elementRank, type ElementRank } from "./rank";
import type { GameState } from "../../lib/state";
import type { ReportVersion, RtState, RtUnit, SideReports, Track } from "./types";

/** Where pictures are read from: a state, or `tick`'s live view of its own. */
export interface PictureSource {
  readonly units: Readonly<Record<string, RtUnit>>;
  readonly game: GameState;
  readonly reports: RtState["reports"];
  /** Element order, if the caller has it; otherwise looked up (and cached) from `game`. */
  readonly rank?: ElementRank;
}

export const NO_REPORTS: RtState["reports"] = { blue: {}, red: {} };

const has = (bits: Uint8Array, slot: number) => (bits[slot >> 3] & (1 << (slot & 7))) !== 0;

/**
 * A unit as a reader of its side's record: which record, and its bit. Work
 * it out once and ask `heldTrack` many times — the hot loops do.
 */
export interface Holder {
  readonly reports: SideReports;
  readonly byte: number;
  readonly bit: number;
}

export function holderOf(source: PictureSource, unitId: string): Holder | undefined {
  const fe = source.game.forceElements[unitId];
  if (!fe || !source.units[unitId]) return undefined;
  const reports = source.reports?.[fe.side];
  const slot = (source.rank ?? elementRank(source.game)).index.get(unitId);
  if (!reports || slot == null) return undefined;
  return { reports, byte: slot >> 3, bit: 1 << (slot & 7) };
}

/** The holder's report of this enemy, if it holds one. */
export function heldTrack(holder: Holder | undefined, enemyId: string): Track | undefined {
  const versions = holder?.reports[enemyId];
  if (!versions) return undefined;
  for (const version of versions) if ((version.heardBy[holder.byte] & holder.bit) !== 0) return version.track;
  return undefined;
}

/** This unit's report of this enemy, if it holds one. */
export function trackFor(source: PictureSource, unitId: string, enemyId: string): Track | undefined {
  return heldTrack(holderOf(source, unitId), enemyId);
}

/** Every report this unit holds, in the order its side first heard of each enemy. */
export function trackedBy(source: PictureSource, unitId: string, visit: (enemyId: string, track: Track) => void): void {
  const holder = holderOf(source, unitId);
  if (!holder) return;
  for (const enemyId in holder.reports) {
    for (const version of holder.reports[enemyId]) {
      if ((version.heardBy[holder.byte] & holder.bit) !== 0) {
        visit(enemyId, version.track);
        break;
      }
    }
  }
}

/** Every report this unit holds, as [enemyId, track] pairs in `trackedBy` order. For display. */
export function reportsHeldBy(source: PictureSource, unitId: string): [string, Track][] {
  const out: [string, Track][] = [];
  trackedBy(source, unitId, (enemyId, track) => out.push([enemyId, track]));
  return out;
}

/**
 * What a side has heard about each enemy, as a whole: the newest report any
 * of its living units holds, and how many hold it. The commander's view of
 * the net (for Jev), not any one unit's.
 */
export function sideReports(source: PictureSource, side: Side): { enemyId: string; track: Track; holders: number }[] {
  const reports = source.reports?.[side];
  if (!reports) return [];
  const rank = source.rank ?? elementRank(source.game);
  const living = (slot: number) => {
    const id = rank.ids[slot];
    return (source.game.forceElements[id]?.combatStrength ?? 0) > 0 && source.units[id] != null;
  };
  const out: { enemyId: string; track: Track; holders: number }[] = [];
  for (const enemyId in reports) {
    let best: Track | undefined;
    let holders = 0;
    for (const version of reports[enemyId]) {
      let count = 0;
      for (let slot = 0; slot < rank.ids.length; slot++) if (has(version.heardBy, slot) && living(slot)) count++;
      if (count === 0) continue;
      holders += count;
      if (!best || version.track.seenAt > best.seenAt) best = version.track;
    }
    if (best) out.push({ enemyId, track: best, holders });
  }
  return out;
}

const sameTrack = (a: Track, b: Track) =>
  a.level === b.level &&
  a.at.lat === b.at.lat &&
  a.at.lng === b.at.lng &&
  a.seenAt === b.seenAt &&
  a.receivedAt === b.receivedAt &&
  a.from === b.from &&
  a.via === b.via &&
  a.errorM === b.errorM &&
  a.moving === b.moving &&
  a.label === b.label;

/**
 * The only way to change who holds what. One per tick (or per one-off
 * change): it copies the record, an enemy's version list, or a version, the
 * first time it changes each — never the previous state's objects, which the
 * worker's patches and the replays rely on being untouched.
 */
export class ReportWriter {
  private current: RtState["reports"];
  private readonly ownSides = new Set<Side>();
  private readonly ownLists = new Set<ReportVersion[]>();
  private readonly ownVersions = new Set<ReportVersion>();
  private readonly bytes: number;

  constructor(
    previous: RtState["reports"] | undefined,
    private readonly rank: ElementRank,
  ) {
    this.current = previous ?? NO_REPORTS;
    this.bytes = Math.ceil(rank.ids.length / 8);
  }

  /** The record as it stands. */
  get reports(): RtState["reports"] {
    return this.current;
  }

  private side(side: Side): SideReports {
    if (!this.ownSides.has(side)) {
      this.current = { ...this.current, [side]: { ...this.current[side] } };
      this.ownSides.add(side);
    }
    return this.current[side];
  }

  private list(side: Side, enemyId: string): ReportVersion[] {
    const reports = this.side(side);
    let list = reports[enemyId];
    if (!list) {
      list = [];
      reports[enemyId] = list;
      this.ownLists.add(list);
    } else if (!this.ownLists.has(list)) {
      list = [...list];
      reports[enemyId] = list;
      this.ownLists.add(list);
    }
    return list;
  }

  private version(list: ReportVersion[], i: number): ReportVersion {
    let version = list[i];
    if (!this.ownVersions.has(version)) {
      version = { track: version.track, heardBy: version.heardBy.slice() };
      list[i] = version;
      this.ownVersions.add(version);
    }
    return version;
  }

  /** This unit (on `side`) now holds `track` for this enemy, instead of whatever it held. */
  hear(side: Side, unitId: string, enemyId: string, track: Track): void {
    const slot = this.rank.index.get(unitId);
    if (slot == null) return;
    const byte = slot >> 3;
    const bit = 1 << (slot & 7);
    const list = this.list(side, enemyId);
    for (let i = 0; i < list.length; i++) {
      if ((list[i].heardBy[byte] & bit) === 0) continue;
      const version = this.version(list, i);
      version.heardBy[byte] &= ~bit;
      if (version.heardBy.every((b) => b === 0)) list.splice(i, 1);
      break;
    }
    // Listeners to the same message usually end up with the same track: one version between them.
    for (const version of list) {
      if (this.ownVersions.has(version) && sameTrack(version.track, track)) {
        version.heardBy[byte] |= bit;
        return;
      }
    }
    const heardBy = new Uint8Array(this.bytes);
    heardBy[byte] |= bit;
    const version: ReportVersion = { track, heardBy };
    this.ownVersions.add(version);
    list.push(version);
  }

  /** Drop every version `keep` rejects: stale, or about the dead. Once per side per step. */
  forget(side: Side, keep: (enemyId: string, track: Track) => boolean): void {
    const reports = this.current[side];
    for (const enemyId in reports) {
      const versions = reports[enemyId];
      if (versions.every((version) => keep(enemyId, version.track))) continue;
      const kept = versions.filter((version) => keep(enemyId, version.track));
      const own = this.side(side);
      if (kept.length) {
        own[enemyId] = kept;
        this.ownLists.add(kept);
      } else {
        delete own[enemyId];
      }
    }
  }
}

/** A state in which this unit holds `track` for this enemy (tests and set pieces). */
export function withReport<S extends RtState>(state: S, unitId: string, enemyId: string, track: Track): S {
  const side = state.game.forceElements[unitId]?.side;
  if (!side) return state;
  const writer = new ReportWriter(state.reports, elementRank(state.game));
  writer.hear(side, unitId, enemyId, track);
  return { ...state, reports: writer.reports };
}
