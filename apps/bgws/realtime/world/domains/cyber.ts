// ── world/domains/cyber.ts ───────────────────────────────────────────────
// Cyber (MULTI_DOMAIN_PLAN.md W9): accesses prepared before the game, used
// inside a window the planner authorised. Sandworm's Kyivstar access took
// months to build and its effect lasted two to three days [ref R5.5, P21].
//
// When an access is ready and its window open, the side's cyber cell is asked
// (cyberWindowOpen): execute now, hold for a better moment, or abandon it if
// it may be found. Executed, it works with its probability of effect, for its
// duration, and may be detected — burning it, and shortening the outage as
// the victim remediates. Effects act on modelled systems:
//
//   bearer       that bearer is out for the victim side (net.ts)
//   logistics    the victim's logistics cell stops its cycle: resupply waits
//   airDefence   the victim's air defence loses its shared picture: each
//                launcher engages only what its own radar holds
//   public       alerts and morale: noted in the feed (effects come later)
//
// Executing an access is escalatory: an access against public networks is
// refused while the side's guidance limits it to conventional means.

import { opposing, type Side } from "../../../lib/state";
import { registerHandlers, registerRules, prefer } from "../actions";
import type { WorldCtx } from "../context";
import { wfig } from "../figures";
import { raiseQuestion } from "../questions";
import { registerSystem } from "../step";
import type { CyberAccess } from "../types";

/** Who decides for a side's cyber operations: its cyber cell, or failing that its senior HQ. */
function cellOf(c: WorldCtx, side: Side): string | undefined {
  const cell = Object.values(c.world.entities).find((e) => e.owner === side && e.archetype === "generic/cyberCell" && e.status === "active");
  if (cell) return cell.id;
  const top = Object.values(c.world.hqs).find((h) => h.side === side && !h.parent && !h.down);
  return top?.postId;
}

function escalationAllows(c: WorldCtx, a: CyberAccess): boolean {
  if (a.target !== "public") return true;
  const limits = Object.values(c.world.hqs).filter((h) => h.side === a.side && h.guidance).map((h) => h.guidance!.roe.escalationLimit);
  return !limits.includes("conventional");
}

function setAccess(c: WorldCtx, id: string, patch: Partial<CyberAccess>): void {
  c.world.cyber = c.world.cyber.map((a) => (a.id === id ? { ...a, ...patch } : a));
}

/** The outage key an access's effect sets, for the side that suffers it. */
export function cyberKey(a: CyberAccess): string {
  const victim = opposing(a.side);
  switch (a.target) {
    case "bearer":
      return `${a.system}:${victim}`;
    case "logistics":
      return `logistics:${victim}`;
    case "airDefence":
      return `adNet:${victim}`;
    case "public":
      return `public:${victim}`;
  }
}

function windows(c: WorldCtx): void {
  if (c.time % 60 !== 0) return;
  for (const a of c.world.cyber) {
    if (a.executedAt != null || a.detected || !a.window) continue;
    if (c.time < a.readiness || c.time < a.window.from || c.time > a.window.until) continue;
    if (!escalationAllows(c, a)) continue;
    if (c.time - c.last(`cyberAsk:${a.id}`) < wfig("world/domains/cyber.ASK_S")) continue;
    const cell = cellOf(c, a.side);
    if (!cell) continue;
    c.mark(`cyberAsk:${a.id}`);
    raiseQuestion(c, { agentId: cell, side: a.side, trigger: "cyberWindowOpen", about: { accessId: a.id, target: a.target, system: a.system, pEffect: a.pEffect, detectionRisk: a.detectionRisk, windowLeftS: a.window.until - c.time }, quality: 6 });
  }
}

function execute(c: WorldCtx, a: CyberAccess): void {
  const worked = c.chance(a.pEffect);
  const detected = c.chance(a.detectionRisk);
  const length = a.durationS * (detected ? wfig("world/domains/cyber.REMEDIATED_SHARE") : 1);
  setAccess(c, a.id, { executedAt: c.time, ...(worked ? { effectUntil: c.time + length } : {}), detected });
  const victim = opposing(a.side);
  if (worked) {
    const key = cyberKey(a);
    c.world.outages[key] = Math.max(c.world.outages[key] ?? -Infinity, c.time + length);
  }
  c.log({ side: a.side, domain: "cyber", kind: worked ? "effect" : "failed", text: worked ? `cyber access ${a.id} takes down ${victim}'s ${a.target === "bearer" ? a.system : a.target} for ${Math.round(length / 3600)} h` : `cyber access ${a.id} fails to take effect`, ids: [a.id] });
  if (detected) c.log({ side: victim, domain: "cyber", kind: "detected", text: `${victim} finds an intrusion in its ${a.target === "bearer" ? a.system : a.target} and starts to remediate`, ids: [a.id] });
}

const accessOf = (c: WorldCtx, q: { about: Record<string, unknown> }) => c.world.cyber.find((a) => a.id === q.about.accessId);

registerHandlers({
  "cyber.executeNow": {
    available: (c, q) => accessOf(c, q)?.executedAt == null,
    apply: (c, q) => {
      const a = accessOf(c, q);
      if (a) execute(c, a);
    },
  },
  "cyber.holdForMoment": { apply: () => {} },
  "cyber.abandon": {
    apply(c, q) {
      const a = accessOf(c, q);
      if (a?.window) setAccess(c, a.id, { window: { ...a.window, until: c.time } });
    },
  },
});

registerRules({
  // Execute when the fight needs it: the window closing, or the enemy in contact; hold a risky access otherwise.
  cyberWindowOpen: (c, q, options) => {
    const closing = (q.about.windowLeftS as number) < wfig("world/domains/cyber.ASK_S") * 2;
    const contact = Object.keys(c.game.forceElements).some((id) => c.land.units[id] && Object.keys(c.land.units[id].attackers).length > 0);
    if ((q.about.detectionRisk as number) > 0.6 && (q.about.pEffect as number) < 0.3) return prefer(options, "cyber.abandon");
    return closing || contact || (q.about.detectionRisk as number) < 0.5 ? prefer(options, "cyber.executeNow") : prefer(options, "cyber.holdForMoment", "cyber.executeNow");
  },
});

registerSystem({
  name: "cyber",
  order: 64,
  when: (c) => c.simulated("cyber"),
  run: windows,
});
