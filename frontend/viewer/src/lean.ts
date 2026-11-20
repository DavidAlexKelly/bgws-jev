// Where the satellite was: the lean of buildings in satellite imagery.
//
// Imagery is taken a little off straight down, so a building's roof shows in
// the photograph shifted from its footprint, away from the satellite, by its
// height times the tangent of the view's off-nadir angle. That is one vector
// for a whole image, `k`: the roof's shift (east, south) per metre of height.
// The tile service does not say where the satellite was, so `k` is measured
// from the imagery: the shift at which the outlines of the tall buildings'
// roofs fall on the strongest edges in the picture.
//
// The imagery is read as edges: per tile, how sharply the colour changes at
// each pixel (capped, so an outline edged all the way round beats one with a
// few very strong edges, a shadow's, on one side), and the same spread to
// 1–6 pixels, so a coarse search does not step over an edge that lies
// between its candidates. Positions are Web Mercator (0–1, y south).

/** Edge strength at a mercator point, the strongest within `RADII[level]`
 * pixels (0–`STEP_CAP`; NaN where the imagery is not loaded). */
export type EdgeMap = (mx: number, my: number, level: number) => number;

export interface LeanBuilding {
  /** Outer ring, mercator, without the closing point. */
  ring: [number, number][];
  /** Height, m. */
  h: number;
  /** Mercator units per metre at the building. */
  pm: number;
}

/** A colour step counts up to this (mean over the channels, 0–255). */
export const STEP_CAP = 40;
/** The spreads kept per tile, pixels. */
export const RADII = [0, 1, 2, 4, 6];

/** Edge strength per pixel of an RGBA image: the larger colour step across
 * it, left–right or up–down (mean over the channels), capped. */
export function edgeImage(rgba: ArrayLike<number>, w: number, h: number): Uint8Array {
  const out = new Uint8Array(w * h);
  const at = (x: number, y: number) => (Math.min(h - 1, Math.max(0, y)) * w + Math.min(w - 1, Math.max(0, x))) * 4;
  const diff = (a: number, b: number) => (Math.abs(rgba[a] - rgba[b]) + Math.abs(rgba[a + 1] - rgba[b + 1]) + Math.abs(rgba[a + 2] - rgba[b + 2])) / 3;
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const v = Math.max(diff(at(x + 1, y), at(x - 1, y)), diff(at(x, y + 1), at(x, y - 1)));
      out[y * w + x] = Math.min(STEP_CAP, v);
    }
  }
  return out;
}

/** The strongest value within `r` pixels (a square), in two passes. */
export function spread(img: Uint8Array, w: number, h: number, r: number): Uint8Array {
  if (r <= 0) return img;
  const tmp = new Uint8Array(w * h);
  const out = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let m = 0;
      for (let o = Math.max(0, x - r); o <= Math.min(w - 1, x + r); o++) m = Math.max(m, img[y * w + o]);
      tmp[y * w + x] = m;
    }
  }
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      let m = 0;
      for (let o = Math.max(0, y - r); o <= Math.min(h - 1, y + r); o++) m = Math.max(m, tmp[o * w + x]);
      out[y * w + x] = m;
    }
  }
  return out;
}

/** The spread level that covers `tol` (mercator) at a pixel of `px`. */
function levelFor(tol: number, px: number): number {
  const want = tol / px;
  for (let i = 0; i < RADII.length; i++) if (RADII[i] >= want) return i;
  return RADII.length - 1;
}

/** How sharply the ring, shifted by (dx, dy) mercator, lies on edges in the
 * imagery: the mean edge strength along it, sampled every 2 pixels (`px`: a
 * pixel's size, mercator), each the strongest within `tol`. NaN when most of
 * it is not loaded. */
export function edgeScore(ring: [number, number][], dx: number, dy: number, edge: EdgeMap, px: number, tol = 0): number {
  const level = levelFor(tol, px);
  const every = 2 * px;
  let sum = 0;
  let n = 0;
  let all = 0;
  for (let i = 0; i < ring.length; i++) {
    const a = ring[i];
    const b = ring[(i + 1) % ring.length];
    const ex = b[0] - a[0];
    const ey = b[1] - a[1];
    const len = Math.hypot(ex, ey);
    if (len < px) continue;
    const steps = Math.max(1, Math.floor(len / every));
    for (let s = 0; s < steps; s++) {
      const t = (s + 0.5) / steps;
      all++;
      const v = edge(a[0] + ex * t + dx, a[1] + ey * t + dy, level);
      if (Number.isFinite(v)) {
        sum += v;
        n++;
      }
    }
  }
  return n >= 8 && n >= all * 0.6 ? sum / n : NaN;
}

/** The weighted mean edge score of the buildings for a lean `k`, each
 * outline tolerant of `kTol` of lean. */
function score(bs: LeanBuilding[], k: [number, number], edge: EdgeMap, px: number, kTol = 0): number {
  let s = 0;
  let w = 0;
  for (const b of bs) {
    const m = b.h * b.pm;
    const e = edgeScore(b.ring, k[0] * m, k[1] * m, edge, px, kTol * m);
    if (!Number.isFinite(e)) continue;
    const wb = Math.min(b.h, 40);
    s += wb * e;
    w += wb;
  }
  return w > 0 ? s / w : NaN;
}

export interface Lean {
  /** Roof shift (east, south) per metre of height. */
  k: [number, number];
  /** How much better the roofs' outlines fit with it than with none (1: no better). */
  gain: number;
  /** Buildings it was measured on. */
  used: number;
}

/** Largest lean looked for: about 39° off nadir. */
export const MAX_K = 0.8;
/** A lean is taken only if the outlines fit this much better than with none. */
export const MIN_GAIN = 1.08;

/** The lean that best lays the buildings' roof outlines on the imagery's edges. */
export function estimateLean(bs: LeanBuilding[], edge: EdgeMap, px: number): Lean {
  const search = leanSearch(bs, edge, px);
  let r = search.next();
  while (!r.done) r = search.next();
  return r.value;
}

/** As `estimateLean`, a step at a time: it pauses after each row of
 * candidates, so a caller can spread the work over several frames. */
export function* leanSearch(bs: LeanBuilding[], edge: EdgeMap, px: number): Generator<void, Lean> {
  const none: Lean = { k: [0, 0], gain: 1, used: 0 };
  const base = score(bs, [0, 0], edge, px);
  if (!Number.isFinite(base) || base <= 0) return none;
  let best: [number, number] = [0, 0];
  // Coarse, then finer round the best: each pass tolerant of half its step,
  // so it cannot step over the right lean, the last exact.
  for (const [inc, span, tol] of [
    [0.1, MAX_K, 0.05],
    [0.03, 0.1, 0.015],
    [0.01, 0.03, 0],
  ]) {
    const centre = best;
    let bestS = -Infinity;
    let next = best;
    const steps = Math.round(span / inc);
    for (let j = -steps; j <= steps; j++) {
      for (let i = -steps; i <= steps; i++) {
        const k: [number, number] = [centre[0] + i * inc, centre[1] + j * inc];
        if (Math.hypot(k[0], k[1]) > MAX_K) continue;
        const s = score(bs, k, edge, px, tol);
        if (s > bestS) {
          bestS = s;
          next = k;
        }
      }
      yield;
    }
    best = next;
  }
  const gain = score(bs, best, edge, px) / base;
  const used = bs.filter((b) => Number.isFinite(edgeScore(b.ring, 0, 0, edge, px))).length;
  return gain >= MIN_GAIN ? { k: best, gain, used } : { ...none, gain, used };
}
