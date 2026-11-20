// Satellite roofs: in 3D over satellite imagery, each building's top shows
// the imagery of its own roof rather than a flat colour.
//
// A MapLibre custom layer, so it shares the map's depth buffer: roofs hide
// behind nearer buildings and hills as the blocks do. The roofs are the
// buildings MapLibre has loaded round the view (the pack's basemap), each
// triangulated at its top: the building's height above the terrain at its
// centroid, as MapLibre places its blocks, and a little above, so the roof
// covers the block's plain top.
//
// The imagery is laid over them as a blanket, seen the way the satellite saw
// it: imagery is taken a little off straight down, so everything at a
// height shows in the photograph shifted from where it stands, by its height
// times one lean for the whole picture. A roof reads the imagery at its
// footprint plus its height times that lean. The lean is measured once, from
// the tallest buildings in view (`lean.ts`), and kept.
//
// Each roof is drawn once for every imagery tile it reads from, with that
// tile as its texture and anything outside the tile cut away, so a roof
// across a tile edge joins up. A new set of roofs (after a zoom, a pan past
// what was built, or more buildings loading) is drawn only once its imagery
// is in: until then the last set stays, so roofs never blink out.

import earcut from "earcut";
import maplibregl from "maplibre-gl";
import { ghostOutline } from "./ghost";
import { edgeImage, leanSearch, MAX_K, RADII, refineScale, spread, type EdgeMap, type Lean, type LeanBuilding } from "./lean";

/** Height of a building as the blocks draw it (see `buildingBlocks`). */
function heightOf(p: Record<string, unknown>): number {
  const h = Number(p.height ?? p.est_height ?? 6);
  return Number.isFinite(h) && h > 0 ? h : 6;
}

/** Raised this far over the block's top, m, so the roof covers it. */
const LIFT_M = 0.25;
/** Roofs are drawn from this map zoom (the blocks are too small below it). */
const MIN_ZOOM = 14;
/** Buildings at least this tall measure the lean (lower ones barely show it); the tallest this many. */
const LEAN_MIN_H = 10;
const LEAN_BUILDINGS = 60;
/** A lean is measured for the view per zoom-12 tile (about 6 km), and per zoom-14 tile (about 1.4 km)
 * where that has enough tall buildings: the mosaic's images change from place to place. */
const LEAN_ZOOM = 12;
const AREA_ZOOM = 14;
const AREA_MIN_BUILDINGS = 4;
/** Buildings at least this tall take their own share of the lean. */
const SHARE_MIN_H = 8;
/** A ghost is laid where the shift is at least this, m. */
const GHOST_MIN_M = 0.5;
/** Roofs are built over the view and this much again on each side, so a pan does not rebuild them. */
const MARGIN = 0.5;
/** A new set of roofs takes over once this share of its imagery is in, or after `SWAP_MS`. */
const SWAP_READY = 0.95;
const SWAP_MS = 3000;
/** The lean is measured in slices of this many milliseconds, between frames. */
const SLICE_MS = 12;

type TileKey = string;
interface Tex {
  tex: WebGLTexture | null;
  img: HTMLImageElement;
  ready: boolean;
  failed: boolean;
  used: number;
  /** Edge strength per pixel, spread to each of `RADII`, for measuring the lean (null: not readable). */
  edges?: Uint8Array[] | null;
}

interface Roof {
  /** Outline and holes relative to the build's origin, mercator; triangles over them. */
  flat: number[];
  tri: number[];
  /** Roof altitude, mercator units above sea level. */
  z: number;
  h: number;
  pm: number;
  ring: [number, number][];
  box: [number, number, number, number];
  /** Where it is, for its lean: its zoom-12 and zoom-14 tiles; and who it is, for its share. */
  place: string;
  area: string;
  id: string;
}

/** A piece of background work, run a slice at a time. */
interface Job {
  key: string;
  run: Generator<void, void>;
}

/** A set of roofs in the GPU's buffers, grouped by the imagery tile they read from. */
interface Built {
  vbo: WebGLBuffer | null;
  ibo: WebGLBuffer | null;
  runs: { key: TileKey; x: number; y: number; start: number; count: number }[];
  origin: [number, number];
  zoom: number;
  wide: boolean;
  /** The mercator box its roofs cover. */
  box: [number, number, number, number];
  made: number;
}

const VS = `
uniform mat4 u_matrix;
uniform float u_z_offset;
attribute vec3 a_pos;
attribute vec2 a_shift;
varying vec2 v_src;
void main() {
  v_src = a_pos.xy + a_shift;
  gl_Position = u_matrix * vec4(a_pos.xy, a_pos.z + u_z_offset, 1.0);
}`;

const FS = `
precision highp float;
uniform sampler2D u_image;
uniform vec2 u_tile_min;
uniform float u_scale;
uniform float u_brightness;
varying vec2 v_src;
void main() {
  vec2 uv = (v_src - u_tile_min) * u_scale;
  if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) discard;
  vec4 c = texture2D(u_image, uv);
  gl_FragColor = vec4(c.rgb * u_brightness, 1.0);
}`;

export interface RoofOptions {
  /** The vector source and layer the buildings come from. */
  source: string;
  sourceLayer: string;
  /** The terrain's source: roofs are placed again once more of it has loaded. */
  terrainSource?: string;
  /** A GeoJSON source the buildings' ghosts (where the imagery shows them lying on the ground) are written to. */
  ghostSource?: string;
  /** The imagery's tile URL, `{z}/{y}/{x}` as for the raster source. */
  tiles: string;
  /** The imagery's brightness as the ground draws it (`raster-brightness-max`). */
  brightness: number;
  /** Most imagery tiles held and drawn at once. */
  maxTiles: number;
}

/** What the layer last measured, for display and checks. */
export interface RoofStats {
  roofs: number;
  /** The view's lean, and where it was measured (zoom-12 tile), if it has been. */
  lean: Lean | null;
  place: string | null;
  /** Areas with a lean of their own (by zoom-14 tile), buildings with a share measured, ghosts laid. */
  areas: Record<string, [number, number]>;
  shares: number;
  ghosts: number;
  /** Sets of roofs built, and swapped in; how long the last build took, and background work in all (spread over frames), ms. */
  builds: number;
  swaps: number;
  buildMs: number;
  measureMs: number;
}

export type RoofLayer = maplibregl.CustomLayerInterface & {
  stats: RoofStats;
  /** Correct for the imagery's lean (on by default). */
  lean: boolean;
  /** Draw again (after changing `lean`). */
  refresh(): void;
};

/** The custom layer. Add it after the building blocks. */
export function roofLayer(id: string, o: RoofOptions): RoofLayer {
  let map: maplibregl.Map | null = null;
  let gl: WebGLRenderingContext | WebGL2RenderingContext | null = null;
  let program: WebGLProgram | null = null;
  let loc: { pos: number; shift: number; matrix: WebGLUniformLocation | null; z: WebGLUniformLocation | null; image: WebGLUniformLocation | null; tileMin: WebGLUniformLocation | null; scale: WebGLUniformLocation | null; brightness: WebGLUniformLocation | null } | null = null;
  let uint32 = false;
  let frame = 0;
  /** The roofs on screen, and the next set while its imagery loads. */
  let current: Built | null = null;
  let pending: Built | null = null;
  /** The latest roofs' geometry, for the next upload. */
  let roofs: Roof[] = [];
  let origin: [number, number] = [0, 0];
  let builtBox: [number, number, number, number] = [0, 0, 0, 0];
  /** The imagery's zoom: changed only when the map's zoom has moved well past it. */
  let band = 0;
  /** Buildings or terrain loaded since the roofs were built. */
  let dirty = true;
  /** Leans measured (roof shift east, south per metre of height): for views by zoom-12 tile, for
   * areas by zoom-14 tile; buildings' shares by lean and building. Each measured once and kept. */
  const leans = new Map<string, Lean>();
  const areaLeans = new Map<string, Lean>();
  const shares = new Map<string, number>();
  /** The last lean measured anywhere: for places not measured yet. */
  let fallback: [number, number] | null = null;
  /** Background work: measuring, then laying the ghosts. */
  const jobs: Job[] = [];
  let running = false;
  let uploadTimer: ReturnType<typeof setTimeout> | null = null;
  const ghostCache = new Map<string, [number, number][][][]>();
  /** The ghosts need laying again (new roofs, or a lean or share measured). */
  let ghostDirty = true;
  /** Tries at each lean, by job: one whose imagery never gives a clear answer is left after a few. */
  const tries = new Map<string, number>();
  const MAX_TRIES = 3;
  const textures = new Map<TileKey, Tex>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let leanTimer: ReturnType<typeof setTimeout> | null = null;
  const stats: RoofStats = { roofs: 0, lean: null, place: null, areas: {}, shares: 0, ghosts: 0, builds: 0, swaps: 0, buildMs: 0, measureMs: 0 };

  const url = (z: number, x: number, y: number) => o.tiles.replace("{z}", String(z)).replace("{x}", String(x)).replace("{y}", String(y));
  /** The camera's ground elevation, m (what queryTerrainElevation is relative to). */
  const camElevation = () => ((map as unknown as { transform?: { elevation?: number } })?.transform?.elevation ?? 0);
  /** Mercator units per metre at a latitude. */
  const perMetre = (lat: number) => maplibregl.MercatorCoordinate.fromLngLat([0, lat], 1).z;

  function texture(key: TileKey, z: number, x: number, y: number): Tex {
    let t = textures.get(key);
    if (t) {
      t.used = frame;
      return t;
    }
    const img = new Image();
    img.crossOrigin = "anonymous";
    t = { tex: null, img, ready: false, failed: false, used: frame };
    const entry = t;
    img.onload = () => {
      if (!gl) return;
      const g = gl;
      entry.tex = g.createTexture();
      g.bindTexture(g.TEXTURE_2D, entry.tex);
      g.pixelStorei(g.UNPACK_FLIP_Y_WEBGL, false);
      g.texImage2D(g.TEXTURE_2D, 0, g.RGBA, g.RGBA, g.UNSIGNED_BYTE, img);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_S, g.CLAMP_TO_EDGE);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_WRAP_T, g.CLAMP_TO_EDGE);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MAG_FILTER, g.LINEAR);
      // Mipmaps: roofs seen at a slant, or zoomed out, stay smooth (tiles are 256², a power of two).
      g.generateMipmap(g.TEXTURE_2D);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MIN_FILTER, g.LINEAR_MIPMAP_LINEAR);
      entry.ready = true;
      map?.triggerRepaint();
      // More imagery to measure on.
      planSoon();
    };
    img.onerror = () => {
      entry.failed = true;
    };
    img.src = url(z, x, y);
    textures.set(key, t);
    // Keep the most recently used tiles; let the rest go (never one the roofs on screen or coming use).
    if (textures.size > o.maxTiles * 3) {
      const keep = new Set([...(current?.runs ?? []), ...(pending?.runs ?? [])].map((r) => r.key));
      const old = [...textures.entries()].filter(([k2]) => !keep.has(k2)).sort((a, b) => a[1].used - b[1].used);
      for (const [k2, v] of old.slice(0, textures.size - o.maxTiles * 3)) {
        if (v.tex && gl) gl.deleteTexture(v.tex);
        v.img.onload = null;
        textures.delete(k2);
      }
    }
    return t;
  }

  /** The imagery's edges at a mercator point (NaN where not loaded or not readable). */
  let last: { key: number; edges: Uint8Array[] | null } = { key: -1, edges: null };
  const edge: EdgeMap = (mx, my, level) => {
    const n = 2 ** band;
    const fx = mx * n;
    const fy = my * n;
    const tx = Math.floor(fx);
    const ty = Math.floor(fy);
    // Most reads fall in the tile the last one did.
    const key = tx * 1048576 + ty;
    let edges = last.key === key ? last.edges : null;
    if (last.key !== key) {
      const t = textures.get(`${band}/${tx}/${ty}`);
      if (t && t.ready) {
        if (t.edges === undefined) {
          try {
            const c = document.createElement("canvas");
            c.width = c.height = 256;
            const g2 = c.getContext("2d", { willReadFrequently: true });
            if (!g2) throw new Error("no 2d");
            g2.drawImage(t.img, 0, 0, 256, 256);
            const e0 = edgeImage(g2.getImageData(0, 0, 256, 256).data, 256, 256);
            t.edges = RADII.map((r) => spread(e0, 256, 256, r));
          } catch {
            // Imagery served without leave to read it: no lean is measured.
            t.edges = null;
          }
        }
        edges = t.edges ?? null;
      }
      // Only a loaded tile is remembered: one still loading is looked for again.
      last = edges ? { key, edges } : { key: -1, edges: null };
    }
    if (!edges) return NaN;
    const px = Math.min(255, Math.max(0, Math.floor((fx - tx) * 256)));
    const py = Math.min(255, Math.max(0, Math.floor((fy - ty) * 256)));
    return edges[level][py * 256 + px];
  };

  /** The tile at `z` a mercator point is in, as a key. */
  const tileKey = (x: number, y: number, z: number) => `${z}/${Math.floor(x * 2 ** z)}/${Math.floor(y * 2 ** z)}`;
  /** Where the view is: its zoom-12 tile. */
  const place = () => {
    if (!map) return "";
    const c = maplibregl.MercatorCoordinate.fromLngLat(map.getCenter());
    return tileKey(c.x, c.y, LEAN_ZOOM);
  };

  /** The view's mercator box. */
  const viewBox = (m: maplibregl.Map): [number, number, number, number] => {
    const b = m.getBounds();
    const a = maplibregl.MercatorCoordinate.fromLngLat([b.getWest(), b.getNorth()]);
    const c = maplibregl.MercatorCoordinate.fromLngLat([b.getEast(), b.getSouth()]);
    return [a.x, a.y, c.x, c.y];
  };

  /** Build again if the view needs it: a new imagery zoom, a pan past what is built, or more loaded. */
  function check() {
    if (!map || !gl) return;
    const m = map;
    const z = m.getZoom();
    if (z < MIN_ZOOM || !m.getSource(o.source)) {
      drop(current);
      drop(pending);
      current = pending = null;
      m.triggerRepaint();
      return;
    }
    // Imagery about as sharp as the ground's (256-pixel tiles), kept until the zoom is well past it.
    const want = Math.max(16, Math.min(19, Math.round(z) + 1));
    if (band === 0 || Math.abs(z + 1 - band) > 0.8) band = want;
    const v = viewBox(m);
    const inside = v[0] >= builtBox[0] && v[1] >= builtBox[1] && v[2] <= builtBox[2] && v[3] <= builtBox[3];
    const shown = pending ?? current;
    if (!dirty && shown && shown.zoom === band && inside) return;
    dirty = false;
    ghostDirty = true;
    const t0 = performance.now();
    build(v);
    upload();
    stats.buildMs = Math.round(performance.now() - t0);
    planSoon();
  }

  /** Roofs for the buildings loaded round the view. */
  function build(v: [number, number, number, number]) {
    const m = map!;
    const w = v[2] - v[0];
    const h = v[3] - v[1];
    builtBox = [v[0] - w * MARGIN, v[1] - h * MARGIN, v[2] + w * MARGIN, v[3] + h * MARGIN];
    origin = [(v[0] + v[2]) / 2, (v[1] + v[3]) / 2];
    const camEle = camElevation();
    roofs = [];
    for (const f of m.querySourceFeatures(o.source, { sourceLayer: o.sourceLayer })) {
      const g = f.geometry;
      const polys = g.type === "Polygon" ? [g.coordinates] : g.type === "MultiPolygon" ? g.coordinates : [];
      const hgt = heightOf(f.properties ?? {});
      for (const rings of polys) {
        const outer = rings[0];
        if (!outer || outer.length < 4) continue;
        // Its centroid, where its block takes the ground's height.
        let cx = 0;
        let cy = 0;
        for (const p of outer) {
          cx += p[0];
          cy += p[1];
        }
        cx /= outer.length;
        cy /= outer.length;
        const cm = maplibregl.MercatorCoordinate.fromLngLat([cx, cy]);
        if (cm.x < builtBox[0] || cm.x > builtBox[2] || cm.y < builtBox[1] || cm.y > builtBox[3]) continue;
        const ground = (m.queryTerrainElevation([cx, cy]) ?? 0) + camEle;
        const pm = perMetre(cy);
        const flat: number[] = [];
        const holes: number[] = [];
        const ring: [number, number][] = [];
        const box: [number, number, number, number] = [Infinity, Infinity, -Infinity, -Infinity];
        for (let r = 0; r < rings.length; r++) {
          if (r > 0) holes.push(flat.length / 2);
          // The closing point repeats the first: left out.
          for (let q = 0; q < rings[r].length - 1; q++) {
            const mc = maplibregl.MercatorCoordinate.fromLngLat([rings[r][q][0], rings[r][q][1]]);
            flat.push(mc.x - origin[0], mc.y - origin[1]);
            if (r === 0) {
              ring.push([mc.x, mc.y]);
              box[0] = Math.min(box[0], mc.x);
              box[1] = Math.min(box[1], mc.y);
              box[2] = Math.max(box[2], mc.x);
              box[3] = Math.max(box[3], mc.y);
            }
          }
        }
        const tri = earcut(flat, holes.length ? holes : undefined, 2);
        if (!tri.length) continue;
        roofs.push({ flat, tri, z: (ground + hgt + LIFT_M) * pm, h: hgt, pm, ring, box, place: tileKey(cm.x, cm.y, LEAN_ZOOM), area: tileKey(cm.x, cm.y, AREA_ZOOM), id: `${ring[0][0]},${ring[0][1]},${hgt}` });
      }
    }
    stats.roofs = roofs.length;
    stats.builds++;
  }

  /** A roof's lean (view's, or its area's), as far as it is known. */
  const leanOf = (r: Roof): [number, number] | null => (self.lean ? (areaLeans.get(r.area)?.k ?? leans.get(r.place)?.k ?? fallback) : null);
  const shareKey = (k: [number, number], r: Roof) => `${k[0].toFixed(3)},${k[1].toFixed(3)}|${r.id}`;
  /** A roof's shift, mercator: its height times its lean, times its own share. */
  const shiftOf = (r: Roof): [number, number] => {
    const k = leanOf(r);
    if (!k) return [0, 0];
    const m = r.h * r.pm * (shares.get(shareKey(k, r)) ?? 1);
    return [k[0] * m, k[1] * m];
  };

  /** Load the imagery the roofs of these buildings could have been photographed in. */
  function want(bs: Roof[]) {
    const n = 2 ** band;
    for (const b of bs) {
      const reach = MAX_K * b.h * b.pm * 2;
      for (let tx = Math.floor((b.box[0] - reach) * n); tx <= Math.floor((b.box[2] + reach) * n); tx++) {
        for (let ty = Math.floor((b.box[1] - reach) * n); ty <= Math.floor((b.box[3] + reach) * n); ty++) texture(`${band}/${tx}/${ty}`, band, tx, ty);
      }
    }
  }

  const tallest = (rs: Roof[]): LeanBuilding[] =>
    rs
      .filter((r) => r.h >= LEAN_MIN_H)
      .sort((a, b) => b.h - a.h)
      .slice(0, LEAN_BUILDINGS)
      .map((r) => ({ ring: r.ring, h: r.h, pm: r.pm }));

  /** Measure a lean for these buildings; kept under `key` once measured on enough of them. */
  function* leanJob(rs: Roof[], into: Map<string, Lean>, key: string, job: string): Generator<void, void> {
    const tall = tallest(rs);
    if (tall.length < AREA_MIN_BUILDINGS) return;
    want(rs.filter((r) => r.h >= LEAN_MIN_H));
    const zoomAt = band;
    const search = leanSearch(tall, edge, 1 / (2 ** band * 256));
    let r = search.next();
    while (!r.done) {
      yield;
      // The imagery's zoom changed under it: measured again later.
      if (band !== zoomAt) return;
      last = { key: -1, edges: null };
      r = search.next();
    }
    if (r.value.used >= Math.min(20, tall.length)) {
      into.set(key, r.value);
      fallback = r.value.k;
      changed();
    } else if (r.value.used >= tall.length * 0.8) {
      // Its imagery was in and still no clear lean: a try. (One cut short by imagery still loading is not.)
      tries.set(job, (tries.get(job) ?? 0) + 1);
    }
  }

  /** Each tall building's own share of its lean, where its imagery is in. */
  function* shareJob(): Generator<void, void> {
    const zoomAt = band;
    const px = 1 / (2 ** band * 256);
    let n = 0;
    for (const r of roofs) {
      const k = leanOf(r);
      if (!k || r.h < SHARE_MIN_H || shares.has(shareKey(k, r))) continue;
      if (band !== zoomAt) return;
      last = { key: -1, edges: null };
      const s = refineScale({ ring: r.ring, h: r.h, pm: r.pm }, k, edge, px);
      if (s !== null) {
        shares.set(shareKey(k, r), s);
        n++;
      }
      yield;
    }
    if (n) changed();
  }

  /** The ghosts of the buildings round the view, written to the ghost source. */
  function* ghostJob(): Generator<void, void> {
    if (!o.ghostSource) return;
    ghostDirty = false;
    const features: GeoJSON.Feature[] = [];
    let i = 0;
    for (const r of roofs) {
      const d = shiftOf(r);
      if (Math.hypot(d[0], d[1]) / r.pm < GHOST_MIN_M) continue;
      const key = `${r.id}|${d[0].toExponential(4)},${d[1].toExponential(4)}`;
      let g = ghostCache.get(key);
      if (!g) {
        // Worked out in mercator, then to longitude and latitude.
        g = ghostOutline(r.ring, d).map((poly) => poly.map((ring) => ring.map((p) => {
          const ll = new maplibregl.MercatorCoordinate(p[0], p[1], 0).toLngLat();
          return [ll.lng, ll.lat] as [number, number];
        })));
        if (ghostCache.size > 50000) ghostCache.clear();
        ghostCache.set(key, g);
      }
      if (g.length) features.push({ type: "Feature", properties: {}, geometry: { type: "MultiPolygon", coordinates: g } });
      if (++i % 25 === 0) yield;
    }
    const src = map?.getSource(o.ghostSource) as maplibregl.GeoJSONSource | undefined;
    src?.setData({ type: "FeatureCollection", features });
    stats.ghosts = features.length;
  }

  /** Queue a job unless it is queued already. */
  function queue(key: string, make: () => Generator<void, void>, limited = false) {
    if (jobs.some((j) => j.key === key)) return;
    if (limited && (tries.get(key) ?? 0) >= MAX_TRIES) return;
    jobs.push({ key, run: make() });
    if (!running) {
      running = true;
      setTimeout(work, 0);
    }
  }

  /** Background work in slices of a few milliseconds between frames, so the map never stalls on it. */
  function work() {
    const t0 = performance.now();
    while (jobs.length && performance.now() - t0 < SLICE_MS) {
      if (jobs[0].run.next().done) jobs.shift();
    }
    stats.measureMs += Math.round(performance.now() - t0);
    if (jobs.length && map) setTimeout(work, 0);
    else running = false;
  }

  /** What is still to measure round the view, queued: the view's lean, its areas', the buildings' shares, then the ghosts. */
  function plan() {
    if (!map || !gl || !roofs.length) return;
    if (self.lean) {
      const byPlace = new Map<string, Roof[]>();
      const byArea = new Map<string, Roof[]>();
      for (const r of roofs) {
        byPlace.set(r.place, [...(byPlace.get(r.place) ?? []), r]);
        if (r.h >= LEAN_MIN_H) byArea.set(r.area, [...(byArea.get(r.area) ?? []), r]);
      }
      for (const [p, rs] of byPlace) if (!leans.has(p)) queue(`view:${p}:${band}`, () => leanJob(rs, leans, p, `view:${p}:${band}`), true);
      for (const [a, rs] of byArea) if (!areaLeans.has(a) && rs.length >= AREA_MIN_BUILDINGS) queue(`area:${a}:${band}`, () => leanJob(rs, areaLeans, a, `area:${a}:${band}`), true);
      queue("shares", shareJob);
    }
    if (ghostDirty) queue("ghosts", ghostJob);
    const here = leans.get(place());
    stats.lean = here ?? null;
    stats.place = here ? place() : null;
    stats.areas = Object.fromEntries([...areaLeans].map(([a, l]) => [a, l.k]));
    stats.shares = shares.size;
  }

  function planSoon() {
    if (leanTimer) clearTimeout(leanTimer);
    leanTimer = setTimeout(plan, 400);
  }

  /** A lean or share was measured: draw the roofs with it (soon, once for several), and measure what follows from it. */
  function changed() {
    if (uploadTimer) clearTimeout(uploadTimer);
    ghostDirty = true;
    uploadTimer = setTimeout(() => {
      upload();
      plan();
    }, 250);
  }

  function drop(b: Built | null) {
    if (!b || !gl) return;
    if (b.vbo) gl.deleteBuffer(b.vbo);
    if (b.ibo) gl.deleteBuffer(b.ibo);
  }

  /** The roofs, with the lean, into a new set of buffers, drawn once their imagery is in. */
  function upload() {
    if (!map || !gl) return;
    const g = gl;
    const n = 2 ** band;
    const data: number[] = [];
    const byTile = new Map<TileKey, { x: number; y: number; idx: number[]; d: number }>();
    for (const r of roofs) {
      const [sx, sy] = shiftOf(r);
      const base = data.length / 5;
      for (let q = 0; q < r.flat.length; q += 2) data.push(r.flat[q], r.flat[q + 1], r.z, sx, sy);
      // Every tile its roof reads from draws its part of it.
      for (let tx = Math.floor((r.box[0] + sx) * n); tx <= Math.floor((r.box[2] + sx) * n); tx++) {
        for (let ty = Math.floor((r.box[1] + sy) * n); ty <= Math.floor((r.box[3] + sy) * n); ty++) {
          const key = `${band}/${tx}/${ty}`;
          let t = byTile.get(key);
          if (!t) {
            t = { x: tx, y: ty, idx: [], d: Math.hypot((tx + 0.5) / n - origin[0], (ty + 0.5) / n - origin[1]) };
            byTile.set(key, t);
          }
          for (const v of r.tri) t.idx.push(base + v);
        }
      }
    }
    // The tiles nearest the middle of the view, as many as are allowed.
    const tiles = [...byTile.entries()].sort((a, b) => a[1].d - b[1].d).slice(0, o.maxTiles);
    const total = tiles.reduce((s, [, t]) => s + t.idx.length, 0);
    const vertices = data.length / 5;
    if (!total || (vertices > 65535 && !uint32)) return;
    const wide = vertices > 65535;
    const indices = wide ? new Uint32Array(total) : new Uint16Array(total);
    const runs: Built["runs"] = [];
    let at = 0;
    for (const [key, t] of tiles) {
      indices.set(t.idx, at);
      runs.push({ key, x: t.x, y: t.y, start: at, count: t.idx.length });
      at += t.idx.length;
      // Its imagery starts loading now.
      texture(key, band, t.x, t.y);
    }
    const b: Built = { vbo: g.createBuffer(), ibo: g.createBuffer(), runs, origin: [...origin], zoom: band, wide, box: [...builtBox], made: performance.now() };
    g.bindBuffer(g.ARRAY_BUFFER, b.vbo);
    g.bufferData(g.ARRAY_BUFFER, new Float32Array(data), g.STATIC_DRAW);
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, b.ibo);
    g.bufferData(g.ELEMENT_ARRAY_BUFFER, indices, g.STATIC_DRAW);
    drop(pending);
    pending = b;
    map.triggerRepaint();
  }

  /** Whether a set's imagery is in (or has had its time). */
  function ready(b: Built): boolean {
    if (performance.now() - b.made > SWAP_MS) return true;
    let done = 0;
    for (const r of b.runs) {
      const t = textures.get(r.key);
      if (t && (t.ready || t.failed)) done++;
    }
    return done >= b.runs.length * SWAP_READY;
  }

  const later = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(check, 150);
  };
  const loaded = (e: maplibregl.MapSourceDataEvent) => {
    if ((e.sourceId === o.source || e.sourceId === o.terrainSource) && e.tile) dirty = true;
  };

  const self: RoofLayer = {
    id,
    type: "custom",
    renderingMode: "3d",
    stats,
    lean: true,
    refresh: () => {
      ghostDirty = true;
      upload();
      plan();
    },
    onAdd(m, g) {
      map = m;
      gl = g;
      uint32 = typeof WebGL2RenderingContext !== "undefined" && g instanceof WebGL2RenderingContext ? true : !!g.getExtension("OES_element_index_uint");
      const shader = (type: number, src: string) => {
        const s = g.createShader(type)!;
        g.shaderSource(s, src);
        g.compileShader(s);
        if (!g.getShaderParameter(s, g.COMPILE_STATUS)) console.warn("roofs:", g.getShaderInfoLog(s));
        return s;
      };
      program = g.createProgram()!;
      g.attachShader(program, shader(g.VERTEX_SHADER, VS));
      g.attachShader(program, shader(g.FRAGMENT_SHADER, FS));
      g.linkProgram(program);
      loc = {
        pos: g.getAttribLocation(program, "a_pos"),
        shift: g.getAttribLocation(program, "a_shift"),
        matrix: g.getUniformLocation(program, "u_matrix"),
        z: g.getUniformLocation(program, "u_z_offset"),
        image: g.getUniformLocation(program, "u_image"),
        tileMin: g.getUniformLocation(program, "u_tile_min"),
        scale: g.getUniformLocation(program, "u_scale"),
        brightness: g.getUniformLocation(program, "u_brightness"),
      };
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
      if (leanTimer) clearTimeout(leanTimer);
      if (uploadTimer) clearTimeout(uploadTimer);
      jobs.length = 0;
      drop(current);
      drop(pending);
      current = pending = null;
      for (const t of textures.values()) {
        if (t.tex) g.deleteTexture(t.tex);
        t.img.onload = null;
      }
      textures.clear();
      if (program) g.deleteProgram(program);
      map = null;
      gl = null;
    },
    render(g, args) {
      if (!program || !loc || !map) return;
      frame++;
      // The next set takes over once its imagery is in (at once if nothing is on screen).
      if (pending && (!current || ready(pending))) {
        drop(current);
        current = pending;
        pending = null;
        stats.swaps++;
      } else if (pending) {
        map.triggerRepaint();
      }
      const b = current;
      if (!b) return;
      const matrix = (Array.isArray(args) || ArrayBuffer.isView(args) ? args : (args as { defaultProjectionData?: { mainMatrix: ArrayLike<number> } }).defaultProjectionData?.mainMatrix) as ArrayLike<number>;
      // The matrix with the origin moved in (in double precision, then to the GPU).
      const mm = Array.from(matrix);
      const [ox, oy] = b.origin;
      const shifted = new Float32Array(16);
      for (let r = 0; r < 4; r++) {
        for (let c = 0; c < 4; c++) shifted[c * 4 + r] = mm[c * 4 + r];
        shifted[12 + r] = mm[r] * ox + mm[4 + r] * oy + mm[12 + r];
      }
      g.useProgram(program);
      g.uniformMatrix4fv(loc.matrix, false, shifted);
      // Heights are stored above sea level; the matrix is about the camera's ground.
      g.uniform1f(loc.z, -camElevation() * perMetre(map.getCenter().lat));
      g.uniform1f(loc.scale, 2 ** b.zoom);
      g.uniform1f(loc.brightness, o.brightness);
      g.uniform1i(loc.image, 0);
      g.bindBuffer(g.ARRAY_BUFFER, b.vbo);
      g.enableVertexAttribArray(loc.pos);
      g.vertexAttribPointer(loc.pos, 3, g.FLOAT, false, 20, 0);
      g.enableVertexAttribArray(loc.shift);
      g.vertexAttribPointer(loc.shift, 2, g.FLOAT, false, 20, 12);
      g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, b.ibo);
      g.disable(g.CULL_FACE);
      g.disable(g.BLEND);
      g.activeTexture(g.TEXTURE0);
      const type = b.wide ? g.UNSIGNED_INT : g.UNSIGNED_SHORT;
      const bytes = b.wide ? 4 : 2;
      const n = 2 ** b.zoom;
      for (const r of b.runs) {
        const t = texture(r.key, b.zoom, r.x, r.y);
        if (!t.ready || !t.tex) continue;
        g.bindTexture(g.TEXTURE_2D, t.tex);
        g.uniform2f(loc.tileMin, r.x / n - ox, r.y / n - oy);
        g.drawElements(g.TRIANGLES, r.count, type, r.start * bytes);
      }
      g.disableVertexAttribArray(loc.pos);
      g.disableVertexAttribArray(loc.shift);
    },
  };
  return self;
}
