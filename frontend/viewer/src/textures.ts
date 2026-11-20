// Ground textures for the quality graphics mode: small tiles drawn once on a
// canvas, in the map's dark palette, and registered with MapLibre as images
// for `fill-pattern` (land cover, water). Drawn here rather than shipped as
// files so the viewer stays one bundle with nothing to fetch, and so they
// match the palette.
//
// Every tile is seamless: anything drawn near an edge is drawn again on the
// far side. A seeded random keeps them the same every run.

import type maplibregl from "maplibre-gl";

/** Tile edge in CSS pixels; drawn at 2× for sharp edges on fine screens. */
const SIZE = 64;
const RATIO = 2;

/** A small seeded random (mulberry32): the same tiles every time. */
function rng(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

type Draw = (g: CanvasRenderingContext2D, r: () => number) => void;

/** Draw `f` at (x, y) and again wherever it would cross an edge. */
function wrap(x: number, y: number, reach: number, f: (x: number, y: number) => void) {
  for (const dx of [-SIZE, 0, SIZE]) {
    for (const dy of [-SIZE, 0, SIZE]) {
      const px = x + dx;
      const py = y + dy;
      if (px > -reach && px < SIZE + reach && py > -reach && py < SIZE + reach) f(px, py);
    }
  }
}

function speckle(g: CanvasRenderingContext2D, r: () => number, n: number, colours: string[], size: [number, number]) {
  for (let i = 0; i < n; i++) {
    const x = r() * SIZE;
    const y = r() * SIZE;
    const s = size[0] + r() * (size[1] - size[0]);
    g.fillStyle = colours[Math.floor(r() * colours.length)];
    wrap(x, y, s, (px, py) => g.fillRect(px, py, s, s));
  }
}

const PATTERNS: Record<string, { base: string; draw: Draw }> = {
  // Open ground: a soft, uneven tone.
  "tex-ground": {
    base: "#141c2b",
    draw: (g, r) => speckle(g, r, 260, ["#18213a", "#111827", "#1a2233"], [0.6, 1.6]),
  },
  // Woods from above: overlapping crowns, lit from the north-west.
  "tex-wood": {
    base: "#17291e",
    draw: (g, r) => {
      for (let i = 0; i < 42; i++) {
        const x = r() * SIZE;
        const y = r() * SIZE;
        const rad = 3.5 + r() * 4.5;
        const shade = Math.floor(r() * 3);
        wrap(x, y, rad + 2, (px, py) => {
          g.fillStyle = "rgba(4, 10, 6, 0.55)";
          g.beginPath();
          g.arc(px + 1.4, py + 1.6, rad, 0, Math.PI * 2);
          g.fill();
          g.fillStyle = ["#1e3a28", "#21422c", "#1a3324"][shade];
          g.beginPath();
          g.arc(px, py, rad, 0, Math.PI * 2);
          g.fill();
          g.fillStyle = "rgba(120, 170, 120, 0.12)";
          g.beginPath();
          g.arc(px - rad * 0.3, py - rad * 0.3, rad * 0.55, 0, Math.PI * 2);
          g.fill();
        });
      }
    },
  },
  // Fields: furrows on a slant, a little uneven.
  "tex-farm": {
    base: "#222a1b",
    draw: (g, r) => {
      g.strokeStyle = "rgba(60, 70, 40, 0.55)";
      g.lineWidth = 1;
      for (let k = -SIZE; k < SIZE * 2; k += 5) {
        g.beginPath();
        g.moveTo(k, 0);
        g.lineTo(k + SIZE, SIZE);
        g.stroke();
      }
      speckle(g, r, 60, ["#262f1e", "#1d2417"], [0.6, 1.4]);
    },
  },
  // Grass and parks: short strokes.
  "tex-grass": {
    base: "#1a271e",
    draw: (g, r) => {
      g.lineWidth = 0.8;
      for (let i = 0; i < 140; i++) {
        const x = r() * SIZE;
        const y = r() * SIZE;
        const h = 1.5 + r() * 2.5;
        g.strokeStyle = r() < 0.5 ? "rgba(70, 110, 70, 0.35)" : "rgba(20, 35, 24, 0.6)";
        wrap(x, y, 4, (px, py) => {
          g.beginPath();
          g.moveTo(px, py);
          g.lineTo(px + (r() - 0.5) * 1.5, py - h);
          g.stroke();
        });
      }
    },
  },
  // Scrub and heath: low clumps.
  "tex-scrub": {
    base: "#1d281d",
    draw: (g, r) => {
      for (let i = 0; i < 70; i++) {
        const x = r() * SIZE;
        const y = r() * SIZE;
        const rad = 1 + r() * 2;
        g.fillStyle = r() < 0.5 ? "#253425" : "#18221a";
        wrap(x, y, rad, (px, py) => {
          g.beginPath();
          g.arc(px, py, rad, 0, Math.PI * 2);
          g.fill();
        });
      }
    },
  },
  // Wetland: reed dashes over standing water.
  "tex-wet": {
    base: "#152628",
    draw: (g, r) => {
      speckle(g, r, 40, ["#12324a"], [1.5, 3]);
      g.strokeStyle = "rgba(80, 120, 100, 0.45)";
      g.lineWidth = 0.8;
      for (let i = 0; i < 60; i++) {
        const x = r() * SIZE;
        const y = r() * SIZE;
        wrap(x, y, 4, (px, py) => {
          g.beginPath();
          g.moveTo(px - 2, py);
          g.lineTo(px + 2, py);
          g.stroke();
        });
      }
    },
  },
  // Built-up ground: a faint street grid.
  "tex-urban": {
    base: "#1c2030",
    draw: (g, r) => {
      g.strokeStyle = "rgba(55, 62, 86, 0.6)";
      g.lineWidth = 1;
      for (let k = 0; k <= SIZE; k += 16) {
        g.beginPath();
        g.moveTo(k + 0.5, 0);
        g.lineTo(k + 0.5, SIZE);
        g.moveTo(0, k + 0.5);
        g.lineTo(SIZE, k + 0.5);
        g.stroke();
      }
      speckle(g, r, 50, ["#20253a"], [1, 2.5]);
    },
  },
  // Industrial and hard standing: a coarser grid.
  "tex-industrial": {
    base: "#1f2132",
    draw: (g, r) => {
      g.strokeStyle = "rgba(60, 64, 88, 0.55)";
      g.lineWidth = 1;
      for (let k = 0; k <= SIZE; k += 32) {
        g.beginPath();
        g.moveTo(k + 0.5, 0);
        g.lineTo(k + 0.5, SIZE);
        g.moveTo(0, k + 0.5);
        g.lineTo(SIZE, k + 0.5);
        g.stroke();
      }
      speckle(g, r, 80, ["#24273a", "#1a1c2b"], [0.8, 2]);
    },
  },
  // Sand, rock and quarries: fine stipple.
  "tex-sand": {
    base: "#29271f",
    draw: (g, r) => speckle(g, r, 320, ["#332f24", "#211f18", "#2e2b21"], [0.5, 1.2]),
  },
  // Water: faint ripples.
  "tex-water": {
    base: "#0d2540",
    draw: (g, r) => {
      g.strokeStyle = "rgba(60, 110, 160, 0.22)";
      g.lineWidth = 0.8;
      for (let i = 0; i < 26; i++) {
        const x = r() * SIZE;
        const y = r() * SIZE;
        const w = 3 + r() * 5;
        wrap(x, y, w + 1, (px, py) => {
          g.beginPath();
          g.arc(px, py + w, w, Math.PI * 1.25, Math.PI * 1.75);
          g.stroke();
        });
      }
    },
  },
};

export const TEXTURE_NAMES = Object.keys(PATTERNS);

/** Draw one tile as RGBA pixels (at 2×). */
function draw(name: string): ImageData | null {
  const p = PATTERNS[name];
  if (!p || typeof document === "undefined") return null;
  const c = document.createElement("canvas");
  c.width = SIZE * RATIO;
  c.height = SIZE * RATIO;
  const g = c.getContext("2d");
  if (!g) return null;
  g.scale(RATIO, RATIO);
  g.fillStyle = p.base;
  g.fillRect(0, 0, SIZE, SIZE);
  let seed = 0;
  for (const ch of name) seed = (seed * 31 + ch.charCodeAt(0)) >>> 0;
  p.draw(g, rng(seed));
  return g.getImageData(0, 0, c.width, c.height);
}

const cache = new Map<string, ImageData | null>();

/** Register every texture with a map (once per map: images survive style edits). */
export function addTextures(m: maplibregl.Map) {
  for (const name of TEXTURE_NAMES) {
    if (m.hasImage(name)) continue;
    if (!cache.has(name)) cache.set(name, draw(name));
    const img = cache.get(name);
    if (img) m.addImage(name, img, { pixelRatio: RATIO });
  }
}
