// ── realtime/world/jamming.ts ────────────────────────────────────────────
// Jamming of the land engine's own radio net (W6): a hearer in a jammer's
// footprint loses transmissions, whichever side the jammer is on — jamming
// is indiscriminate, and Russia's 2022 jamming disrupted its own comms
// [ref R2]. Only built when the game has a jammer on.

import type { GameState } from "../../lib/state";
import type { Rng } from "../../rules/dice";
import { distanceM } from "./geo";
import { wfig } from "./figures";
import type { WorldState } from "./types";

export function landRadioJamming(world: WorldState, game: () => GameState, rng: Rng): (senderId: string, hearerId: string) => boolean {
  const jammers = world.jammers.filter((j) => j.on && j.bands.includes("vhf"));
  return (senderId, hearerId) => {
    const g = game();
    const hearer = g.forceElements[hearerId]?.position;
    const sender = g.forceElements[senderId]?.position;
    if (!hearer || !sender || !jammers.length) return false;
    let p = 0;
    for (const j of jammers) {
      const radius = j.radii?.vhf ?? j.radiusM;
      const d = distanceM(j.at, hearer);
      if (d < radius) p = Math.max(p, (1 - d / radius) * (1 - wfig("world/net.VHF_HOP_RESIST")) + 0.1);
    }
    return p > 0 && rng.int(1_000_000) < p * 1_000_000;
  };
}
