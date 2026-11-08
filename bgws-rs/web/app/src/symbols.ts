// Unit symbols (MIL-STD-2525C) drawn with milsymbol, cached as images for
// deck.gl's IconLayer. The catalogue gives every platform a friendly and a
// hostile code; drones in flight and enemy tracks get generic ones.

import { ms, std2525c } from "milsymbol-esm";
import type { FrameEntity, Side } from "./protocol";

export interface Icon {
  id: string;
  url: string;
  width: number;
  height: number;
  anchorX: number;
  anchorY: number;
}

// The catalogue's codes are 2525C letter codes: that set alone is registered.
ms.addIcons(std2525c);

const cache = new Map<string, Icon>();

function draw(sidc: string, opts: Record<string, unknown>, key: string): Icon {
  const hit = cache.get(key);
  if (hit) return hit;
  const sym = new ms.Symbol(sidc, { size: 26, outlineWidth: 3, outlineColor: "rgba(6,13,24,0.85)", ...opts });
  const canvas = sym.asCanvas(2);
  const anchor = sym.getAnchor();
  const icon = { id: key, url: canvas.toDataURL(), width: canvas.width, height: canvas.height, anchorX: anchor.x * 2, anchorY: anchor.y * 2 };
  cache.set(key, icon);
  return icon;
}

const affiliation = (side: Side) => (side === "red" ? "H" : side === "blue" ? "F" : "N");

/** A unit's symbol: greyed when destroyed, fainter when broken. */
export function unitIcon(e: FrameEntity): Icon {
  let sidc = e.sidc;
  if (!sidc) sidc = e.domain === "drone" ? `S${affiliation(e.side)}APMFQ` : `S${affiliation(e.side)}GPU`;
  const gone = e.status === "destroyed";
  const opts: Record<string, unknown> = gone ? { monoColor: "#6b6f78", fillOpacity: 0.5 } : e.status === "broken" ? { fillOpacity: 0.45 } : {};
  if (e.domain === "drone") opts.size = 16;
  return draw(sidc, opts, `${sidc}|${gone ? "x" : e.status === "broken" ? "b" : ""}|${e.domain === "drone" ? "d" : ""}`);
}

/** An enemy track as its holder believes it: the opposing side's generic symbol, by what it is thought to be. */
export function trackIcon(owner: Side, cls: string): Icon {
  const them = affiliation(owner === "blue" ? "red" : "blue");
  const c = cls.toLowerCase();
  const fn = c.includes("mbt") || c.includes("tank") ? "UCA" : c.includes("ifv") || c.includes("apc") ? "UCI" : c.includes("art") || c.includes("sph") ? "UCF" : c.includes("air") || c.includes("heli") ? "" : "U";
  const sidc = c.includes("air") || c.includes("heli") ? `S${them}APM` : `S${them}GP${fn}`;
  return draw(sidc, { size: 22, fillOpacity: 0.55 }, `track|${sidc}`);
}
