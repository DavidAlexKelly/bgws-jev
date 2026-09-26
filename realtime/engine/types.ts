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

/** What a unit is doing. The autopilot carries it out every tick. */
export type RtOrder =
  /**
   * Go there, at the ground's speed, along `route` when one was planned.
   * Fires on the move within its rules of engagement, at the moving penalty.
   */
  | { kind: "move"; to: LatLng; route?: LatLng[] }
  /** Stay. Fire only as the rules of engagement allow. */
  | { kind: "hold" }
  /** Stay and watch: fire at anything in reach, unless the ROE is "never". */
  | { kind: "overwatch" }
  /** Stay and fire at this target whenever it can be hit. */
  | { kind: "engage"; targetId: string }
  /** Get away. Moves like `move`, and does not fire. */
  | { kind: "withdraw"; to: LatLng; route?: LatLng[] };

export type Roe = StandingEngagement;

/** One unit's real-time status, alongside its ForceElement in `game`. */
export interface RtUnit {
  order: RtOrder;
  roe: Roe;
  /** What the unit is FOR, in its commander's words. Jev reads it. */
  purpose?: string;
  /** Sim time at which its weapon can next fire. */
  weaponReadyAt: number;
  lastMovedAt: number;
  /** Sim time it was last hurt or suppressed. Morale recovers after a quiet spell. */
  lastHurtAt: number;
  /** Identified enemies that could already reach it when its current order began. */
  exposedTo: string[];
}

export interface RtState {
  /** Simulated seconds since Play was pressed. */
  time: number;
  /**
   * The board in the turn engine's shape, so the shared rules — fire,
   * sighting, fog of war, victory — can read it unchanged. `turn` is only
   * ever used as a label here.
   */
  game: GameState;
  units: Record<string, RtUnit>;
  /** When each side last had an enemy in sight, by enemy id. Contact fades after a while. */
  lastSeen: Record<Side, Record<string, number>>;
  /** When each enemy last fired at each side, for the "ifFiredUpon" rule. */
  lastFiredOn: Record<Side, Record<string, number>>;
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
  | "contact";

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
  /** How often a unit engaging something resolves a shot. The main calibration knob. */
  engagementCycleS: number;
  /** How often each observer gets a sighting attempt at each enemy it can see. */
  sightingIntervalS: number;
  /** How long a contact stays on the map after the last time anyone saw it. */
  contactMemoryS: number;
  /** Quiet time after which morale recovers one step (never from broken). */
  recoveryS: number;
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
