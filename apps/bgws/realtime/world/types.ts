// ── realtime/world/types.ts ──────────────────────────────────────────────
// The world layer: everything the real-time game holds beyond the land
// engine's own state (docs/plan/MULTI_DOMAIN_PLAN.md §4).
//
// THE LAND ENGINE IS UNCHANGED. `RtState.world` is optional; a game without
// it (every skirmish with every other domain off) runs exactly as it always
// has, and the golden tests prove it. With it, `tick` hands the world one
// step at the end of each second (world/step.ts), and the world's systems —
// one per domain, each asking `domainMode` before it runs — read and change
// the land units through a narrow context, and everything else here.
//
// WHAT LIVES WHERE
//   land units      stay ForceElements + RtUnits, fought by engine.ts. The
//                   world adds their components (`land[unitId]`): archetype,
//                   stores, mounts, emitters, carried troops and drones.
//   entities        everything else that moves, senses or fires on its own:
//                   drones in flight, aircraft, ships, submarines, satellites,
//                   installations (depots, ports, airbases, command posts in
//                   large scopes), convoys, and munitions in flight.
//   side records    what each side knows of entities (tracks), its HQ
//                   pictures (C2), its networks, PNT, cyber accesses.
//
// Plain data throughout, never mutated once handed out (statePatch.ts relies
// on it): every step builds new objects along what it changed.

import type { LatLng } from "../../lib/board";
import type { Side } from "../../lib/state";
import type { AmmoFamily, BearerId, Classification, PlatformDomain } from "../catalogue/schema";
import type { Echelon, Planner } from "../scope";
import type { RtOrder, UnitOrders } from "../engine/types";

export type Owner = Side | "neutral";

/** Light and weather (§4.2): sensors, movement, flying and drones read them. */
export interface Environment {
  /** Sim time 0 is this hour of the day (local, 0–24). */
  startHour: number;
  /** Visibility, km. */
  visibilityKm: number;
  /** Cloud base, m above ground; EO imaging and low flying care. */
  cloudBaseM: number;
  /** Cloud cover, 0–1. EO satellites see nothing through full cover. */
  cloudCover: number;
  /** Precipitation, 0–1. */
  precipitation: number;
  /** Wind, m/s. Small drones fly badly above about 12 m/s. */
  windMs: number;
  /** Sea state, 0–9 (Douglas). Small craft and USVs suffer above 4. */
  seaState: number;
}

/** A component's lasting damage (§4.8). */
export interface DamageState {
  mobility: boolean;
  firepower: boolean;
  sensor: boolean;
  comms: boolean;
}

/** One weapon mount's magazine and when it can fire again. */
export interface MountState {
  weaponId: string;
  rounds: number;
  readyAt: number;
}

/** Stores carried (W3, W10), in the catalogue's units. */
export interface Stores {
  fuelL: number;
  ammo: Partial<Record<AmmoFamily, number>>;
  batteryWh: number;
  medical: number;
  spares: number;
}

/** EMCON (W6): a hard gate set in orders. */
export type Emcon = "silent" | "limited" | "free";

/** Weapon control status (W7): free / tight (only identified hostile) / hold. */
export type WeaponControl = "free" | "tight" | "hold";

/** The world's view of a land unit: its components, beside its ForceElement. */
export interface LandComponents {
  archetype: string;
  stores: Stores;
  /** Level of detail (§4.2): aggregated into its formation away from contact, or far enough that the rules decide for it. */
  lod?: "aggregated" | "far";
  /** While aggregated: the formation's lead unit and this unit's place relative to it, m. */
  formation?: { lead: string; eastM: number; northM: number };
  /** The order it was on when its fuel ran out: resumed when it is resupplied (W10). */
  stalledOrder?: RtOrder;
  /** Its own world weapons: indirect fire, air defence, anti-ship. */
  mounts: MountState[];
  /** Drones carried and not yet launched. */
  drones: number;
  /** Dismounted sections still carried, and the ids of those already out. */
  dismounts: { carried: number; out: string[] };
  /** Carried by this vehicle (a dismounted section's carrier). */
  carrier?: string;
  emcon: Emcon;
  weaponControl: WeaponControl;
  /** Seconds it has been transmitting since it last moved (W6 exposure). */
  onAirS: number;
  /** Dug in: 0 (not) to 1 (fully prepared position), grows while still. */
  dugIn: number;
  /** Its own position error, m: grows with GNSS loss (W6). */
  positionErrorM: number;
  /** Its last report of stocks (W10 LOGSTAT), and whether it asked for resupply. */
  stocksLowAt: number | null;
  /** Wounded waiting for evacuation, by when they were hit (W10). */
  wounded: number[];
  /** Killed in action. */
  killed: number;
  /** Mobility-killed vehicles awaiting recovery (W10 maintenance). */
  disabled: number;
  /** Last time it was in contact with its HQ (W4 outOfComms). */
  lastHqContactAt: number;
  damage: DamageState;
  /** Its HQ in the command tree (W4). */
  hq?: string;
  /** Aggregated into a formation (LOD): stands for this many platoons. */
  aggregated?: { platoons: number };
}

/** What an entity is doing (its task), set by a planner, a decision or a drill. */
export type EntityTask =
  | { kind: "idle" }
  | { kind: "route"; waypoints: LatLng[]; altM?: number; then?: EntityTask }
  | { kind: "orbit"; at: LatLng; radiusM: number; altM: number; until?: number }
  | { kind: "strike"; targetId?: string; aimAt: LatLng; altM?: number }
  | { kind: "intercept"; targetId: string }
  | { kind: "returnToBase"; baseId: string }
  | { kind: "station"; at: LatLng; radiusM: number }
  | { kind: "landed"; baseId: string; readyAt: number }
  | { kind: "loiterSearch"; at: LatLng; radiusM: number; until: number }
  | { kind: "deliver"; toUnit: string; cargo: Partial<Stores>; via: LatLng[] }
  | { kind: "reload"; portId: string; readyAt: number };

/** A munition in flight (§4.7): rounds, rockets, missiles, FPVs, torpedoes. */
export interface MunitionState {
  weaponId: string;
  launcherId: string;
  launchedAt: number;
  /** Who it is aimed at, if a platform; and where. */
  targetId?: string;
  aimAt: LatLng;
  /** Where it set off from, for ballistic arcs and counter-battery. */
  from: LatLng;
  /** Expected impact time, for ballistic and glide rounds. */
  impactAt?: number;
  /** The fire mission it belongs to. */
  missionId?: string;
  /** The kill chain it is the engagement of (W11). */
  chainId?: string;
  /** Guided by GNSS with the side's PNT degraded here at launch: its error grows. */
  pntFactor: number;
  /** A decoy: no warhead, looks the same on radar. */
  decoy?: boolean;
}

/** A drone's control link (W5). */
export interface LinkState {
  kind: "rf" | "fibre" | "satcom" | "autonomous";
  operatorId: string;
  /** 0–1. Below 0.3 control is lost. */
  quality: number;
  /** Fibre: spool left, m. */
  spoolM?: number;
}

export interface Entity {
  id: string;
  owner: Owner;
  archetype: string;
  label: string;
  domain: PlatformDomain;
  pos: LatLng;
  /** Height above sea level, m; negative is depth. */
  altM: number;
  headingDeg: number;
  speedMs: number;
  status: "ready" | "active" | "destroyed" | "expended" | "landed";
  /** Platforms still fit in it (aircraft in a flight, 1 for a ship). */
  fit: number;
  /** Staying power left, in standard hits (ships, installations). */
  hp: number;
  damage: DamageState;
  task: EntityTask;
  stores: Stores;
  mounts: MountState[];
  /** Battery or fuel endurance left, s (drones, aircraft). */
  enduranceS?: number;
  /** The base, team, ship or launcher it belongs to. */
  parent?: string;
  template: string;
  emcon: Emcon;
  weaponControl: WeaponControl;
  link?: LinkState;
  munition?: MunitionState;
  /** Satellites: orbit phase at t = 0, degrees. */
  orbitPhaseDeg?: number;
  /** Satellites: the orbit's ascending node, degrees east (with the phase, where its passes fall). */
  orbitRaanDeg?: number;
  /** Satellites: whose service it is — a military system, or a commercial provider that can cut it off (W9). */
  provider?: "military" | "commercial";
  /** Cost of the platform, for the cost-exchange report. */
  costUsd: number;
  /** Planner that owns it. */
  plannedBy?: Planner;
  /** Last time it fired, emitted, was seen: for decisions and ESM. */
  lastFiredAt: number;
  /** Fuel or battery state at which it must go home (bingo), fraction. */
  bingo?: number;
  /** Neutral shipping: the lane it follows. */
  lane?: LatLng[];
  /** For the AAR: its decisions, the latest last. */
  history: { time: number; chose: string; by: "jev" | "rules" | "drill" }[];
  /** Aircraft: the mission it is flying, and its sortie count (W7). */
  air?: AirState;
}

/** A flight's air tasking (W7): a light version of the ATO. */
export interface AirMission {
  kind: "cap" | "strike" | "sead" | "cas" | "isr" | "tanker" | "aew" | "escort" | "asw";
  /** Station, target area or orbit centre. */
  at: LatLng;
  radiusM: number;
  /** A target platform, when the mission has one. */
  targetId?: string;
  altM: number;
  /** When the mission ends and the flight comes home, at the latest. */
  until: number;
  tasked: number;
  by: string;
  /** Where to release stand-off weapons from (glide bombs). */
  releaseAt?: LatLng;
  /** A waypoint flown first: the way round a threat ring. */
  via?: LatLng;
}

export interface AirState {
  mission?: AirMission;
  sorties: number;
  /** On the ground: ready to fly again at. */
  readyAt: number;
  /** Defending against a missile until. */
  defendingUntil?: number;
  /** Has used its weapons on this sortie. */
  winchester?: boolean;
}

/** One side's track on an entity or a land unit, as its sensors or reports hold it (§4.3). */
export interface WorldTrack {
  id: string;
  /** The real thing it is a track on; a false track has none. */
  truthId?: string;
  at: LatLng;
  altM: number;
  seenAt: number;
  /** Track quality 0–15, Link-16 style: the common currency (§4.3). */
  quality: number;
  classification: Classification;
  /** What it is thought to be, as far as the classification goes: "aircraft", "MBT", a label. */
  thought: string;
  domain: PlatformDomain;
  /** Who reported it. */
  by: string;
  /** A decoy or false track (it is believed real). */
  falseTrack?: boolean;
  /** Error in position, m. */
  errorM: number;
}

/** An obstacle (W3): a minefield or a ditch, as an area. */
export interface Obstacle {
  id: string;
  owner: Side;
  kind: "minefield" | "ditch" | "dragonsTeeth" | "seaMines";
  /** A rectangle: centre, half-width along the front, half-depth, and the front's bearing. */
  at: LatLng;
  widthM: number;
  depthM: number;
  bearingDeg: number;
  /** Mines per metre of front. */
  density: number;
  /** Lanes cleared through it: each a centre point on its front line. */
  lanes: { at: LatLng; widthM: number; by: string; doneAt: number }[];
  /** Known to the other side (seen, struck, reported). */
  knownTo: Side[];
}

/** A breach under way (W3). */
export interface Breach {
  obstacleId: string;
  by: string;
  startedAt: number;
  doneAt: number;
  at: LatLng;
  /** Engineers on their way to the breach point, or at work. */
  state?: "moving" | "working";
}

/** A fire mission (W3, W11): request → fires cell → shooter → lay → fire → impact → BDA. */
export interface FireMission {
  id: string;
  side: Side;
  requestedBy: string;
  requestedAt: number;
  /** The target as the requester believes it. */
  targetAt: LatLng;
  targetId?: string;
  /** Rounds per gun. */
  rounds: number;
  shooterId?: string;
  weaponId?: string;
  state: "requested" | "assigned" | "laying" | "firing" | "inFlight" | "complete" | "refused";
  /** Step times, for the kill-chain timeline (W11). */
  times: { found?: number; requested: number; assigned?: number; fired?: number; impact?: number; assessed?: number };
  result?: string;
  /** Kill chain it is part of. */
  chainId?: string;
  priority: number;
  reason?: string;
}

/** An emission heard by the other side's ESM (W6). */
export interface Emission {
  emitterId: string;
  side: Side;
  kind: "radio" | "radar" | "datalink" | "jammer" | "droneLink" | "satcom";
  at: LatLng;
  time: number;
}

/** A direction-finding fix on an emitter (W6): an ellipse for the fires net. */
export interface EmitterFix {
  emitterId: string;
  at: LatLng;
  errorM: number;
  bearings: number;
  time: number;
  kind: Emission["kind"];
}

/** An active jammer's footprint (W6). */
export interface JammerState {
  id: string;
  side: Side;
  at: LatLng;
  radiusM: number;
  /** By band, where it differs from `radiusM`. */
  radii?: Record<string, number>;
  bands: string[];
  on: boolean;
}

/** One message on the C2 networks (W4), typed after real formats. */
export interface NetMessage {
  id: number;
  side: Side;
  kind: "spotrep" | "callForFire" | "nineLine" | "medevac" | "sitrep" | "logstat" | "frago" | "request" | "reply" | "track" | "intent" | "bda";
  from: string;
  to: string | "all";
  bearer: BearerId;
  sentAt: number;
  dueAt: number;
  /** Lost to range, jamming or a destroyed relay. */
  lost?: boolean;
  delivered?: boolean;
  text: string;
  /** What it carries, structured. */
  body: Record<string, unknown>;
  /** Hops it travelled. */
  path: string[];
}

/** An HQ: an entity or land unit that commands, with its staff, picture and seat (W4, §4.4). */
export interface Headquarters {
  id: string;
  side: Side;
  echelon: Echelon;
  /** The land unit or entity it sits in (its command post). */
  postId: string;
  parent?: string;
  subordinates: string[];
  /** The alternate HQ named in orders, if the post is lost. */
  alternate?: string;
  /** The planner that sits here, and who sits in the seat. */
  planner?: Planner;
  seat: "claude" | "heuristic" | "human";
  /** When it last re-planned, and the next time it is due to. */
  lastPlannedAt: number;
  nextPlanAt: number;
  /** A plan being made: it applies at `readyAt` (planning takes sim-time). */
  planning?: { startedAt: number; readyAt: number; why: string };
  /** Messages that have ARRIVED here, newest last. */
  inbox: number[];
  /** Its fused picture: tracks by id (§4.3), older and coarser than the truth. */
  cop: Record<string, WorldTrack>;
  /** Its staff cells' queues. */
  cells: { fires: string[]; sustainment: string[]; isr: string[] };
  /** Out of action: the post was destroyed; the seat moves at `movesAt`. */
  down?: { since: number; movesAt: number };
  /** The guidance it works inside (from the HQ above). */
  guidance?: Guidance;
}

/** A plan an HQ is making (§4.4.2): asked of its seat, applied when planning time is up. */
export interface PlanRequest {
  id: number;
  hq: string;
  side: Side;
  planner: Planner;
  seat: "claude" | "heuristic" | "human";
  raisedAt: number;
  /** Planning takes sim-time: the one-third/two-thirds rule (P1). */
  readyAt: number;
  why: string;
  /** Handed to the seat (runner bookkeeping, kept in state for replays). */
  asked?: boolean;
  answer?: PlanAnswer;
}

/** What a seat answers with: orders for its subordinates, and guidance for the HQs below. */
export interface PlanAnswer {
  by: "claude" | "heuristic" | "human";
  plan: string;
  orders: Record<string, UnitOrders>;
  structured?: Record<string, StructuredOrder>;
  guidance?: Guidance;
  /** Which subordinate is held back as the reserve. */
  reserve?: string;
  warnings?: string[];
}

/**
 * The five-paragraph order, shaped after C2SIM (W4): what UnitOrders carries
 * for the land engine, plus phase triggers, branches, control measures with
 * real geometry, weapon control, EMCON and the PACE plan.
 */
export interface StructuredOrder {
  mission: string;
  intent: string;
  /** When each phase after the first may start: at a time, on an event, or at a place. */
  phaseTriggers: (PhaseTrigger | null)[];
  branches: { label: string; when: PhaseTrigger; phases: UnitOrders["phases"] }[];
  measures: string[];
  /** A polygon it must stay inside. */
  boundary?: LatLng[];
  weaponControl: WeaponControl;
  emcon: Emcon;
  pace: BearerId[];
  sustainment?: string;
  issuedAt: number;
  by: string;
}

export type PhaseTrigger =
  | { kind: "time"; at: number }
  | { kind: "event"; event: "objectiveTaken" | "enemyBroken" | "lanesOpen" | "reserveCommitted"; side?: Side }
  | { kind: "position"; at: LatLng; withinM: number }
  | { kind: "enemySeen"; atLeast: number; withinM: number };

/** What the HQ above says (§4.4.2 JointGuidance, simplified for every level). */
export interface Guidance {
  issuedAt: number;
  by: string;
  aims: string[];
  mainEffort?: string;
  priorities: string[];
  /** Shared means apportioned: sorties, drone hours, fire missions, by type. */
  apportionment: Record<string, number>;
  roe: { weaponsFree: boolean; escalationLimit: "conventional" | "noStrategicStrike" | "noCrossBorder" };
  /** Control measures in force. */
  measures: ControlMeasure[];
  /** Where the HQ below is to go (its objective), if the task has one. */
  objective?: LatLng;
}

/** Control measures with real geometry (W4). */
export interface ControlMeasure {
  id: string;
  kind: "phaseLine" | "boundary" | "objective" | "nai" | "tai" | "fscl" | "cfl" | "nfa" | "rfa" | "roz" | "corridor" | "killBox";
  side: Side;
  label: string;
  /** A line or a polygon (closed), as points. */
  points: LatLng[];
  /** Airspace measures: lowest and highest altitude, m. */
  altM?: { min: number; max: number };
  activeFrom?: number;
  activeUntil?: number;
}

/** A kill chain (W11): one target from find to assess, every step timed. */
export interface KillChain {
  id: string;
  side: Side;
  targetTrackId: string;
  truthId?: string;
  found: number;
  fixed?: number;
  tracked?: number;
  targeted?: number;
  engaged?: number;
  assessed?: number;
  shooterId?: string;
  means?: string;
  /** When the engagement's effect is due: no assessment before it. */
  effectAt?: number;
  result?: "destroyed" | "damaged" | "missed" | "stale" | "deferred" | "unknown";
  /** Network the reports went over: voice or data link (the reference contrast). */
  network: "voice" | "datalink" | "mixed";
  costUsd: number;
}

/** A side's GNSS service, as quality in an area (W6, W9). */
export interface PntState {
  /** 0–1, before local jamming. Counterspace lowers it. */
  global: number;
}

/** A prepared cyber access (W9). */
export interface CyberAccess {
  id: string;
  side: Side;
  target: "bearer" | "logistics" | "airDefence" | "public";
  /** The bearer or system it hits. */
  system: string;
  /** Sim time from which it can be used, s (months of preparation before the game come to this). */
  readiness: number;
  /** Chance it takes effect when executed, 0–1. */
  pEffect: number;
  /** How long the effect lasts, s. */
  durationS: number;
  /** Chance the victim finds it when it is used, 0–1. */
  detectionRisk: number;
  /** Executed and, if it worked, until when. */
  executedAt?: number;
  effectUntil?: number;
  detected?: boolean;
  /** Authorised by the planner: Jev decides only timing within the window. */
  window?: { from: number; until: number };
}

/** A supply request in the logistics cell (W10). */
export interface SupplyRequest {
  id: string;
  side: Side;
  unitId: string;
  at: number;
  need: Partial<Stores>;
  state: "open" | "assigned" | "delivered" | "cancelled";
  by?: string;
}

/** A question for a world agent's leader (Jev or the rules), raised by a world system. */
export interface WorldQuestion {
  id: number;
  /** A land unit id or an entity id. */
  agentId: string;
  side: Side;
  /** The template trigger it came from: "fireRequestReceived", "linkDegraded", … */
  trigger: string;
  raisedAt: number;
  /** When the answer takes effect: raisedAt + the agent's reaction time. */
  dueAt: number;
  title: string;
  question: string;
  doctrine: string[];
  template: string;
  path: string[];
  options: WorldOption[];
  fallback: string;
  /** What the question is about, for the handler: a mission id, an enemy, a track. */
  about: Record<string, string | number | boolean>;
  /** Dispatched to a decider (runner bookkeeping, kept in state so a replay sees it). */
  asked?: boolean;
  /** The answer, once given. */
  answer?: { optionId: string; by: "jev" | "rules"; at: number; rationale?: string };
}

export interface WorldOption {
  id: string;
  /** The action in the world action library (behaviour/actions/world.ts). */
  action: string;
  summary: string;
  bias: number;
  params?: Record<string, string | number | boolean | null>;
}

/** A line for the feed and the AAR. */
export interface WorldLogItem {
  time: number;
  side: Owner;
  domain: string;
  kind: string;
  text: string;
  /** Who it is about. */
  ids?: string[];
  /** Where on the map. */
  at?: LatLng;
}

/** Running totals for the AAR and the vignette tests (W12). */
export interface WorldStats {
  /** Spent, US dollars, by side: munitions fired. */
  spentUsd: Record<Side, number>;
  /** Destroyed, US dollars, by side that lost it. */
  lostUsd: Record<Side, number>;
  interceptsTried: number;
  interceptsMade: number;
  /** By weapon id: launched and what came of them. */
  munitions: Record<string, { fired: number; hits: number; kills: number; intercepted: number }>;
  /** Where land units were hit, by distance from their own side's start line, km (kill-zone depth, W5). */
  landLossDepthKm: number[];
  /** How far from its operator each drone strike landed, km: the kill zone's reach (W5). */
  droneStrikeRangesKm?: number[];
  /** Hits on fixed sites and ships, by owner: what got through to them. */
  structureHits?: Partial<Record<Owner, number>>;
  /** Casualty evacuation times, s (W10). */
  evacuationS: number[];
  /** How late satellite imagery reached its side, s (W9). */
  imageryLatencyS?: number[];
  /** Kill chains finished: find to engage, s, by side (W11). */
  killChainS?: Record<Side, number[]>;
  /** Neutral ships that completed their passage, and neutral ships hit (W8). */
  transits?: number;
  neutralHits?: number;
  /** Casualties who died of wounds waiting for or during evacuation (W10). */
  diedOfWounds?: number;
  /** Time from a convoy's dispatch to its delivery, s (W10). */
  resupplyS?: number[];
  /** Breaches completed and how long each took, s (W3). */
  breachS: number[];
  /** Report latency at HQs, s, by side (W4). */
  reportLatencyS: Record<Side, number[]>;
  /** The same, by HQ: how late contacts reached each HQ's picture. */
  reportLatencyByHq: Record<string, number[]>;
}

/** The scheduler's queue (§4.2): future events keyed by time, ties by kind then key. */
export interface ScheduledEvent {
  at: number;
  kind: string;
  key: string;
  data?: Record<string, string | number | boolean>;
}

export interface WorldState {
  version: 1;
  env: Environment;
  land: Record<string, LandComponents>;
  entities: Record<string, Entity>;
  /** What each side knows of things the land picture does not hold (air, sea, space). */
  tracks: Record<Side, Record<string, WorldTrack>>;
  obstacles: Obstacle[];
  breaches: Breach[];
  missions: FireMission[];
  emissions: Emission[];
  fixes: Record<Side, Record<string, EmitterFix>>;
  jammers: JammerState[];
  hqs: Record<string, Headquarters>;
  messages: NetMessage[];
  measures: ControlMeasure[];
  chains: KillChain[];
  pnt: Record<Side, PntState>;
  cyber: CyberAccess[];
  supply: SupplyRequest[];
  questions: WorldQuestion[];
  queue: ScheduledEvent[];
  stats: WorldStats;
  /** Each side's start line (mean latitude of its starting units), for kill-zone depth. */
  startLines: Record<Side, LatLng>;
  /** Outages from cyber or counterspace: bearer or system → until. */
  outages: Record<string, number>;
  /** Abstracted domains: support services and when each is next available (§5.2). */
  support: SupportService[];
  /** Orders that have ARRIVED at a land unit by message (W4), for the runner to hand over (D0 if it is in a fight). */
  orderQueue: { unitId: string; orders: UnitOrders; dueAt: number; messageId: number; plan?: string }[];
  /** Plans being made at HQs (W4 planners): the runner asks the HQ's seat, and the plan goes out at `readyAt`. */
  plans: PlanRequest[];
  /** Structured orders in force, by unit (W4): what the land engine's UnitOrders do not hold. */
  structured: Record<string, StructuredOrder>;
  /** Rate limits and small per-game memories, by key ("call:b1:r2" → last time). Kept in state, never in modules, so games never leak into each other. */
  memo: Record<string, number>;
  /** Strength of land units that joined during the game, by side: counted into the breakpoint's start. */
  joinedStrength: Record<Side, number>;
  /** The sea (W8): water areas as polygons; none means open ocean everywhere. */
  sea?: SeaArea;
  /** The supply network and casualty evacuation when logistics or medical is simulated (W10). */
  logistics?: LogisticsState;
  /** Named areas of interest: where each side's collection plan wants to look (CCIR, W9). */
  nais?: NamedArea[];
  /** Satellite imagery taken, and when each batch reaches its side (W9). */
  imagery?: ImageryBatch[];
  /** How each side's kill web passes targets (W11): flat over data links, or up and down the voice nets. */
  network?: Record<Side, "voice" | "datalink">;
  /** Victory conditions beyond the land breakpoint (world/victory.ts), and how far each has got. */
  victory?: VictoryCondition[];
  /** Victory as the world judges it, for scopes whose aims are not a land breakpoint (world/victory.ts). */
  verdict?: { winner: Side | null; reason: string };
  nextId: number;
  log: WorldLogItem[];
}

/** An abstracted domain as a support service (§5.2). */
export interface SupportService {
  id: string;
  side: Side;
  domain: "air" | "fires" | "drones" | "space" | "logistics";
  label: string;
  /** Minutes from request to effect. */
  responseMin: number;
  /** How many it can give per hour. */
  perHour: number;
  /** Used in the current hour, and the hour it is counting. */
  used: number;
  hour: number;
  /** Cancelled while an air-defence threat is known this close to the target, m (air only). */
  cancelIfAdWithinM?: number;
  /** Pending deliveries. */
  pending: { at: number; dueAt: number; requestedBy: string; targetId?: string }[];
}

/**
 * A victory condition the world judges (§5.3): air superiority over an area,
 * a target set destroyed, sea control, ships through, or all of several (a
 * joint operation's aims). `holdS` is how long a state must last to count.
 */
export type VictoryCondition =
  | { kind: "airSuperiority"; side: Side; at: LatLng; radiusM: number; holdS: number; label?: string }
  | { kind: "targetSet"; side: Side; targets: string[]; share: number; label?: string }
  | { kind: "seaControl"; side: Side; at: LatLng; radiusM: number; holdS: number; label?: string }
  | { kind: "through"; side: Side; ids: string[]; at: LatLng; radiusM: number; count: number; label?: string }
  | { kind: "all"; side: Side; of: VictoryCondition[]; label?: string };

/** A unit's request for resupply (its LOGSTAT), waiting for the logistics cell. */
export interface LogRequest {
  unitId: string;
  side: Side;
  at: number;
  /** Lowest share of basic load left, fuel or ammunition, when it asked. */
  need: number;
}

/** One delivery under way: a convoy, ground robot or drone from a node to a unit. */
export interface SupplyJob {
  id: string;
  side: Side;
  unitId: string;
  nodeId: string;
  means: "convoy" | "ugv" | "drone";
  /** The convoy unit on the board (convoys only). */
  vehicleId?: string;
  state: "waiting" | "outbound" | "returning" | "done" | "failed";
  /** Held until this time (waiting for dark, scattered). */
  holdUntil?: number;
  waitForDark?: boolean;
  /** A detour round a threat on the route. */
  via?: LatLng;
  startedAt: number;
  deliveredAt?: number;
}

/** Casualties being evacuated from one unit (W10 medical). */
export interface Evacuation {
  id: string;
  side: Side;
  unitId: string;
  /** When each casualty was hit. */
  hitAt: number[];
  means: "ambulance" | "ugv" | "abstract";
  vehicleId?: string;
  state: "waiting" | "inbound" | "carrying" | "done";
  waitForDark?: boolean;
  /** Where the casualties are being taken. */
  to: LatLng;
  /** The casualty collection point, back from the fighting, where the ambulance collects them. */
  ccp: LatLng;
  doneAt?: number;
}

export interface LogisticsState {
  requests: LogRequest[];
  jobs: SupplyJob[];
  evacs: Evacuation[];
  /** Convoy and ambulance units on the board, free for the next job, by side. */
  pool: Record<Side, string[]>;
}

/** Water (W8): the sea as polygons, with bathymetry declared from the distance to the shore. */
export interface SeaArea {
  waters: LatLng[][];
}

/** One satellite image: what it saw, when, and when the side gets it after processing (W9). */
export interface ImageryBatch {
  id: string;
  side: Side;
  satelliteId: string;
  takenAt: number;
  deliveredAt: number;
  at: LatLng;
  detections: WorldTrack[];
}

/** A scenario event at a set time (W9): a provider cutting its service, counterspace, a cyber window. */
export interface ScenarioEvent {
  at: number;
  kind: "cutCommercialImagery" | "cutLeoSatcom" | "jamSatcom" | "jamGnss" | "dazzleEo" | "asat" | "restore";
  /** The side that suffers it. */
  side: Side;
  /** How long it lasts, s (none: for the rest of the game). */
  durationS?: number;
  /** A satellite, for ASAT and dazzling. */
  target?: string;
  /** GNSS quality left, for jamGnss. */
  quality?: number;
}

/** A named area of interest in a side's collection plan. */
export interface NamedArea {
  side: Side;
  label: string;
  at: LatLng;
  /** Higher is looked at first (the CCIR's order). */
  priority: number;
}
