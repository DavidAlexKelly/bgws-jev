// ── bgws/realtime/overlays.ts ──────────────────────────────────────────────
// What happens, drawn where it happens: the map's overlays, as GeoJSON.
//
// Pure: game state and the recent feed in, feature collections out, so the
// screen only has to hand them to the map (RealtimePlay.tsx). Every effect
// fades over a short life in SIMULATED seconds; the screen stretches the life
// at high speed so a flash is still visible to the eye.
//
//   fire      each volley as a line from firer to target — dashed for a miss,
//             solid when it struck — and a burst at the target: grey for a
//             round that bounced, orange for one that got through, a large
//             red burst for a vehicle knocked out
//   wrecks    where vehicles were knocked out, smoking for a minute, then a mark
//   spotting  a dotted line from a unit to an enemy it has just spotted, a pop
//             on the spotter; red when it found a shooter that was firing on it;
//             and a wedge along the bearing when it is fired on by a shooter it
//             cannot see
//   radio     a line from sender to each hearer, coloured by what was said, and
//             a pulse on the sender
//   plan      for the selected unit: its route and the steps of its orders
//             after it, numbered; the lines it may not cross; its trigger; and
//             its support links

import { bearingDeg, distanceM, type LatLng } from "../lib/board";
import type { Side } from "../lib/state";
import { offsetBy } from "./engine/geometry";
import type { RtLogEntry } from "./engine/runner";
import type { RtOrder, RtState } from "./engine/types";

/** GeoJSON, in the shapes used here (the global GeoJSON types are not everywhere this is built). */
export type Feature = {
  type: "Feature";
  properties: Record<string, unknown>;
  geometry:
    | { type: "Point"; coordinates: number[] }
    | { type: "LineString"; coordinates: number[][] }
    | { type: "Polygon"; coordinates: number[][][] };
};
export type Collection = { type: "FeatureCollection"; features: Feature[] };

const collection = (features: Feature[]): Collection => ({ type: "FeatureCollection", features });
const point = (at: LatLng, properties: Record<string, unknown> = {}): Feature => ({
  type: "Feature",
  properties,
  geometry: { type: "Point", coordinates: [at.lng, at.lat] },
});
const line = (points: LatLng[], properties: Record<string, unknown> = {}): Feature => ({
  type: "Feature",
  properties,
  geometry: { type: "LineString", coordinates: points.map((p) => [p.lng, p.lat]) },
});
const polygon = (ring: LatLng[], properties: Record<string, unknown> = {}): Feature => ({
  type: "Feature",
  properties,
  geometry: { type: "Polygon", coordinates: [[...ring, ring[0]].map((p) => [p.lng, p.lat])] },
});
/** A circle of `radiusM` around `at`, as a polygon. */
export function circle(at: LatLng, radiusM: number, properties: Record<string, unknown> = {}): Feature {
  return polygon(
    Array.from({ length: 36 }, (_, i) => offsetBy(at, i * 10, radiusM)),
    properties,
  );
}

/** How far into its life an effect is: 0 new, 1 gone. */
const age = (time: number, at: number, lifeS: number) => Math.max(0, Math.min(1, (time - at) / lifeS));

export interface OverlayView {
  /** Is this side's business shown (umpire view, or that side's eyes)? */
  own: (side: Side) => boolean;
  /** Stretches an effect's life at high speed, so it is still seen: base seconds → sim seconds. */
  life: (baseS: number) => number;
}

// ── Fire ───────────────────────────────────────────────────────────────────

/** Volleys: lines and impact bursts. */
export function fireOverlay(state: RtState, log: readonly RtLogEntry[], view: OverlayView): { lines: Collection; impacts: Collection } {
  const lines: Feature[] = [];
  const impacts: Feature[] = [];
  const lineLife = view.life(10);
  const burstLife = view.life(5);
  for (const entry of log) {
    if (entry.type !== "shot") continue;
    const shot = entry.shot;
    const a = age(state.time, entry.time, lineLife);
    if (a >= 1) continue;
    const firer = state.game.forceElements[shot.firerId];
    const target = state.game.forceElements[shot.targetId];
    if (!firer || !target) continue;
    // A shot is seen by its target's side as well as its own.
    if (!view.own(firer.side) && !view.own(target.side)) continue;
    const from = shot.firerAt ?? firer.position;
    const to = shot.targetAt ?? target.position;
    const outcome = shot.outcome ?? "miss";
    lines.push(line([from, to], { side: firer.side, outcome, hit: outcome === "miss" ? 0 : 1, opacity: 1 - a }));
    const b = age(state.time, entry.time, burstLife);
    if (b < 1 && outcome !== "miss") {
      impacts.push(point(to, { outcome, grow: b, opacity: 1 - b }));
    }
  }
  return { lines: collection(lines), impacts: collection(impacts) };
}

/** Where vehicles were knocked out, gathered from the feed as the game runs. */
export interface Wreck {
  at: LatLng;
  time: number;
  side: Side;
  count: number;
}

/** New wrecks in the feed since `after` (sim time). */
export function newWrecks(state: RtState, log: readonly RtLogEntry[], after: number): Wreck[] {
  const out: Wreck[] = [];
  for (const entry of log) {
    if (entry.type !== "shot" || entry.time <= after) continue;
    const shot = entry.shot;
    if (!shot.knockedOut) continue;
    const target = state.game.forceElements[shot.targetId];
    out.push({ at: shot.targetAt ?? target?.position ?? { lat: 0, lng: 0 }, time: entry.time, side: target?.side ?? "blue", count: shot.knockedOut });
  }
  return out;
}

/** Wrecks: smoking for a minute, then a dark mark where they lie. */
export function wreckOverlay(state: RtState, wrecks: readonly Wreck[]): Collection {
  return collection(
    wrecks.map((w) => {
      const smoke = age(state.time, w.time, 60);
      return point(w.at, { side: w.side, smoke: 1 - smoke, count: w.count });
    }),
  );
}

// ── Spotting and unseen fire ───────────────────────────────────────────────

/** Sightings (a dotted line and a pop), shooters found, and bearings to shooters not found. */
export function spottingOverlay(
  state: RtState,
  log: readonly RtLogEntry[],
  view: OverlayView,
): { lines: Collection; pops: Collection; wedges: Collection } {
  const lines: Feature[] = [];
  const pops: Feature[] = [];
  const wedges: Feature[] = [];
  const lifeS = view.life(6);
  const wedgeLife = view.life(10);
  for (const entry of log) {
    if (entry.type !== "event") continue;
    const e = entry.event;
    const self = state.game.forceElements[e.unitId];
    if (!self || !view.own(self.side)) continue;
    if (e.kind === "sighted" && !e.reported && e.about) {
      const a = age(state.time, entry.time, lifeS);
      const enemy = state.game.forceElements[e.about];
      if (a >= 1 || !enemy) continue;
      lines.push(line([self.position, enemy.position], { kind: "spotted", opacity: 1 - a }));
      pops.push(point(self.position, { kind: "spotted", grow: a, opacity: 1 - a }));
    }
    if ((e.kind === "underFire" || e.kind === "hit") && e.located === true && e.about) {
      const a = age(state.time, entry.time, lifeS);
      const shooter = state.game.forceElements[e.about];
      if (a >= 1 || !shooter) continue;
      lines.push(line([self.position, shooter.position], { kind: "located", opacity: 1 - a }));
    }
    if ((e.kind === "underFire" || e.kind === "hit") && e.located === false && e.bearingDeg != null) {
      const a = age(state.time, entry.time, wedgeLife);
      if (a >= 1) continue;
      // Fire from somewhere along here: a narrow wedge, a kilometre long.
      const reach = 1000;
      wedges.push(
        polygon(
          [self.position, offsetBy(self.position, e.bearingDeg - 8, reach), offsetBy(self.position, e.bearingDeg, reach * 1.05), offsetBy(self.position, e.bearingDeg + 8, reach)],
          { opacity: 1 - a },
        ),
      );
    }
  }
  return { lines: collection(lines), pops: collection(pops), wedges: collection(wedges) };
}

// ── Radio ──────────────────────────────────────────────────────────────────

/** Radio traffic: sender to each hearer, and a pulse on the sender. */
export function radioOverlay(state: RtState, log: readonly RtLogEntry[], view: OverlayView): { lines: Collection; pulses: Collection } {
  const lines: Feature[] = [];
  const pulses: Feature[] = [];
  const lifeS = view.life(4);
  for (const entry of log) {
    if (entry.type !== "message") continue;
    const a = age(state.time, entry.time, lifeS);
    if (a >= 1) continue;
    const m = entry.delivery.message;
    if (!view.own(m.side)) continue;
    const sender = state.game.forceElements[m.sender];
    if (!sender) continue;
    for (const id of entry.delivery.to) {
      const hearer = state.game.forceElements[id];
      if (!hearer) continue;
      // A gentle arc: through a point pushed off the straight line, so a
      // message and its reply do not sit on top of each other.
      const d = distanceM(sender.position, hearer.position);
      const mid = offsetBy(
        offsetBy(sender.position, bearingDeg(sender.position, hearer.position), d / 2),
        bearingDeg(sender.position, hearer.position) + 90,
        d * 0.12,
      );
      // Said to everyone (a report) is faint; said to one unit (a request, a reply) stands out.
      lines.push(line([sender.position, mid, hearer.position], { kind: m.kind, broadcast: m.to === "all" ? 1 : 0, opacity: 1 - a }));
    }
    pulses.push(point(sender.position, { kind: m.kind, grow: a, opacity: 1 - a }));
  }
  return { lines: collection(lines), pulses: collection(pulses) };
}

// ── The selected unit's plan ──────────────────────────────────────────────

/** Where an order ends, following any `then`, and the path to get there. */
function pathOf(order: RtOrder, from: LatLng): LatLng[] {
  const points: LatLng[] = [];
  let at = from;
  let current: RtOrder | undefined = order;
  for (let depth = 0; current && depth < 6; depth += 1) {
    if (current.kind === "move" || current.kind === "withdraw") {
      points.push(...(current.route ?? []), current.to);
      at = current.to;
    }
    current = "then" in current ? current.then : undefined;
  }
  return points.length ? [from, ...points] : [at];
}

export interface PlanOverlay {
  lines: Collection;
  zones: Collection;
  /** Numbered step ends, for labels drawn by the screen. */
  steps: { at: LatLng; label: string; current: boolean }[];
}

/** The selected unit's plan: route, later steps, limits, trigger and support links. */
export function planOverlay(state: RtState, id: string | null, view: OverlayView): PlanOverlay {
  const empty: PlanOverlay = { lines: collection([]), zones: collection([]), steps: [] };
  if (!id) return empty;
  const fe = state.game.forceElements[id];
  const unit = state.units[id];
  if (!fe || !unit || fe.combatStrength <= 0 || !view.own(fe.side)) return empty;
  const lines: Feature[] = [];
  const zones: Feature[] = [];
  const steps: PlanOverlay["steps"] = [];

  // What it is doing now.
  const now = pathOf(unit.order, fe.position);
  if (now.length > 1) lines.push(line(now, { kind: "current" }));
  let end = now[now.length - 1];

  // The steps of its orders after this one.
  const orders = unit.orders;
  if (orders && !orders.done) {
    const onStep = unit.order.phase === orders.phase;
    if (onStep) steps.push({ at: end, label: `${orders.phase + 1}`, current: true });
    for (let i = onStep ? orders.phase + 1 : orders.phase; i < orders.phases.length; i += 1) {
      const step = orders.phases[i].order;
      if (step.kind === "move" || step.kind === "withdraw") {
        lines.push(line([end, step.to], { kind: "later" }));
        end = step.to;
      }
      steps.push({ at: end, label: `${i + 1}${step.kind === "move" || step.kind === "withdraw" ? "" : ` ${step.kind}`}`, current: false });
    }
    // Lines it may not cross.
    for (const b of orders.boundaries) {
      const along = b.keep === "north" || b.keep === "south" ? 90 : 0;
      lines.push(line([offsetBy(b.at, along, 15_000), offsetBy(b.at, along + 180, 15_000)], { kind: "boundary" }));
    }
    // The unit it supports.
    const supported = orders.supports ? state.game.forceElements[orders.supports] : undefined;
    if (supported && supported.combatStrength > 0) lines.push(line([fe.position, supported.position], { kind: "support" }));
  }
  // Units that support it.
  for (const [otherId, other] of Object.entries(state.units)) {
    const f = state.game.forceElements[otherId];
    if (other.orders?.supports === id && f && f.combatStrength > 0) lines.push(line([f.position, fe.position], { kind: "support" }));
  }

  // Its trigger, if it is waiting for one.
  const order = unit.order;
  if (order.kind === "wait") {
    const target = state.game.forceElements[order.targetId];
    const t = order.trigger;
    const met = order.met === true;
    if (t.kind === "range") zones.push(circle(fe.position, t.withinM, { kind: "trigger", met }));
    else if (t.kind === "hitChance") zones.push(circle(fe.position, t.aboutM, { kind: "trigger", met }));
    else if (t.kind === "reaches") zones.push(circle(t.at, t.withinM, { kind: "trigger", met }));
    if (target && target.combatStrength > 0) lines.push(line([fe.position, target.position], { kind: "watching" }));
  }
  // Steps that end in the same place share one label: "1 · 2 overwatch".
  const merged: PlanOverlay["steps"] = [];
  for (const step of steps) {
    const same = merged.find((m) => distanceM(m.at, step.at) < 30);
    if (same) {
      same.label = `${same.label} · ${step.label}`;
      same.current = same.current || step.current;
    } else merged.push({ ...step });
  }
  return { lines: collection(lines), zones: collection(zones), steps: merged };
}
