// ── bgws/realtime/engine/types.ts ──────────────────────────────────────────
// The real-time mode's vocabulary.
//
// A separate game from the turn-based one. It borrows the rules' building
// blocks — the fire table, the sighting table, terrain, line of sight, fog of
// war — by importing them, and changes none of them. What is new is the
// clock: every unit acts at once, one simulated second at a time, and a
// decision is taken whenever something HAPPENS to a unit rather than once a
// turn.
//
// THE SPLIT THAT MAKES IT WORK
//   autopilot  (engine.ts)   carries out each unit's current order, every tick
//   decider    (deciders.ts) chooses a unit's next order, only on an event
//
// A crew keeps doing what it was told until something changes; then it is
// told something else. Jev is the one telling it.

import type { LatLng } from "../../lib/board";
import type { RoutePlanner } from "../../lib/routePlan";
import type { GameState, Side } from "../../lib/state";
import type { TerrainSampler } from "../../lib/lineOfSight";
import type { Rng } from "../../rules/dice";
import type { RuleSet, StandingEngagement } from "../../rules/ruleset";

/**
 * How a unit moves, which also decides what it does when it meets the enemy.
 * After Steel Beasts' route tactics and Arma's behaviour modes.
 *
 *   march     fastest; does not fire on the move; on contact halts and runs
 *             the react-to-contact drill
 *   tactical  slower; fires on the move within its ROE; on contact halts and
 *             takes cover (the default)
 *   assault   fires on the move at anything, and keeps closing until point
 *             blank — it is how ground is taken
 *   bound     bounding overwatch: moves in bounds, stopping to cover between
 *             them; paired with a friend doing the same, one always covers
 */
export type MoveMode = "march" | "tactical" | "assault" | "bound";

/** What a unit is doing. The autopilot carries it out every tick. */
export type RtOrder =
  /** Go there, at the ground's speed, along `route` when one was planned. */
  | {
      kind: "move";
      to: LatLng;
      route?: LatLng[];
      mode: MoveMode;
      /** The react-to-contact drill's dash for cover: it does not halt on contact again. */
      dash?: boolean;
    }
  /** Stay. Fire only as the rules of engagement (and self-defence) allow. */
  | { kind: "hold" }
  /** Stay and watch: fire at anything in reach, unless the ROE is "never". */
  | { kind: "overwatch" }
  /** Stay and fire at this target whenever it can be hit. */
  | { kind: "engage"; targetId: string }
  /** Get away. Moves at full speed, and does not fire. */
  | { kind: "withdraw"; to: LatLng; route?: LatLng[] };

export type Roe = StandingEngagement;

/**
 * Whether a unit is still a fighting unit. Separate from suppression, which
 * is momentary; this is the lasting effect of losses (Combat Mission's split).
 *
 *   steady   takes orders
 *   shaken   holds where it is and fires only in self-defence; runs itself
 *   broken   falls back once to a rally point and tries to rally; runs itself
 */
export type Cohesion = "steady" | "shaken" | "broken";

/** How exposed a unit is, from how long it has been still and where. */
export type Posture = "moving" | "halted" | "settled" | "hullDown";

/** What a unit is FOR, which outlasts any one order. */
export interface Mission {
  task: "take" | "hold" | "support";
  /** Where the mission is: the objective to take, or the ground to hold. */
  at?: LatLng;
  purpose: string;
}

/** One unit's real-time status, alongside its ForceElement in `game`. */
export interface RtUnit {
  order: RtOrder;
  roe: Roe;
  mission: Mission;
  /** Sim time at which its weapon can next fire. */
  weaponReadyAt: number;
  lastMovedAt: number;
  /** 0–100. From incoming fire, misses included; fades once the fire stops. */
  suppression: number;
  /** Sim time of the last incoming shot. */
  lastIncomingAt: number;
  /** Sim time suppression first reached "pinned", or null. */
  pinnedSince: number | null;
  cohesion: Cohesion;
  /** Break tests passed so far; the next is at the next loss threshold. */
  breakTests: number;
  /** A broken unit that has reached its rally point: it tries to rally there. */
  fellBack: boolean;
  lastRallyCheckAt: number;
  /** Who has fired at THIS unit, and when — for self-defence and threat. */
  attackers: Record<string, number>;
  /** Enemies this unit has seen itself — at once, before any report reaches its side. */
  ownSeen: Record<string, { time: number; level: ReportLevel }>;
  /** Sim time a friend close by was destroyed or broke. */
  lastFriendLostAt: number;
  posture: Posture;
  /** Sim time it last fired. */
  lastShotAt: number;
  /** Sim time something last happened to it, for the idle check. */
  lastEventAt: number;
  /** Bounding overwatch: moving in this bound, or covering, until `until`. */
  bound?: { moving: boolean; until: number };
  /** Identified enemies that could already reach it when its current order began. */
  exposedTo: string[];
}

export type ReportLevel = "veryPartial" | "partial" | "full";

/** A sighting on its way from the unit that made it to the rest of its side. */
export interface ContactReport {
  side: Side;
  enemyId: string;
  level: ReportLevel;
  dueAt: number;
}

/** Where an enemy was last seen, once nobody can see it any more. */
export interface LastKnown {
  at: LatLng;
  time: number;
  label?: string;
}

export interface RtState {
  /** Simulated seconds since Play was pressed. */
  time: number;
  /**
   * The board in the turn engine's shape, so the shared rules — fire,
   * sighting, fog of war, victory — can read it unchanged. `turn` is only
   * ever used as a label here. `morale` on each element is DERIVED from
   * cohesion and suppression every tick, so the fire table's modifiers see it.
   */
  game: GameState;
  units: Record<string, RtUnit>;
  /** When each side last had an enemy in sight, by enemy id. Contact fades after a while. */
  lastSeen: Record<Side, Record<string, number>>;
  /** When each enemy last fired at each side, for the "ifFiredUpon" rule. */
  lastFiredOn: Record<Side, Record<string, number>>;
  /** Sightings made but not yet reported to the side. */
  reports: ContactReport[];
  /** Faded contacts: where they were last seen. */
  lastKnown: Record<Side, Record<string, LastKnown>>;
  /** Combat strength each side started with, for the side breakpoint. */
  startStrength: Record<Side, number>;
  /** Each side's plan, in its commander's words. */
  plan: Partial<Record<Side, string>>;
  over?: { winner: Side | null; reason: string };
}

export type RtEventKind =
  | "sighted"
  | "underFire"
  | "hit"
  | "moraleDrop"
  | "friendLost"
  | "targetGone"
  | "arrived"
  | "blocked"
  | "exposed"
  /** Closed to point-blank range with an enemy: it has halted. */
  | "contact"
  /** Nothing has happened to it for a while and it is not doing its mission. */
  | "idle"
  /** Recovered from shaken or broken and takes orders again. */
  | "rallied"
  /** An enemy it can see has broken: pursue, or consolidate? */
  | "enemyBroke";

/** Something that happened to a unit. What a decider is asked about. */
export interface RtEvent {
  time: number;
  unitId: string;
  kind: RtEventKind;
  detail: string;
  /** Asked about at once, cooldown or not: hit, broken, under fire at close range. */
  severe: boolean;
}

/** A shot, for the feed and the fire lines on the map. */
export interface RtShot {
  time: number;
  firerId: string;
  targetId: string;
  result: string;
  narrative?: string;
}

/** How time maps onto the rules. See timing.ts. */
export interface RtTiming {
  /** Simulated seconds per tick. */
  tickS: number;
  /** One turn of the turn-based game, in seconds — what the rules' rates are per. */
  turnS: number;
  /** How often a unit engaging something fires. */
  shotIntervalS: number;
  /**
   * How many turn-game fire results one turn's worth of continuous fire is
   * worth. Each shot's fire-table result is scaled by
   * `lethalityPerTurn × shotIntervalS / turnS`. THE calibration knob.
   */
  lethalityPerTurn: number;
  /** How often each observer gets a sighting attempt at each enemy it can see. */
  sightingIntervalS: number;
  /** How long a contact stays on the map after the last time anyone saw it. */
  contactMemoryS: number;
  /** How often a shaken or fallen-back broken unit tries to rally. */
  rallyCheckS: number;
  /** A steady unit that is not on its mission and has been quiet this long is asked again. */
  idleS: number;
  /** How long a sighting takes to reach the rest of the side. */
  reportDelayS: number;
  /** Events for one unit within this window become one question. */
  coalesceS: number;
  /** A unit is not asked again within this long, unless the event is severe. */
  cooldownS: number;
  /** Sim time before a decision takes effect, by troop quality. */
  reactionS: (troopQuality: number) => number;
  /** The game stops here and is judged on the ground and what is left. */
  maxDurationS: number;
}

export interface RtConfig {
  ruleset: RuleSet;
  terrain: TerrainSampler;
  rng: Rng;
  timing: RtTiming;
  /**
   * How a move is routed. The terrain raster's A* on real ground; absent,
   * the offline bearing planner over `terrain`. Either way units follow the
   * waypoints rather than a straight line.
   */
  planner?: RoutePlanner;
  /**
   * Ground no unit can enter at all — a river, on the real raster. The land
   * cover classes treat water as slow rather than impassable (fords), so the
   * raster's own answer is taken when there is one.
   */
  isPassable?: (point: LatLng) => boolean;
}

/** A choice a decider may make for one unit. Generated by the rules. */
export interface RtOption {
  id: string;
  summary: string;
  order: RtOrder;
  /** Only on "roe:" options: the rules of engagement it sets. */
  roe?: Roe;
}
