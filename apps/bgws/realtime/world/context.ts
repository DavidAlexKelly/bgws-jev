// ── realtime/world/context.ts ────────────────────────────────────────────
// What a world system sees during a tick, and how it changes things.
//
// `tick` (engine.ts) builds one `WorldCtx` at the end of each second and
// hands it to `stepWorld`. The land side is reached only through the same
// closures the engine itself uses (setFe, setUnit, emit, the radio net), so a
// world system cannot change a land unit in a way the engine would not.
//
// The world side is a working copy made at the start of the step: top-level
// collections are copied once, and each entity or land record is replaced,
// never edited in place, so unchanged parts stay the same objects and the
// worker's patches stay small (statePatch.ts).

import type { LatLng } from "../../lib/board";
import type { ForceElement, GameState, SightingLevel } from "../../lib/state";
import type { Rng } from "../../rules/dice";
import type { MessageDraft } from "../engine/comms";
import type { RtConfig, RtEvent, RtOrder, RtShot, RtState, RtUnit } from "../engine/types";
import type { Domain, DomainMode } from "../scope";
import type { Entity, LandComponents, WorldLogItem, WorldState } from "./types";
import { seaTerrain } from "./sea";

const SEA_CONFIGS = new WeakMap<RtConfig, WeakMap<object, RtConfig>>();

export type Emit = (
  unitId: string,
  kind: RtEvent["kind"],
  detail: string,
  severe?: boolean,
  extra?: Partial<Pick<RtEvent, "about" | "located" | "bearingDeg" | "info" | "requestId" | "reported">>,
) => void;

/** The land engine's view, as `tick` lends it. */
export interface LandView {
  readonly time: number;
  readonly config: RtConfig;
  readonly rng: Rng;
  readonly game: GameState;
  readonly units: Record<string, RtUnit>;
  setFe(id: string, patch: Partial<ForceElement>): void;
  setUnit(id: string, patch: Partial<RtUnit>): void;
  /** Give a land unit a new order, routed from where it stands (engine.ts `routed`). */
  order(id: string, order: RtOrder, extra?: Partial<RtUnit>): void;
  /** Put a new land unit on the board (a dismounted section), appended in element order. */
  addUnit(fe: ForceElement, unit: RtUnit): void;
  /** Direct-fire shots the land engine made this second (empty outside a tick). */
  shots: readonly RtShot[];
  emit: Emit;
  /** A message on the land radio net (comms.ts). */
  radio(draft: MessageDraft): void;
  known(id: string, enemyId: string): SightingLevel;
  /** The state as it stood at the start of this tick. */
  prev: RtState;
}

export class WorldCtx {
  readonly world: WorldState;
  readonly newLog: WorldLogItem[] = [];

  constructor(
    prev: WorldState,
    readonly land: LandView,
  ) {
    this.world = {
      ...prev,
      land: { ...prev.land },
      entities: { ...prev.entities },
      tracks: { blue: { ...prev.tracks.blue }, red: { ...prev.tracks.red } },
      obstacles: [...prev.obstacles],
      breaches: [...prev.breaches],
      missions: [...prev.missions],
      // Emissions of the last few seconds: what a listener scanning now can have heard.
      emissions: prev.emissions.filter((e) => e.time > land.time - 10),
      fixes: { blue: { ...prev.fixes.blue }, red: { ...prev.fixes.red } },
      jammers: [...prev.jammers],
      hqs: { ...prev.hqs },
      messages: [...prev.messages],
      chains: [...prev.chains],
      pnt: { ...prev.pnt },
      cyber: [...prev.cyber],
      supply: [...prev.supply],
      questions: [...prev.questions],
      queue: [...prev.queue],
      stats: prev.stats,
      outages: { ...prev.outages },
      support: [...prev.support],
      orderQueue: [...prev.orderQueue],
      plans: [...prev.plans],
      structured: { ...prev.structured },
      memo: { ...prev.memo },
      log: prev.log,
    };
  }

  get time(): number {
    return this.land.time;
  }
  /** The game's config, its terrain seen with the sea at sea level when there is one (W8). */
  get config(): RtConfig {
    const sea = this.world.sea;
    if (!sea) return this.land.config;
    let byConfig = SEA_CONFIGS.get(this.land.config);
    if (!byConfig) SEA_CONFIGS.set(this.land.config, (byConfig = new WeakMap()));
    let config = byConfig.get(sea);
    if (!config) byConfig.set(sea, (config = { ...this.land.config, terrain: seaTerrain(this.land.config.terrain, sea) }));
    return config;
  }
  get rng(): Rng {
    return this.land.rng;
  }
  get game(): GameState {
    return this.land.game;
  }

  mode(domain: Domain): DomainMode {
    return this.config.scope?.domains[domain] ?? (domain === "land" ? "simulated" : "off");
  }
  simulated(domain: Domain): boolean {
    return this.mode(domain) === "simulated";
  }

  /** A chance, from the game's dice: the one place world systems roll. */
  chance(p: number): boolean {
    if (p <= 0) return false;
    if (p >= 1) return true;
    return this.rng.int(1_000_000) < p * 1_000_000;
  }

  newId(prefix: string): string {
    const id = `${prefix}-${this.world.nextId}`;
    (this.world as { nextId: number }).nextId = this.world.nextId + 1;
    return id;
  }
  newNumber(): number {
    const n = this.world.nextId;
    (this.world as { nextId: number }).nextId = n + 1;
    return n;
  }

  fe(id: string): ForceElement | undefined {
    return this.game.forceElements[id];
  }
  /** A living land unit. */
  landAlive(id: string): boolean {
    return (this.fe(id)?.combatStrength ?? 0) > 0 && this.land.units[id] != null;
  }
  comps(id: string): LandComponents | undefined {
    return this.world.land[id];
  }
  setLand(id: string, patch: Partial<LandComponents>): void {
    const was = this.world.land[id];
    if (was) this.world.land[id] = { ...was, ...patch };
  }
  entity(id: string): Entity | undefined {
    return this.world.entities[id];
  }
  setEntity(id: string, patch: Partial<Entity>): void {
    const was = this.world.entities[id];
    if (was) this.world.entities[id] = { ...was, ...patch };
  }
  addEntity(entity: Entity): void {
    this.world.entities[entity.id] = entity;
  }
  /** Where a land unit or an entity is. */
  where(id: string): LatLng | undefined {
    return this.fe(id)?.position ?? this.entity(id)?.pos;
  }
  sideOf(id: string): "blue" | "red" | "neutral" | undefined {
    return this.fe(id)?.side ?? this.entity(id)?.owner;
  }

  log(item: Omit<WorldLogItem, "time">): void {
    const full = { time: this.time, ...item };
    this.newLog.push(full);
  }

  /** Something due later (the scheduler, §4.2). */
  schedule(at: number, kind: string, key: string, data?: Record<string, string | number | boolean>): void {
    const queue = this.world.queue;
    const event = { at, kind, key, ...(data ? { data } : {}) };
    // Kept sorted by (time, kind, key): ties never fall to insertion order.
    let i = queue.length;
    while (i > 0 && compareEvents(queue[i - 1], event) > 0) i--;
    queue.splice(i, 0, event);
  }

  /** Everything due now, in order, taken off the queue. */
  due(): WorldState["queue"] {
    const queue = this.world.queue;
    let n = 0;
    while (n < queue.length && queue[n].at <= this.time) n++;
    return n ? queue.splice(0, n) : [];
  }

  /** When this key last happened (rate limits); −∞ if never. */
  last(key: string): number {
    return this.world.memo[key] ?? -Infinity;
  }
  mark(key: string): void {
    this.world.memo[key] = this.time;
  }

  /** Copy-on-write for the stats record. */
  stat(change: (stats: WorldState["stats"]) => WorldState["stats"]): void {
    (this.world as { stats: WorldState["stats"] }).stats = change(this.world.stats);
  }
}

export function compareEvents(a: { at: number; kind: string; key: string }, b: { at: number; kind: string; key: string }): number {
  return a.at - b.at || (a.kind < b.kind ? -1 : a.kind > b.kind ? 1 : 0) || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
}
