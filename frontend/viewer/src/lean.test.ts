// The lean of buildings measured from synthetic imagery: a textured ground,
// buildings whose roofs are photographed shifted by a known lean, the walls
// that show on the side facing the satellite, and shadows cast another way
// (edges that do not belong to any roof). Run: npm test.

import assert from "node:assert/strict";
import test from "node:test";
import { edgeImage, edgeScore, estimateLean, RADII, refineScale, spread, type EdgeMap, type LeanBuilding } from "./lean.ts";

/** Seeded random (mulberry32). */
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

const PX = 0.35; // a pixel, m (about zoom 18 at 55°N)
const SIZE = 640; // the town's side, m

interface Box {
  x: number;
  y: number;
  w: number;
  d: number;
  h: number;
  roof: [number, number, number];
}

/** A town of boxes (metres; y south), photographed with lean `k` and the sun
 * casting shadows along `sun`, read as the viewer reads imagery. */
function town(seed: number, k: [number, number], sun: [number, number]) {
  const r = rng(seed);
  const boxes: Box[] = [];
  for (let gy = 0; gy < 5; gy++) {
    for (let gx = 0; gx < 5; gx++) {
      if (r() < 0.2) continue;
      const w = 15 + r() * 45;
      const d = 12 + r() * 20;
      // Roofs in colour, some about as bright as the walls (so only colour tells them apart).
      const roof: [number, number, number] = r() < 0.5 ? [200, 90, 60] : [90, 150, 170];
      boxes.push({ x: 40 + gx * 110 + r() * 20, y: 40 + gy * 110 + r() * 20, w, d, h: 9 + r() * 36, roof });
    }
  }
  const n = Math.round(SIZE / PX);
  const img = new Uint8ClampedArray(n * n * 4);
  // Ground: a fine noisy texture, so edges are everywhere and not only at roofs.
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const mx = x * PX;
      const my = y * PX;
      const v = 70 + 25 * Math.sin(mx * 1.7 + Math.cos(my * 0.9) * 3) * Math.cos(my * 1.3 + mx * 0.2);
      const i = (y * n + x) * 4;
      img[i] = v;
      img[i + 1] = v + 8;
      img[i + 2] = v - 6;
      img[i + 3] = 255;
    }
  }
  const fill = (b: Box, ox: number, oy: number, c: [number, number, number]) => {
    const x0 = Math.max(0, Math.floor((b.x + ox) / PX));
    const x1 = Math.min(n - 1, Math.floor((b.x + ox + b.w) / PX));
    const y0 = Math.max(0, Math.floor((b.y + oy) / PX));
    const y1 = Math.min(n - 1, Math.floor((b.y + oy + b.d) / PX));
    for (let y = y0; y <= y1; y++) {
      for (let x = x0; x <= x1; x++) {
        const i = (y * n + x) * 4;
        img[i] = c[0];
        img[i + 1] = c[1];
        img[i + 2] = c[2];
      }
    }
  };
  // Shadows cast along the sun, as long as the building is tall; walls toward the satellite; roofs where photographed.
  for (const b of boxes) for (let t = 0.1; t <= 1.0001; t += 0.05) fill(b, sun[0] * b.h * t, sun[1] * b.h * t, [22, 24, 30]);
  for (const b of boxes) for (let t = 0; t < 1; t += 0.05) fill(b, k[0] * b.h * t, k[1] * b.h * t, [125, 120, 110]);
  for (const b of boxes) fill(b, k[0] * b.h, k[1] * b.h, b.roof);
  const levels = RADII.map((rad) => spread(edgeImage(img, n, n), n, n, rad));
  const edge: EdgeMap = (mx, my, level) => {
    const x = Math.floor(mx / PX);
    const y = Math.floor(my / PX);
    return x < 0 || y < 0 || x >= n || y >= n ? NaN : levels[level][y * n + x];
  };
  const ring = (b: Box): [number, number][] => [
    [b.x, b.y],
    [b.x + b.w, b.y],
    [b.x + b.w, b.y + b.d],
    [b.x, b.y + b.d],
  ];
  return { boxes, edge, ring };
}

for (const [name, k, sun] of [
  ["leaning north-west, sun from the south-east", [-0.27, -0.33], [0.5, -0.45]],
  ["leaning east, sun from the south-west", [0.43, 0.06], [-0.6, -0.5]],
  ["steeply off nadir, south", [0.04, 0.62], [0.7, 0.2]],
] as [string, [number, number], [number, number]][]) {
  test(`the lean is found: ${name}`, () => {
    const t = town(7, k, sun);
    const bs: LeanBuilding[] = t.boxes.map((b) => ({ ring: t.ring(b), h: b.h, pm: 1 }));
    const lean = estimateLean(bs, t.edge, PX);
    assert.ok(Math.hypot(lean.k[0] - k[0], lean.k[1] - k[1]) < 0.02, `found ${lean.k.map((v) => v.toFixed(3))} for ${k}`);
    assert.ok(lean.gain > 1.3, `gain ${lean.gain.toFixed(2)}`);
    assert.ok(lean.used >= 15);
  });
}

test("straight down: no lean is found", () => {
  const t = town(11, [0, 0], [0.5, -0.4]);
  const bs: LeanBuilding[] = t.boxes.map((b) => ({ ring: t.ring(b), h: b.h, pm: 1 }));
  const lean = estimateLean(bs, t.edge, PX);
  assert.ok(Math.hypot(lean.k[0], lean.k[1]) < 0.02, `found ${lean.k}`);
});

test("a building whose height is wrong takes its own share of the lean", () => {
  const k: [number, number] = [0.3, -0.2];
  const t = town(5, k, [-0.5, 0.4]);
  const b = t.boxes.reduce((a, c) => (c.h > a.h ? c : a));
  // Mapped at 60% of its real height: its roof shows 1/0.6 as far off as that suggests.
  const s = refineScale({ ring: t.ring(b), h: b.h * 0.6, pm: 1 }, k, t.edge, PX);
  assert.ok(Math.abs(s - 1 / 0.6) < 0.1, `share ${s}`);
  // Right as it is: left alone.
  assert.equal(refineScale({ ring: t.ring(b), h: b.h, pm: 1 }, k, t.edge, PX), 1);
});

test("an outline off the loaded imagery scores nothing", () => {
  const ring: [number, number][] = [
    [0, 0],
    [20, 0],
    [20, 20],
    [0, 20],
  ];
  assert.ok(Number.isNaN(edgeScore(ring, 0, 0, () => NaN, PX)));
});

test("edges: a colour step registers, flat colour does not, and spreading widens it", () => {
  const w = 8;
  const img = new Uint8ClampedArray(w * w * 4);
  for (let y = 0; y < w; y++) for (let x = 0; x < w; x++) img.set(x < 4 ? [200, 90, 60, 255] : [110, 120, 125, 255], (y * w + x) * 4);
  const e = edgeImage(img, w, w);
  assert.equal(e[2 * w + 0], 0);
  assert.ok(e[2 * w + 3] > 30 && e[2 * w + 4] > 30, "both sides of the step");
  assert.equal(spread(e, w, w, 2)[2 * w + 1], e[2 * w + 3]);
});
