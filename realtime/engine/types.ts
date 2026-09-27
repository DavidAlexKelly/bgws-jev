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

/**
 * What a waiting unit is waiting FOR (decision point D1 → D4). Concrete, so
 * game logic can watch it every second and Jev does not have to be asked
 * "is it close enough yet?".
 */
export type Trigger =
  /** Its chance of a hit reaches this; `aboutM` is the range that roughly gives it, for the words. */
  | { kind: "hitChance"; atLeast: number; aboutM: number }
  | { kind: "range"; withinM: number }
  /** The target shows its side or rear. */
  | { kind: "flank" }
  /** The target reaches a place: within `withinM` of `at`. */
  | { kind: "reaches"; at: LatLng; withinM: number; label: string };

/**
 * Carried by an order that came from the standing orders (Claude's phases):
 * when it is done, the next phase follows with no decision (D10).
 */
export interface Planned {
  /** The index of the phase this order carries out. */
  phase?: number;
}

/** What a unit is doing. The autopilot carries it out every tick. */
export type RtOrder = Planned &
  (
    /** Go there, at the ground's speed, along `route` when one was planned; then `then`, if given. */
    | {
        kind: "move";
        to: LatLng;
        route?: LatLng[];
        mode: MoveMode;
        /** The react-to-contact drill's dash for cover: it does not halt on contact again. */
        dash?: boolean;
        then?: RtOrder;
      }
    /** Stay. Fire only as the rules of engagement (and self-defence) allow. */
    | { kind: "hold" }
    /** Stay and watch: fire at anything in reach, unless the ROE is "never". */
    | { kind: "overwatch" }
    /**
     * Stay and fire at this target whenever it can be hit. With `volleys`,
     * only that many; with `until`, only until then; either way, then `then`
     * (or hold). Also `then` when the target is destroyed or lost.
     */
    | { kind: "engage"; targetId: string; volleys?: number; until?: number; then?: RtOrder }
    /** Get away. Moves at full speed, and does not fire; then `then`, if given. */
    | { kind: "withdraw"; to: LatLng; route?: LatLng[]; then?: RtOrder }
    /**
     * Stay hidden with a target in view, not firing, until the trigger is
     * met: then fire at once (`autoFire`) or ask (D4). `met` once it has asked.
     */
    | { kind: "wait"; targetId: string; trigger: Trigger; autoFire: boolean; met?: boolean }
    /** Stay still and watch, without firing: what it sees reaches the side. */
    | { kind: "observe" }
    /** Hold still and search a bearing for a shooter it has not located: spotting doubled there. */
    | { kind: "search"; bearingDeg: number; until: number }
  );

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
  /** Its own fire on its current target, for "is this working?". */
  engagement: Engagement | null;
  /** Fire it has taken, by who fired it. */
  incoming: Record<string, { shots: number; damage: number; since: number; last: number }>;
  /** Strength it has taken off the enemy, in all. */
  dealt: number;
  /** When its fire was last reviewed (see the "review" and "ineffective" events). */
  lastReviewAt: number;
  /** Its last few decisions — Jev's, the rules' or the crew's own drill — and where it stood then. */
  history: DecisionMemory[];
  /**
   * The vehicles (or teams) in it. A round knocks out one vehicle, not
   * "3 combat strength"; strength follows from how many are still fit.
   */
  vehicles: { total: number; fit: number };
  /** Laying on a new target: the first round goes when acquisition is done. */
  laying?: { targetId: string; readyAt: number };
  /** When each of its vehicles was knocked out: fresh wrecks still draw fire for a while. */
  losses: number[];

  // ── What it knows (knowledge.ts) ────────────────────────────────────────
  /** Enemies it knows only by a bearing: fired on from there, or heard firing. */
  suspects: Record<string, Suspicion>;
  /** Locate rolls against each shooter it has not found: volleys so far, and where the shooter fired from. */
  locating: Record<string, { volleys: number; at: LatLng; time: number }>;
  /** The last sign that an enemy it can see may have seen it (D5). */
  lastCue: { time: number; enemyId: string; cue: string } | null;
  /** When it last had a cue from each enemy: at most one every 30 s. */
  cueAt: Record<string, number>;
  /** When it last offered help to each friend (D9). */
  helpAt: Record<string, number>;

  /** The orders the player's commander (Claude) gave it, or null. */
  orders: UnitOrders | null;
}

/** A bearing-only contact: it knows something is there, not where. */
export interface Suspicion {
  bearingDeg: number;
  time: number;
  /** Where the unit stood when it formed the suspicion. */
  from: LatLng;
  why: string;
}

/**
 * Knowledge of one enemy, as one unit holds it. Moves up by spotting, a
 * locate roll when fired on, or a friend's report; down when out of sight.
 */
export type Belief = "unaware" | "suspected" | "located" | "identified" | "lost";

/** Whether a unit thinks the enemy knows it is there. From cues, never from the truth. */
export type SelfBelief = "unobserved" | "possiblySeen" | "knownSeen";

/** What a unit does on contact when nobody decides otherwise: the rules' fallback. */
export type OnContact = "engage" | "observe" | "avoid" | "bypass";

/** "now" breaks off a fight to comply; "whenAble" lets it finish first (D0). */
export type Urgency = "now" | "whenAble";

/** A line a unit must not cross: "stay south of the road". */
export interface Boundary {
  /** Which side of the line it must stay on. */
  keep: "north" | "south" | "east" | "west";
  /** A point on the line (east-west for north/south, north-south for east/west). */
  at: LatLng;
  label: string;
}

/** One step of a task: "advance to the ridge", then "overwatch the bridge". */
export interface Phase {
  label: string;
  order: RtOrder;
}

/**
 * Mission orders, written by the player's commander while the clock is
 * stopped: a task in phases, the intent Jev weighs every decision against,
 * an urgency, and constraints game logic enforces.
 */
export interface UnitOrders {
  task: string;
  phases: Phase[];
  /** The phase being carried out now. */
  phase: number;
  intent: string;
  urgency: Urgency;
  /** Its rules of engagement: a constraint, enforced by game logic. */
  roe: Roe;
  onContact: OnContact;
  boundaries: Boundary[];
  by: "claude" | "heuristic" | "rules";
  issuedAt: number;
  /**
   * A phase it found it cannot carry out (the ground, or a line it may not
   * cross). It is not sent back to it; the player is told and it waits for
   * new orders.
   */
  blocked?: { phase: number; time: number; why: string };
  /** Every phase carried out: it waits for new orders. */
  done?: boolean;
}

/** One unit's fire on one target, since it started. */
export interface Engagement {
  targetId: string;
  since: number;
  lastShotAt: number;
  shots: number;
  /** Fire-table hits: rounds that struck, whether or not they did damage. */
  hits: number;
  /** Strength taken off the target. */
  damage: number;
  /** The same, since the last review. */
  window: { since: number; shots: number; hits: number; damage: number };
}

/** What a unit was told, when, by whom, and how things stood — so the next decision can see what came of it. */
export interface DecisionMemory {
  time: number;
  chose: string;
  by: "jev" | "rules" | "crew";
  /** What it was asked about. */
  because: string;
  strength: number;
  dealt: number;
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
  | "enemyBroke"
  /** It has been firing for a while and doing no damage: change something? */
  | "ineffective"
  /** A long exchange of fire: a periodic check that the plan still holds. */
  | "review"
  /** A waiting unit's trigger has been met (D4), or fired automatically. */
  | "triggerMet"
  /** An enemy it can see did something that may mean it has been seen (D5). */
  | "cue"
  /** It has fired its first volley at a target, or knocked a vehicle out (D6). */
  | "volley"
  /** A friend close by is under fire or losing vehicles, and this unit can help (D9). */
  | "friendNeedsHelp"
  /** A search of a bearing ran its course without finding the shooter (D3 again). */
  | "searchDone"
  /** A phase of its orders is done and the next has begun: no decision. */
  | "phaseDone"
  /** Its orders are done: nothing left to carry out (D10, and a flag for the player). */
  | "outOfOrders"
  /** New orders from the player's commander, arriving while it is in a fight (D0). */
  | "newOrders";

/** Something that happened to a unit. What a decider is asked about. */
export interface RtEvent {
  time: number;
  unitId: string;
  kind: RtEventKind;
  detail: string;
  /** Asked about at once, cooldown or not: hit, broken, under fire at close range. */
  severe: boolean;
  /** The enemy (or friend, for friendNeedsHelp) it is about. */
  about?: string;
  /** Fired on: whether this unit located the shooter (D2) or has only a bearing (D3). */
  located?: boolean;
  /** Fired on without locating: the bearing the fire came from. */
  bearingDeg?: number;
  /** For the feed only: nobody is asked about it. */
  info?: boolean;
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
  /**
   * The least time between aimed shots for one vehicle: an engagement cycle
   * (acquire, lay, fire, observe). Each weapon's own rate of fire (fire.ts,
   * from L7) is slower still for most guns.
   */
    shotIntervalS: number;
  /**
   * Multiplies the chance a fire-table hit is a round on target (fire.ts).
   * 1 is the realistic figure; tests set 0 to guarantee nothing lands.
   * Not a pacing knob: rates of fire come from the data.
   */
  strikeScale: number;
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
  /** Chance per minute of damaging the enemy it is about, from where the order leaves it. */
  effect?: number;
  /** Chance a hit gets through the face it would strike. */
  penetrate?: number;
  /** The exact figures behind the words, for the console only: Jev sees bands. */
  exact?: string;
  /** Orders other units take at the same moment: a friend's covering fire. */
  also?: { unitId: string; order: RtOrder }[];
  /** Standing orders it takes on with this choice (D0). */
  orders?: UnitOrders;
}
