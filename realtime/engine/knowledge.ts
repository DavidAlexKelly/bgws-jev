// ── bgws/realtime/engine/knowledge.ts ──────────────────────────────────────
// What each unit knows: about each enemy, and about itself.
//
// docs/REALTIME_COMMAND_DESIGN.html §2. Game logic keeps both beliefs; Jev is
// only ever shown the unit's beliefs, never the truth.
//
// ABOUT EACH ENEMY
//
//   unaware     nothing. Its absence is not evidence it is not there.
//   suspected   a bearing only: fired on from the north-west, heard firing
//   located     a position, type unknown
//   identified  position and type
//   lost        out of sight: a last known position, with its age
//
// Up by a spotting roll each second (detection.ts), a LOCATE ROLL when fired
// on, or a friend's report after 15 s; down when out of sight.
//
// THE LOCATE ROLL. A shot no longer reveals the shooter automatically. The
// target always learns a bearing; whether it finds the shooter is a roll —
// easier at short range, against a shooter in the open, and after several
// volleys; harder against one that is hull-down or in cover. Each later
// volley rolls again with better odds, and a shooter that moves after firing
// starts the count again. That is what gives "keep firing while you have
// surprise" and "fire and move" their meaning.
//
// ABOUT ITSELF
//
//   unobserved     no sign it has been seen
//   possiblySeen   a cue from an enemy it can see: halted, turned towards
//                  it, went to ground, started its way
//   knownSeen      fired on
//
// Cues come only from what the unit can observe, never from whether the enemy
// really spotted it. An enemy that halts at a waypoint by coincidence gives
// the same cue as one that has seen it: that ambiguity is the real one.
//
// Every figure here is DECLARED, like detection.ts's.

import { bearingDeltaDeg, distanceM } from "../../lib/board";
import type { Belief, RtState, RtUnit, SelfBelief, Track } from "./types";

/** DECLARED. How fast a reported enemy's position goes stale, metres a second: moving, or still. */
export const DRIFT_MOVING_MS = 6;
export const DRIFT_STILL_MS = 0.5;

/** How far a reported enemy may be from where it was reported, by now. */
export function trackErrorM(track: Track, time: number): number {
  return track.errorM + Math.max(0, time - track.seenAt) * (track.moving ? DRIFT_MOVING_MS : DRIFT_STILL_MS);
}

/** How sure a position is, in words. */
export function spreadWords(m: number): string {
  if (m < 100) return "precise";
  if (m < 300) return "within a couple of hundred metres";
  if (m < 800) return "within a few hundred metres";
  return "could be a kilometre or more away from there";
}

/** DECLARED. Locate rate for one volley from a shooter in the open at 1 km: about an even chance. */
const LOCATE_RATE_AT_1KM = 0.7;
/** DECLARED. What makes a shooter easier or harder to find. */
const LOCATE_FACTORS = {
  shooterHullDown: 0.5,
  shooterInCover: 0.4,
  /** Already searching that bearing (D3). */
  targetSearching: 2,
  targetSuppressed: 0.6,
  targetPinned: 0.3,
};
/** DECLARED. Each further volley from the same place adds this much to the rate. */
const LOCATE_PER_VOLLEY = 0.5;
/** Inside this, nobody fails to see who is shooting at them. */
export const LOCATE_CERTAIN_M = 150;
/** A shooter that has moved this far since its last volley starts the count again. */
export const SHOOTER_MOVED_M = 50;

/** A bearing-only contact is forgotten after this long. */
export const SUSPECT_MEMORY_S = 120;
/** Fired on this recently: it knows it has been seen. */
export const KNOWN_SEEN_S = 60;
/** A cue this recent: it may have been seen. */
export const POSSIBLY_SEEN_S = 90;
/** At most one cue per enemy this often (D5 is rationed). */
export const CUE_INTERVAL_S = 30;
/** Cues are read off enemies within this range. */
export const CUE_RANGE_M = 3000;
/** Firing is heard this far away: a bearing for anyone who does not know the shooter. */
export const HEARD_M = 2000;
/** A turn "towards" it is within this many degrees of its bearing. */
const TOWARDS_DEG = 25;
/** A turn is a change of heading of at least this much. */
const TURN_DEG = 40;

export interface LocateConditions {
  /** Volleys from this shooter, from where it now is, this one included. */
  volleys: number;
  shooterHullDown?: boolean;
  shooterInCover?: boolean;
  targetSearching?: boolean;
  targetSuppressed?: boolean;
  targetPinned?: boolean;
}

/** The rate behind one volley's locate roll (a hazard, so volleys add up). */
function locateRate(rangeM: number, c: LocateConditions): number {
  let rate = LOCATE_RATE_AT_1KM * (1000 / Math.max(1, rangeM)) ** 2;
  rate *= 1 + LOCATE_PER_VOLLEY * Math.max(0, c.volleys - 1);
  for (const [key, factor] of Object.entries(LOCATE_FACTORS)) {
    if (c[key as keyof typeof LOCATE_FACTORS]) rate *= factor;
  }
  return rate;
}

/** The chance the target of this volley locates the shooter. */
export function locateChance(rangeM: number, c: LocateConditions): number {
  if (rangeM <= LOCATE_CERTAIN_M) return 1;
  return 1 - Math.exp(-locateRate(rangeM, c));
}

/**
 * The chance the target has located the shooter at some point in `volleys`
 * volleys from the same place: for the firer's own estimate (D6), which it
 * can only guess at from its own position and posture.
 */
export function chanceLocatedAfter(rangeM: number, volleys: number, c: Omit<LocateConditions, "volleys">): number {
  if (rangeM <= LOCATE_CERTAIN_M) return 1;
  let total = 0;
  for (let n = 1; n <= volleys; n += 1) total += locateRate(rangeM, { ...c, volleys: n });
  return 1 - Math.exp(-total);
}

/** What this unit knows of this enemy, as a belief. */
export function beliefOf(state: Pick<RtState, "game" | "units" | "lastKnown" | "time">, unitId: string, enemyId: string): Belief {
  const self = state.game.forceElements[unitId];
  const unit = state.units[unitId];
  if (!self || !unit) return "unaware";
  // Its own sight, or what it has been told — not what its side knows.
  const own: string = unit.ownSeen[enemyId]?.level ?? "none";
  const told: string = unit.picture?.[enemyId]?.level ?? "none";
  if (own === "full" || told === "full") return "identified";
  if (own !== "none" || told !== "none") return "located";
  const suspect = unit.suspects[enemyId];
  if (suspect && state.time - suspect.time <= SUSPECT_MEMORY_S) return "suspected";
  if (state.lastKnown?.[self.side]?.[enemyId]) return "lost";
  return "unaware";
}

/** Whether a unit thinks the enemy knows it is there. */
export function selfBeliefOf(unit: RtUnit, time: number): SelfBelief {
  if (time - unit.lastIncomingAt <= KNOWN_SEEN_S) return "knownSeen";
  if (unit.lastCue && time - unit.lastCue.time <= POSSIBLY_SEEN_S) return "possiblySeen";
  return "unobserved";
}

/**
 * Trying to stay hidden: waiting, observing, searching or holding, and not
 * having fired for a while. Only such a unit reads cues (D5).
 */
export function tryingToHide(unit: RtUnit, time: number): boolean {
  const still = ["wait", "observe", "search", "hold", "overwatch"].includes(unit.order.kind);
  return still && time - unit.lastShotAt > KNOWN_SEEN_S && selfBeliefOf(unit, time) !== "knownSeen";
}

/** What an enemy it is watching did in the last second, if it may mean it has been seen. */
export function cueOf(watch: {
  wasMoving: boolean;
  isMoving: boolean;
  /** Heading before and after, degrees. */
  wasFacing?: number;
  isFacing?: number;
  /** Bearing from the enemy to the watcher. */
  bearingToMe: number;
  /** It began a dash for cover this second. */
  wentToGround: boolean;
}): string | null {
  if (watch.wentToGround) return "went to ground";
  if (watch.wasMoving && !watch.isMoving) return "halted";
  const towards = watch.isFacing != null && bearingDeltaDeg(watch.isFacing, watch.bearingToMe) <= TOWARDS_DEG;
  if (!watch.wasMoving && watch.isMoving && towards) return "started moving towards us";
  if (
    watch.wasMoving &&
    watch.isMoving &&
    towards &&
    watch.wasFacing != null &&
    bearingDeltaDeg(watch.wasFacing, watch.isFacing!) >= TURN_DEG
  ) {
    return "turned towards us";
  }
  return null;
}

// ── Words, not numbers ─────────────────────────────────────────────────────
//
// Jev is shown odds and distances as bands; the exact figures go to the
// console. Its research notes: numbers in a prompt are a weakness, words are not.

/** A probability, in words. */
export function chanceBand(p: number): string {
  if (p < 0.05) return "almost no chance";
  if (p < 0.25) return "unlikely";
  if (p < 0.45) return "possible";
  if (p < 0.65) return "about even";
  if (p < 0.9) return "likely";
  return "almost certain";
}

/** A range, in words. */
export function rangeBand(m: number): string {
  if (m < 300) return "point-blank";
  if (m < 800) return "close";
  if (m < 1500) return "medium range";
  if (m < 2500) return "long range";
  return "very long range";
}

/** Seconds, in words. */
export function agoBand(s: number): string {
  if (s < 20) return "just now";
  if (s < 60) return "under a minute ago";
  if (s < 180) return "a few minutes ago";
  return "a while ago";
}

/** How a unit sees each enemy it knows anything about, for Jev. */
export function beliefsOf(state: RtState, unitId: string) {
  const self = state.game.forceElements[unitId];
  const unit = state.units[unitId];
  if (!self || !unit) return [];
  return Object.values(state.game.forceElements)
    .filter((enemy) => enemy.side !== self.side && enemy.combatStrength > 0)
    .map((enemy) => ({ enemy, belief: beliefOf(state, unitId, enemy.id) }))
    .filter(({ belief }) => belief !== "unaware")
    .map(({ enemy, belief }) => {
      if (belief === "suspected") {
        const s = unit.suspects[enemy.id];
        return { id: `unknown-${Math.round(s.bearingDeg)}`, belief, bearing: compassWord(s.bearingDeg), why: s.why, when: agoBand(state.time - s.time) };
      }
      if (belief === "lost") {
        const seen = state.lastKnown[self.side][enemy.id];
        return {
          id: enemy.id,
          belief,
          lastSeen: `${rangeBand(distanceM(self.position, seen.at))}, ${agoBand(state.time - seen.time)}`,
          ...(seen.label ? { unit: seen.label } : {}),
        };
      }
      // Where it is as this unit believes: in sight, or where it was reported.
      const own = unit.ownSeen[enemy.id];
      const told = unit.picture?.[enemy.id];
      const inSight = own != null && state.time - own.time <= 5;
      const useReport = !own && told != null;
      const where = useReport ? told.at : own && !inSight && own.at ? own.at : enemy.position;
      return {
        id: enemy.id,
        belief,
        ...(belief === "identified" ? { unit: told?.label ?? enemy.label } : {}),
        range: rangeBand(distanceM(self.position, where)),
        source: inSight
          ? "in sight"
          : own
            ? `seen ${agoBand(state.time - own.time)}`
            : `reported by ${told!.from}${told!.via ? ` (passed on by ${told!.via})` : ""}, seen ${agoBand(state.time - told!.seenAt)}; ${spreadWords(trackErrorM(told!, state.time))}`,
      };
    });
}

const COMPASS_WORDS = ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"];
/** A bearing as a compass word. */
export function compassWord(bearingDeg: number): string {
  return COMPASS_WORDS[Math.round((((bearingDeg % 360) + 360) % 360) / 45) % 8];
}
