// Trees: in 3D, the woods stand as trees, a trunk and a crown each, instead
// of a block of canopy.
//
// A MapLibre custom layer, so it shares the map's depth buffer: trees stand
// on the terrain, behind and in front of buildings as they should. The trees
// are planted in the woods MapLibre has loaded round the view, on a grid
// about 7.5 m apart, each nudged off it, with gaps here and there. The grid
// is fixed to the map, so a tree stands in the same place whenever the woods
// are planted again. What each tree is like (broadleaf or conifer, height,
// crown, shade, turn) comes from its place on the grid.
//
// All the trees are drawn in a few calls (instancing): a six-sided trunk, and
// a crown, a rounded dome for broadleaf trees and two stacked cones for
// conifers. The crowns are cut through with a pattern of small holes, so
// light and the inside of the crown show through as between leaves; the
// inside is shaded darker, so a crown reads as a mass of foliage, not a shell.

import maplibregl from "maplibre-gl";
import type { TreeCover } from "./protocol";

/** Trees about this far apart, m. */
const SPACING_M = 7.5;
/** Trees are planted from this map zoom (below it the woods' ground texture shows them). */
const MIN_ZOOM = 13.5;
/** Planted out to this far from the middle of the view, m (at zoom 15; half that a zoom further in). */
const REACH_M = 2500;
/** Trees further than this from the middle of the view are drawn plainer: coarser crowns, no gaps, no trunks, m. */
const FAR_M = 400;
/** Planted again at most this often while more of the map loads, ms (a pan past what is planted plants at once). */
const REPLANT_MS = 1500;
/** Planted round the view and this much again each side, so a pan does not plant again. */
const MARGIN = 0.4;

export interface TreeOptions {
  /** The vector source, layer and kinds of land use that are woods. */
  source: string;
  sourceLayer: string;
  kinds: string[];
  /** The terrain's source: trees are set on the ground again once more of it has loaded. */
  terrainSource?: string;
  /** Most trees drawn at once (the nearest the middle of the view). */
  maxTrees: number;
  /** Land cover at 10 m (the pack's ESA WorldCover raster): trees are planted where it shows
   * trees too, beyond the mapped woods (gardens, avenues, overgrown ground). */
  cover?: TreeCover | null;
  /** The buildings' layer in `source`: no tree is planted inside a building. */
  buildingLayer?: string;
  /** The roads' layer in `source`: no tree from the tree cover is planted on a carriageway
   * (the cover counts an avenue's overhanging crowns as trees, but the trunks stand beside it). */
  roadLayer?: string;
}

export interface TreeStats {
  trees: number;
  /** Of them, planted from the tree cover (not in a mapped wood), and drawn plainer far off. */
  fromCover: number;
  far: number;
  conifers: number;
  plants: number;
  plantMs: number;
}

/** A small, fixed hash of a grid cell: the same tree every time. */
function hash(ix: number, iy: number, salt: number): number {
  let h = (Math.imul(ix, 374761393) + Math.imul(iy, 668265263) + Math.imul(salt, 2147483647)) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

/** Each grid cell (size `cell`, mercator) whose middle lies inside the rings (outer and holes,
 * even–odd), within the box: row by row, where the row crosses the rings' edges. */
function cellsInside(rs: number[][][], box: [number, number, number, number], cell: number, visit: (ix: number, iy: number) => void) {
  let x0 = Infinity;
  let x1 = -Infinity;
  let y0 = Infinity;
  let y1 = -Infinity;
  for (const p of rs[0]) {
    x0 = Math.min(x0, p[0]);
    x1 = Math.max(x1, p[0]);
    y0 = Math.min(y0, p[1]);
    y1 = Math.max(y1, p[1]);
  }
  x0 = Math.max(x0, box[0]);
  x1 = Math.min(x1, box[2]);
  y0 = Math.max(y0, box[1]);
  y1 = Math.min(y1, box[3]);
  if (x0 >= x1 || y0 >= y1) return;
  for (let iy = Math.ceil(y0 / cell - 0.5); (iy + 0.5) * cell <= y1; iy++) {
    const y = (iy + 0.5) * cell;
    const xs: number[] = [];
    for (const r of rs) {
      for (let i = 0, j = r.length - 1; i < r.length; j = i++) {
        const a = r[j];
        const b = r[i];
        if (a[1] <= y !== b[1] <= y) xs.push(a[0] + ((y - a[1]) * (b[0] - a[0])) / (b[1] - a[1]));
      }
    }
    xs.sort((p, q) => p - q);
    for (let k = 0; k + 1 < xs.length; k += 2) {
      const from = Math.max(xs[k], x0);
      const to = Math.min(xs[k + 1], x1);
      for (let ix = Math.ceil(from / cell - 0.5); (ix + 0.5) * cell < to; ix++) visit(ix, iy);
    }
  }
}

/** A feature's polygons, each as rings in mercator. */
function mercatorPolygons(f: maplibregl.MapGeoJSONFeature): number[][][][] {
  const gm = f.geometry;
  const polys = gm.type === "Polygon" ? [gm.coordinates] : gm.type === "MultiPolygon" ? gm.coordinates : [];
  return polys.map((rings) =>
    rings.map((r) =>
      r.map((p) => {
        const c = maplibregl.MercatorCoordinate.fromLngLat([p[0], p[1]]);
        return [c.x, c.y];
      }),
    ),
  );
}

/** A small seamless value-noise tile (64², 8 × 8 lattice cells), for the gaps between the
 * leaves: read from a texture, three lookups a pixel instead of computing noise in 3D. */
function noiseTile(): Uint8Array {
  const n = 64;
  const cells = 8;
  const lattice = (i: number, j: number) => hash(((i % cells) + cells) % cells, ((j % cells) + cells) % cells, 13);
  const out = new Uint8Array(n * n);
  for (let y = 0; y < n; y++) {
    for (let x = 0; x < n; x++) {
      const fx = (x / n) * cells;
      const fy = (y / n) * cells;
      const i = Math.floor(fx);
      const j = Math.floor(fy);
      const sx = (fx - i) * (fx - i) * (3 - 2 * (fx - i));
      const sy = (fy - j) * (fy - j) * (3 - 2 * (fy - j));
      const a = lattice(i, j) + (lattice(i + 1, j) - lattice(i, j)) * sx;
      const b = lattice(i, j + 1) + (lattice(i + 1, j + 1) - lattice(i, j + 1)) * sx;
      out[y * n + x] = Math.round((a + (b - a) * sy) * 255);
    }
  }
  return out;
}

/** The meshes, in a unit space (z up): a trunk, a broadleaf crown (near and far), a conifer crown. */
function meshes() {
  const out: Record<"trunk" | "dome" | "domeLow" | "cone", { pos: number[]; normal: number[]; idx: number[] }> = {
    trunk: { pos: [], normal: [], idx: [] },
    dome: { pos: [], normal: [], idx: [] },
    domeLow: { pos: [], normal: [], idx: [] },
    cone: { pos: [], normal: [], idx: [] },
  };
  // Trunk: six sides, radius 1, height 1 (no caps: the crown and the ground hide them).
  {
    const m = out.trunk;
    for (let i = 0; i <= 6; i++) {
      const a = (i / 6) * Math.PI * 2;
      for (const z of [0, 1]) {
        m.pos.push(Math.cos(a), Math.sin(a), z);
        m.normal.push(Math.cos(a), Math.sin(a), 0);
      }
    }
    for (let i = 0; i < 6; i++) m.idx.push(i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 1, i * 2 + 3, i * 2 + 2);
  }
  // Broadleaf crown: a sphere of radius 1, a little lumpy, so no two crowns' outlines are smooth;
  // and a coarser one for trees far off (a third of the triangles).
  for (const [name, rings, segs] of [
    ["dome", 6, 10],
    ["domeLow", 4, 6],
  ] as ["dome" | "domeLow", number, number][]) {
    const m = out[name];
    for (let j = 0; j <= rings; j++) {
      const v = j / rings;
      const phi = v * Math.PI;
      for (let i = 0; i <= segs; i++) {
        const a = (i / segs) * Math.PI * 2;
        const lump = j === 0 || j === rings ? 1 : 1 + 0.12 * Math.sin(a * 3 + j * 1.7) * Math.sin(phi);
        const nx = Math.sin(phi) * Math.cos(a);
        const ny = Math.sin(phi) * Math.sin(a);
        const nz = Math.cos(phi);
        m.pos.push(nx * lump, ny * lump, nz * lump);
        m.normal.push(nx, ny, nz);
      }
    }
    for (let j = 0; j < rings; j++) {
      for (let i = 0; i < segs; i++) {
        const a = j * (segs + 1) + i;
        const b = a + segs + 1;
        m.idx.push(a, b, a + 1, b, b + 1, a + 1);
      }
    }
  }
  // Conifer crown: two stacked cones, base radius 1 and 0.7, from z 0 to 1.
  {
    const m = out.cone;
    const segs = 10;
    for (const [z0, z1, r] of [
      [0, 0.68, 1],
      [0.36, 1, 0.68],
    ]) {
      const base = m.pos.length / 3;
      const slope = r / (z1 - z0);
      for (let i = 0; i <= segs; i++) {
        const a = (i / segs) * Math.PI * 2;
        const n = Math.hypot(1, slope);
        m.pos.push(Math.cos(a) * r, Math.sin(a) * r, z0);
        m.normal.push(Math.cos(a) / n, Math.sin(a) / n, slope / n);
        m.pos.push(0, 0, z1);
        m.normal.push(Math.cos(a) / n, Math.sin(a) / n, slope / n);
      }
      for (let i = 0; i < segs; i++) m.idx.push(base + i * 2, base + i * 2 + 2, base + i * 2 + 1);
      // Its underside, so a crown seen from below is closed.
      const c = m.pos.length / 3;
      m.pos.push(0, 0, z0);
      m.normal.push(0, 0, -1);
      for (let i = 0; i < segs; i++) m.idx.push(c, base + i * 2 + 2, base + i * 2);
    }
  }
  return out;
}

const VS = `
uniform mat4 u_matrix;
uniform float u_z_offset;
uniform float u_pm;
uniform float u_kind;
attribute vec3 a_pos;
attribute vec3 a_normal;
attribute vec3 i_pos;
attribute vec4 i_size;
attribute vec3 i_tint;
varying vec3 v_normal;
varying vec3 v_local;
varying vec3 v_tint;
varying float v_kind;
varying float v_up;
void main() {
  float h = i_size.x;
  float r = i_size.y;
  float turn = i_size.z * 6.2831853;
  vec3 p;
  vec3 n = a_normal;
  if (u_kind < 0.5) {
    // Trunk: up into the crown.
    float tr = 0.12 + 0.012 * h;
    p = vec3(a_pos.xy * tr, a_pos.z * h * 0.62);
    v_up = a_pos.z * 0.6;
  } else if (u_kind < 1.5) {
    // Broadleaf crown: from about a third of the way up to the top.
    float rv = 0.33 * h;
    p = vec3(a_pos.xy * r, h - rv + a_pos.z * rv);
    n = normalize(vec3(a_normal.xy / r, a_normal.z / rv));
    v_up = a_pos.z * 0.5 + 0.5;
  } else {
    // Conifer crown: from a fifth of the way up.
    float z0 = 0.2 * h;
    p = vec3(a_pos.xy * r, z0 + a_pos.z * (h - z0));
    v_up = a_pos.z;
  }
  float c = cos(turn);
  float s = sin(turn);
  p.xy = vec2(c * p.x - s * p.y, s * p.x + c * p.y);
  n.xy = vec2(c * n.x - s * n.y, s * n.x + c * n.y);
  v_normal = n;
  // Metres, for the pattern of gaps between the leaves (the same size on every tree).
  v_local = p + vec3(i_size.w * 97.0);
  v_tint = i_tint;
  v_kind = u_kind;
  gl_Position = u_matrix * vec4(i_pos.xy + p.xy * u_pm, i_pos.z + p.z * u_pm + u_z_offset, 1.0);
}`;

const FS = `
precision highp float;
uniform vec3 u_light;
uniform float u_holes;
uniform sampler2D u_noise;
varying vec3 v_normal;
varying vec3 v_local;
varying vec3 v_tint;
varying float v_kind;
varying float v_up;
void main() {
  vec3 n = normalize(v_normal);
  float inside = 1.0;
  if (v_kind > 0.5 && u_holes > 0.0) {
    // Gaps between the leaves: clumps of foliage about a metre across (two crossed lookups of the
    // noise tile, so no surface of the crown streaks), leaves at a quarter of that. Trees far off
    // have none: their gaps would be smaller than a pixel, and a solid crown is cheaper to draw.
    float clump = 0.5 * (texture2D(u_noise, v_local.xy * 0.139).r + texture2D(u_noise, vec2(v_local.x + v_local.y, v_local.z) * 0.139 + 0.5).r);
    float leaf = 0.65 * clump + 0.35 * texture2D(u_noise, v_local.yz * 0.54 + 0.3).r;
    if (leaf < u_holes) discard;
  }
  if (v_kind > 0.5) {
    // The far side of the crown, seen through a gap: in its own shade.
    if (!gl_FrontFacing) {
      n = -n;
      inside = 0.55;
    }
  }
  float sun = max(dot(n, u_light), 0.0);
  // Lower down, and on the shaded side, darker: the crown shades itself.
  float light = (0.5 + 0.6 * sun) * mix(0.72, 1.0, clamp(v_up, 0.0, 1.0)) * inside;
  // Bark, or the crown's own green.
  vec3 colour = v_kind < 0.5 ? vec3(0.24, 0.18, 0.13) : v_tint;
  gl_FragColor = vec4(colour * light, 1.0);
}`;

export type TreeLayer = maplibregl.CustomLayerInterface & { stats: TreeStats };

/** The custom layer. Add it after the ground's layers. */
export function treeLayer(id: string, o: TreeOptions): TreeLayer {
  let map: maplibregl.Map | null = null;
  let gl: WebGLRenderingContext | WebGL2RenderingContext | null = null;
  let program: WebGLProgram | null = null;
  const loc: Record<string, number> = {};
  const uni: Record<string, WebGLUniformLocation | null> = {};
  const mesh: Record<string, { vbo: WebGLBuffer | null; nbo: WebGLBuffer | null; ibo: WebGLBuffer | null; count: number }> = {};
  let inst: WebGLBuffer | null = null;
  let noise: WebGLTexture | null = null;
  /** Trees near and far off, by kind: drawn in that order from the instance buffer. */
  let broadleaf = 0;
  let pinesNear = 0;
  let broadFar = 0;
  let pinesFar = 0;
  let origin: [number, number] = [0, 0];
  let pmView = 0;
  /** Where the tree cover shows trees (1), pixel by pixel, once it has loaded. */
  let mask: Uint8Array | null = null;
  /** The planting grid's cell, mercator: set once. */
  let cell: number | null = null;
  let builtBox: [number, number, number, number] = [0, 0, 0, 0];
  let dirty = true;
  /** Whether the woods round the view have been planted (perhaps with no trees: no woods there). */
  let planted = false;
  let lastPlant = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let divisor: ((index: number, d: number) => void) | null = null;
  let drawInstanced: ((mode: number, count: number, type: number, offset: number, instances: number) => void) | null = null;
  const stats: TreeStats = { trees: 0, fromCover: 0, far: 0, conifers: 0, plants: 0, plantMs: 0 };

  const camElevation = () => ((map as unknown as { transform?: { elevation?: number } })?.transform?.elevation ?? 0);
  const perMetre = (lat: number) => maplibregl.MercatorCoordinate.fromLngLat([0, lat], 1).z;

  const viewBox = (m: maplibregl.Map): [number, number, number, number] => {
    const b = m.getBounds();
    const a = maplibregl.MercatorCoordinate.fromLngLat([b.getWest(), b.getNorth()]);
    const c = maplibregl.MercatorCoordinate.fromLngLat([b.getEast(), b.getSouth()]);
    return [a.x, a.y, c.x, c.y];
  };

  function check() {
    if (!map || !gl) return;
    const m = map;
    if (m.getZoom() < MIN_ZOOM || !m.getSource(o.source)) {
      stats.trees = 0;
      broadleaf = pinesNear = broadFar = pinesFar = 0;
      planted = false;
      m.triggerRepaint();
      return;
    }
    const v = viewBox(m);
    const inside = v[0] >= builtBox[0] && v[1] >= builtBox[1] && v[2] <= builtBox[2] && v[3] <= builtBox[3];
    if (!dirty && inside && planted) return;
    // Only more of the map has loaded where it is: not more often than every REPLANT_MS.
    const since = performance.now() - lastPlant;
    if (inside && planted && since < REPLANT_MS) {
      if (timer) clearTimeout(timer);
      timer = setTimeout(check, REPLANT_MS - since);
      return;
    }
    dirty = false;
    lastPlant = performance.now();
    const t0 = performance.now();
    plant(v, inside);
    planted = true;
    stats.plantMs = Math.round(performance.now() - t0);
    stats.plants++;
  }

  /** Plant the woods round the view. */
  function plant(v: [number, number, number, number], keep: boolean) {
    const m = map!;
    const g = gl!;
    // A new place to plant round, unless only more of the map has loaded where it is: then the
    // same, so the same trees stand (the nearest to it, when there are more than are drawn).
    const centre = keep && planted ? new maplibregl.MercatorCoordinate(origin[0], origin[1], 0) : maplibregl.MercatorCoordinate.fromLngLat(m.getCenter());
    origin = [centre.x, centre.y];
    pmView = perMetre(centre.toLngLat().lat);
    const w = v[2] - v[0];
    const h = v[3] - v[1];
    // Round the view, but no further than the reach (a pitched view's box runs to the horizon).
    const reach = REACH_M * Math.min(1, 2 ** (15 - m.getZoom())) * pmView;
    builtBox = [Math.max(v[0] - w * MARGIN, centre.x - reach), Math.max(v[1] - h * MARGIN, centre.y - reach), Math.min(v[2] + w * MARGIN, centre.x + reach), Math.min(v[3] + h * MARGIN, centre.y + reach)];
    // The grid, fixed for good: its size from the latitude to the nearest degree, not the view's
    // (which moves with every pan, and would move every tree with it).
    cell ??= SPACING_M * perMetre(Math.round(centre.toLngLat().lat));
    const seen = new Set<number>();
    const found: { ix: number; iy: number; d: number; wood: boolean }[] = [];
    const grid = cell;
    // Cells taken by buildings: no tree there; and by carriageways: no tree from the cover there.
    const built = new Set<number>();
    const road = new Set<number>();
    if (o.buildingLayer) {
      for (const f of m.querySourceFeatures(o.source, { sourceLayer: o.buildingLayer })) {
        for (const rs of mercatorPolygons(f)) cellsInside(rs, builtBox, grid, (ix, iy) => built.add(ix * 4194304 + iy));
      }
    }
    const take = (ix: number, iy: number, wood: boolean) => {
      // Each cell once (woods repeat across the tiles they straddle), some left clear, none in a building.
      const key = ix * 4194304 + iy;
      if (seen.has(key) || built.has(key) || hash(ix, iy, 7) < 0.12) return false;
      seen.add(key);
      found.push({ ix, iy, d: Math.hypot((ix + 0.5) * grid - centre.x, (iy + 0.5) * grid - centre.y), wood });
      return true;
    };
    // The mapped woods.
    for (const f of m.querySourceFeatures(o.source, { sourceLayer: o.sourceLayer, filter: ["in", ["get", "kind"], ["literal", o.kinds]] })) {
      for (const rs of mercatorPolygons(f)) cellsInside(rs, builtBox, grid, (ix, iy) => take(ix, iy, true));
    }
    // And wherever the tree cover shows trees, off the carriageways.
    let fromCover = 0;
    if (mask && o.cover && o.roadLayer) {
      const half: Record<string, number> = { highway: 9, major_road: 6, minor_road: 3.5 };
      for (const f of m.querySourceFeatures(o.source, { sourceLayer: o.roadLayer, filter: ["in", ["get", "kind"], ["literal", Object.keys(half)]] })) {
        const hw = (half[String(f.properties?.kind)] ?? 3.5) * pmView;
        const gm = f.geometry;
        const lines = gm.type === "LineString" ? [gm.coordinates] : gm.type === "MultiLineString" ? gm.coordinates : [];
        const reach = Math.ceil(hw / grid);
        for (const line of lines) {
          const pts = line.map((p) => maplibregl.MercatorCoordinate.fromLngLat([p[0], p[1]]));
          for (let i = 1; i < pts.length; i++) {
            const a = pts[i - 1];
            const b = pts[i];
            const len = Math.hypot(b.x - a.x, b.y - a.y);
            const steps = Math.max(1, Math.ceil(len / (grid * 0.5)));
            for (let k = 0; k <= steps; k++) {
              const x = a.x + ((b.x - a.x) * k) / steps;
              const y = a.y + ((b.y - a.y) * k) / steps;
              if (x < builtBox[0] - hw || x > builtBox[2] + hw || y < builtBox[1] - hw || y > builtBox[3] + hw) continue;
              const cx = Math.round(x / grid - 0.5);
              const cy = Math.round(y / grid - 0.5);
              for (let dy = -reach; dy <= reach; dy++) {
                for (let dx = -reach; dx <= reach; dx++) {
                  if (Math.hypot((cx + dx + 0.5) * grid - x, (cy + dy + 0.5) * grid - y) <= hw) road.add((cx + dx) * 4194304 + cy + dy);
                }
              }
            }
          }
        }
      }
    }
    if (mask && o.cover) {
      const c = o.cover;
      for (let iy = Math.ceil(builtBox[1] / grid - 0.5); (iy + 0.5) * grid <= builtBox[3]; iy++) {
        const lat = (Math.atan(Math.sinh(Math.PI * (1 - 2 * (iy + 0.5) * grid))) * 180) / Math.PI;
        const row = Math.floor((c.north - lat) / c.step);
        if (row < 0 || row >= c.height) continue;
        for (let ix = Math.ceil(builtBox[0] / grid - 0.5); (ix + 0.5) * grid <= builtBox[2]; ix++) {
          const col = Math.floor(((ix + 0.5) * grid * 360 - 180 - c.west) / c.step);
          if (col < 0 || col >= c.width || !mask[row * c.width + col] || road.has(ix * 4194304 + iy)) continue;
          if (take(ix, iy, false)) fromCover++;
        }
      }
    }
    stats.fromCover = fromCover;
    // More than may be drawn: thinned out with distance rather than cut off at a ring. Each
    // tree's distance is weighed by a fixed draw of its own, so near the middle nearly all
    // stand and further out fewer and fewer, the same ones each time.
    if (found.length > o.maxTrees) {
      for (const t of found) t.d *= 0.35 + hash(t.ix, t.iy, 11) * 1.3;
      found.sort((a, b) => a.d - b.d);
      found.length = o.maxTrees;
    }
    // Each tree: where it stands (relative to the origin; its foot's altitude), its size, turn and shade.
    const camEle = camElevation();
    // Near the middle of the view and far off: far ones are drawn plainer (see `render`).
    const groups: number[][] = [[], [], [], []];
    for (const { ix, iy, wood } of found) {
      const r = (s: number) => hash(ix, iy, s);
      const x = (ix + 0.5 + (r(1) - 0.5) * 0.8) * cell;
      const y = (iy + 0.5 + (r(2) - 0.5) * 0.8) * cell;
      const ll = new maplibregl.MercatorCoordinate(x, y, 0).toLngLat();
      const ground = (m.queryTerrainElevation(ll) ?? 0) + camEle;
      // Heights as trees grow: in a wood mostly 10–13 m, up to about 20 m, a few young ones
      // lower; in gardens, yards and streets mostly 6–8 m (about a house's eaves to its ridge),
      // one in ten 13–16 m. The tall ones are the rarer (r^1.3, r^1.6). Fewer conifers in town.
      const conifer = r(3) < (wood ? 0.33 : 0.15);
      let height: number;
      if (wood) {
        height = r(4) < 0.08 ? 18 + r(5) * 3 : 8 + 10 * r(5) ** 1.3;
        if (r(12) < 0.12) height *= 0.6;
      } else {
        height = r(4) < 0.1 ? 13 + r(5) * 3 : 4 + 8 * r(5) ** 1.6;
      }
      // Crowns in proportion: a broadleaf crown about three-fifths as wide as the tree is tall, a conifer's narrower.
      const crown = conifer ? Math.max(1, height * (0.15 + r(6) * 0.06)) : Math.max(1.6, height * (0.26 + r(6) * 0.12));
      const shade = 0.8 + r(8) * 0.4;
      // Greens: broadleaf trees lighter and warmer, some turning yellower; conifers darker and bluer.
      const tint = conifer ? [0.15 * shade, 0.31 * shade, 0.2 * shade] : [(0.24 + r(9) * 0.14) * shade, (0.43 + r(9) * 0.08) * shade, 0.17 * shade];
      const far = Math.hypot(x - origin[0], y - origin[1]) / pmView > FAR_M;
      groups[(far ? 2 : 0) + (conifer ? 1 : 0)].push(x - origin[0], y - origin[1], ground * pmView, height, crown, r(7), r(10), ...tint);
    }
    [broadleaf, pinesNear, broadFar, pinesFar] = groups.map((a) => a.length / 10);
    stats.trees = broadleaf + pinesNear + broadFar + pinesFar;
    stats.conifers = pinesNear + pinesFar;
    stats.far = broadFar + pinesFar;
    // In runs: near broadleaf, near conifers (the near ones' trunks drawn together), far broadleaf, far conifers.
    g.bindBuffer(g.ARRAY_BUFFER, inst);
    g.bufferData(g.ARRAY_BUFFER, new Float32Array(groups.flat()), g.STATIC_DRAW);
    m.triggerRepaint();
  }

  const later = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(check, 150);
  };
  const loaded = (e: maplibregl.MapSourceDataEvent) => {
    if ((e.sourceId === o.source || e.sourceId === o.terrainSource) && e.tile) dirty = true;
  };

  /** Bind the instance attributes from tree `first` on, and draw a mesh for `count` trees. */
  function draw(g: WebGLRenderingContext | WebGL2RenderingContext, name: string, kind: number, first: number, count: number) {
    const ms = mesh[name];
    if (!count || !drawInstanced || !divisor) return;
    g.uniform1f(uni.kind, kind);
    g.bindBuffer(g.ARRAY_BUFFER, ms.vbo);
    g.enableVertexAttribArray(loc.pos);
    g.vertexAttribPointer(loc.pos, 3, g.FLOAT, false, 0, 0);
    g.bindBuffer(g.ARRAY_BUFFER, ms.nbo);
    g.enableVertexAttribArray(loc.normal);
    g.vertexAttribPointer(loc.normal, 3, g.FLOAT, false, 0, 0);
    g.bindBuffer(g.ARRAY_BUFFER, inst);
    const stride = 40;
    const at = first * stride;
    for (const [l, size, off] of [
      [loc.ipos, 3, 0],
      [loc.isize, 4, 12],
      [loc.itint, 3, 28],
    ] as [number, number, number][]) {
      g.enableVertexAttribArray(l);
      g.vertexAttribPointer(l, size, g.FLOAT, false, stride, at + off);
      divisor(l, 1);
    }
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ms.ibo);
    drawInstanced(g.TRIANGLES, ms.count, g.UNSIGNED_SHORT, 0, count);
  }

  const self: TreeLayer = {
    id,
    type: "custom",
    renderingMode: "3d",
    stats,
    onAdd(m, g) {
      map = m;
      gl = g;
      // The tree cover, read once: its pixels that are trees (ESA WorldCover's class 10).
      const coverUrl = o.cover?.url;
      if (!coverUrl) console.info("trees: this pack has no tree cover, so trees stand only in its mapped woods (bgws basemap <pack> --tree-cover-only adds it)");
      if (o.cover && coverUrl) {
        const c = o.cover;
        const img = new Image();
        img.crossOrigin = "anonymous";
        img.onload = () => {
          try {
            const cv = document.createElement("canvas");
            cv.width = c.width;
            cv.height = c.height;
            const g2 = cv.getContext("2d", { willReadFrequently: true });
            if (!g2) return;
            g2.drawImage(img, 0, 0);
            const d = g2.getImageData(0, 0, c.width, c.height).data;
            const out = new Uint8Array(c.width * c.height);
            for (let i = 0; i < out.length; i++) out[i] = d[i * 4] === 10 ? 1 : 0;
            mask = out;
            dirty = true;
            later();
          } catch {
            // Not readable: the mapped woods only.
          }
        };
        img.onerror = () => console.warn("trees: the tree cover did not load:", coverUrl);
        img.src = coverUrl;
      }
      if (typeof WebGL2RenderingContext !== "undefined" && g instanceof WebGL2RenderingContext) {
        divisor = (i, d) => g.vertexAttribDivisor(i, d);
        drawInstanced = (mode, count, type, offset, n) => g.drawElementsInstanced(mode, count, type, offset, n);
      } else {
        const ext = g.getExtension("ANGLE_instanced_arrays");
        if (ext) {
          divisor = (i, d) => ext.vertexAttribDivisorANGLE(i, d);
          drawInstanced = (mode, count, type, offset, n) => ext.drawElementsInstancedANGLE(mode, count, type, offset, n);
        }
      }
      const shader = (type: number, src: string) => {
        const s = g.createShader(type)!;
        g.shaderSource(s, src);
        g.compileShader(s);
        if (!g.getShaderParameter(s, g.COMPILE_STATUS)) console.warn("trees:", g.getShaderInfoLog(s));
        return s;
      };
      program = g.createProgram()!;
      g.attachShader(program, shader(g.VERTEX_SHADER, VS));
      g.attachShader(program, shader(g.FRAGMENT_SHADER, FS));
      g.linkProgram(program);
      for (const [k, n] of [
        ["pos", "a_pos"],
        ["normal", "a_normal"],
        ["ipos", "i_pos"],
        ["isize", "i_size"],
        ["itint", "i_tint"],
      ]) loc[k] = g.getAttribLocation(program, n);
      for (const [k, n] of [
        ["matrix", "u_matrix"],
        ["z", "u_z_offset"],
        ["pm", "u_pm"],
        ["kind", "u_kind"],
        ["light", "u_light"],
        ["holes", "u_holes"],
        ["noise", "u_noise"],
      ]) uni[k] = g.getUniformLocation(program, n);
      // The noise tile for the gaps between the leaves: repeating, smoothed, mipmapped.
      noise = g.createTexture();
      g.bindTexture(g.TEXTURE_2D, noise);
      g.pixelStorei(g.UNPACK_ALIGNMENT, 1);
      g.texImage2D(g.TEXTURE_2D, 0, g.LUMINANCE, 64, 64, 0, g.LUMINANCE, g.UNSIGNED_BYTE, noiseTile());
      g.pixelStorei(g.UNPACK_ALIGNMENT, 4);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_S, g.REPEAT);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_T, g.REPEAT);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MAG_FILTER, g.LINEAR);
      g.generateMipmap(g.TEXTURE_2D);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MIN_FILTER, g.LINEAR_MIPMAP_LINEAR);
      for (const [name, md] of Object.entries(meshes())) {
        const b = { vbo: g.createBuffer(), nbo: g.createBuffer(), ibo: g.createBuffer(), count: md.idx.length };
        g.bindBuffer(g.ARRAY_BUFFER, b.vbo);
        g.bufferData(g.ARRAY_BUFFER, new Float32Array(md.pos), g.STATIC_DRAW);
        g.bindBuffer(g.ARRAY_BUFFER, b.nbo);
        g.bufferData(g.ARRAY_BUFFER, new Float32Array(md.normal), g.STATIC_DRAW);
        g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, b.ibo);
        g.bufferData(g.ELEMENT_ARRAY_BUFFER, new Uint16Array(md.idx), g.STATIC_DRAW);
        mesh[name] = b;
      }
      inst = g.createBuffer();
      m.on("idle", later);
      m.on("moveend", later);
      m.on("sourcedata", loaded);
      later();
    },
    onRemove(_m, g) {
      map?.off("idle", later);
      map?.off("moveend", later);
      map?.off("sourcedata", loaded);
      if (timer) clearTimeout(timer);
      for (const b of Object.values(mesh)) {
        g.deleteBuffer(b.vbo);
        g.deleteBuffer(b.nbo);
        g.deleteBuffer(b.ibo);
      }
      if (inst) g.deleteBuffer(inst);
      if (noise) g.deleteTexture(noise);
      if (program) g.deleteProgram(program);
      map = null;
      gl = null;
    },
    render(g, args) {
      if (!program || !map || !stats.trees) return;
      const matrix = (Array.isArray(args) || ArrayBuffer.isView(args) ? args : (args as { defaultProjectionData?: { mainMatrix: ArrayLike<number> } }).defaultProjectionData?.mainMatrix) as ArrayLike<number>;
      // The matrix with the origin moved in (in double precision, then to the GPU).
      const mm = Array.from(matrix);
      const [ox, oy] = origin;
      const shifted = new Float32Array(16);
      for (let r = 0; r < 4; r++) {
        for (let c = 0; c < 4; c++) shifted[c * 4 + r] = mm[c * 4 + r];
        shifted[12 + r] = mm[r] * ox + mm[4 + r] * oy + mm[12 + r];
      }
      if (typeof WebGL2RenderingContext !== "undefined" && g instanceof WebGL2RenderingContext) g.bindVertexArray(null);
      g.useProgram(program);
      g.uniformMatrix4fv(uni.matrix, false, shifted);
      g.uniform1f(uni.z, -camElevation() * pmView);
      g.uniform1f(uni.pm, pmView);
      // The sun low in the south-west, as the map's light (y runs south).
      const l = [-0.55, 0.5, 0.67];
      const len = Math.hypot(l[0], l[1], l[2]);
      g.uniform3f(uni.light, l[0] / len, l[1] / len, l[2] / len);
      g.disable(g.BLEND);
      g.disable(g.CULL_FACE);
      g.activeTexture(g.TEXTURE0);
      g.bindTexture(g.TEXTURE_2D, noise);
      g.uniform1i(uni.noise, 0);
      // Near: trunks, full crowns with gaps between the leaves.
      const near = broadleaf + pinesNear;
      g.uniform1f(uni.holes, 0.4);
      draw(g, "trunk", 0, 0, near);
      draw(g, "dome", 1, 0, broadleaf);
      draw(g, "cone", 2, broadleaf, pinesNear);
      // Far: coarser crowns, solid, no trunks (hidden under them at that distance).
      g.uniform1f(uni.holes, -1);
      draw(g, "domeLow", 1, near, broadFar);
      draw(g, "cone", 2, near + broadFar, pinesFar);
      // Leave the attributes as MapLibre expects them: no instancing.
      for (const k of ["ipos", "isize", "itint"]) {
        divisor?.(loc[k], 0);
        g.disableVertexAttribArray(loc[k]);
      }
      g.disableVertexAttribArray(loc.pos);
      g.disableVertexAttribArray(loc.normal);
    },
  };
  return self;
}
