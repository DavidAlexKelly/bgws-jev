// ── realtime/world/step.ts ───────────────────────────────────────────────
// One second of the world layer, called by `tick` at the end of each second
// when the game has a world (engine.ts §12).
//
// Systems run in a fixed order — each domain registers its own (world/index.ts
// loads them) with an `order` and a `when` that asks the scope's domain mode
// — and every random draw comes from the game's one generator, so the same
// seed and the same answers give the same game. Then the scheduler's due
// events (§4.2) go to whichever system registered their kind.
//
//   10 environment      light, weather
//   20 c2 networks      bearers, nets, messages, HQ pictures      (W4)
//   30 ew               emitters, ESM, jamming, PNT               (W6)
//   40 sensing          world sensors: radar, drones, ships, sats (W2, W9)
//   50 command          planners on their battle rhythm, seats    (W4)
//   60 fires            fire missions, counter-battery           (W3, W11)
//   70 movers           drones, aircraft, ships, convoys          (W5, W7, W8, W10)
//   80 munitions        rounds and missiles in flight, intercepts (W2, W7, W8)
//   90 land extensions  stocks, obstacles, dismounts, digging     (W3)
//  100 logistics        requests, convoys, medical               (W10)
//  110 space and cyber  passes, accesses                          (W9)
//  120 kill web         nominations, BDA                          (W11)
//  130 support          abstracted domains as support services   (§5.2)
//  140 aggregation      formations away from contact              (§4.2)

import type { WorldCtx } from "./context";
import { wfig } from "./figures";
import type { ScheduledEvent, WorldState } from "./types";

export interface WorldSystem {
  name: string;
  order: number;
  /** Runs this second? Usually: is its domain simulated (or abstracted) in this game's scope. */
  when(c: WorldCtx): boolean;
  run(c: WorldCtx): void;
}

const SYSTEMS: WorldSystem[] = [];
const SCHEDULED = new Map<string, (c: WorldCtx, event: ScheduledEvent) => void>();

export function registerSystem(system: WorldSystem): void {
  const at = SYSTEMS.findIndex((s) => s.name === system.name);
  if (at >= 0) SYSTEMS.splice(at, 1);
  SYSTEMS.push(system);
  SYSTEMS.sort((a, b) => a.order - b.order || a.name.localeCompare(b.name));
}

/** What to do when a scheduled event of this kind falls due. */
export function onScheduled(kind: string, handler: (c: WorldCtx, event: ScheduledEvent) => void): void {
  SCHEDULED.set(kind, handler);
}

export function registeredSystems(): readonly WorldSystem[] {
  return SYSTEMS;
}

/** Run every system that applies, then the scheduler's due events, in order. */
export function runSystems(c: WorldCtx): void {
  for (const system of SYSTEMS) if (system.when(c)) system.run(c);
  for (const event of c.due()) SCHEDULED.get(event.kind)?.(c, event);
  // A question about an agent that has since died or been destroyed asks nobody.
  const live = c.world.questions.filter((q) => c.landAlive(q.agentId) || ((c.entity(q.agentId)?.status ?? "destroyed") !== "destroyed" && (c.entity(q.agentId)?.status ?? "") !== "expended"));
  if (live.length !== c.world.questions.length) {
    c.world.questions.length = 0;
    c.world.questions.push(...live);
  }
}

/** The world as a context left it, with its new feed lines on the end of the log. */
export function finishWorld(c: WorldCtx): WorldState {
  const w = c.world;
  if (!c.newLog.length) return w;
  return { ...w, log: [...w.log, ...c.newLog].slice(-wfig("world/step.LOG_KEEP")) };
}
