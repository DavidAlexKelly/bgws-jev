// Satellite roofs: in 3D over satellite imagery, each building's top shows
// the imagery of its own roof rather than a flat colour.
//
// A MapLibre custom layer, so it shares the map's depth buffer: roofs hide
// behind nearer buildings and hills as the blocks do. The roofs are the
// buildings MapLibre has loaded for the view (the pack's basemap), each
// triangulated at its top: the building's height above the terrain at its
// centroid, as MapLibre places its blocks, and a little above, so the roof
// covers the block's plain top.
//
// Imagery is taken a little off straight down, so a roof shows in the
// photograph shifted from its footprint, by its height, away from where the
// satellite was. That lean is measured from the imagery (`lean.ts`): for each
// area, the shift per metre of height at which the tall buildings' roof
// outlines fall on the picture's edges, and then each building's own share
// of it (a mapped or estimated height can be off). Each roof then reads the
// imagery where its roof was photographed, not where its footprint is.
//
// Each roof is drawn once for every imagery tile it reads from, with that
// tile as its texture and anything outside the tile cut away, so a roof
// across a tile edge joins up.

import earcut from "earcut";
import maplibregl from "maplibre-gl";
import { edgeImage, edgeScore, estimateLean, MAX_K, MIN_GAIN, RADII, refineScale, spread, type EdgeMap, type Lean, type LeanBuilding } from "./lean";

/** Height of a building as the blocks draw it (see `buildingBlocks`). */
function heightOf(p: Record<string, unknown>): number {
  const h = Number(p.height ?? p.est_height ?? 6);
  return Number.isFinite(h) && h > 0 ? h : 6;
}

/** Raised this far over the block's top, m, so the roof covers it. */
const LIFT_M = 0.25;
/** Buildings at least this tall measure the lean (lower ones barely show it). */
const LEAN_MIN_H = 10;
/** An area measures its own lean with at least this many of them; else the view's is used. */
const LEAN_MIN_BUILDINGS = 4;
/** The areas lean is measured over: zoom-14 tiles (about 1.4 km at 55°N). */
const LEAN_ZOOM = 14;

type TileKey = string;
interface Tex {
  tex: WebGLTexture | null;
  img: HTMLImageElement;
  ready: boolean;
  used: number;
  /** Edge strength per pixel, spread to each of `RADII`, for measuring the lean (null: not readable). */
  edges?: Uint8Array[] | null;
}

interface Roof {
  /** Outline and holes relative to the origin, mercator; triangles over them. */
  flat: number[];
  tri: number[];
  /** Roof altitude, mercator units above sea level. */
  z: number;
  h: number;
  pm: number;
  ring: [number, number][];
  box: [number, number, number, number];
  centre: [number, number];
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
  /** The view's lean, and each area's that measured its own (key: z/x/y at zoom 14). */
  view: Lean | null;
  areas: Record<string, Lean>;
  /** Buildings that took a share of the lean other than their height's. */
  rescaled: number;
}

/** Most buildings a lean is measured on (the tallest): enough, and quick. */
const LEAN_MAX_BUILDINGS = 60;

/** The tallest of them, at most `LEAN_MAX_BUILDINGS`. */
function tallest(bs: LeanBuilding[]): LeanBuilding[] {
  return bs.length <= LEAN_MAX_BUILDINGS ? bs : [...bs].sort((a, b) => b.h - a.h).slice(0, LEAN_MAX_BUILDINGS);
}

export type RoofLayer = maplibregl.CustomLayerInterface & {
  stats: RoofStats;
  /** Correct for the imagery's lean (on by default). */
  lean: boolean;
  /** Measure and draw again. */
  refresh(): void;
};

/** The custom layer. Add it after the building blocks. */
export function roofLayer(id: string, o: RoofOptions): RoofLayer {
  let map: maplibregl.Map | null = null;
  let gl: WebGLRenderingContext | WebGL2RenderingContext | null = null;
  let program: WebGLProgram | null = null;
  let vbo: WebGLBuffer | null = null;
  let ibo: WebGLBuffer | null = null;
  let loc: { pos: number; shift: number; matrix: WebGLUniformLocation | null; z: WebGLUniformLocation | null; image: WebGLUniformLocation | null; tileMin: WebGLUniformLocation | null; scale: WebGLUniformLocation | null; brightness: WebGLUniformLocation | null } | null = null;
  // Built geometry: positions relative to `origin` (mercator), and per-tile index runs.
  let origin: [number, number] = [0, 0];
  let zoom = 0;
  let roofs: Roof[] = [];
  let runs: { key: TileKey; x: number; y: number; start: number; count: number }[] = [];
  let built = false;
  let frame = 0;
  let uint32 = false;
  /** Whether the built indices are 32-bit. */
  let wide = false;
  const textures = new Map<TileKey, Tex>();
  let timer: ReturnType<typeof setTimeout> | null = null;
  let leanTimer: ReturnType<typeof setTimeout> | null = null;
  /** Each roof's shift (mercator), from the lean measured so far. */
  let shifts: [number, number][] = [];
  /** Leans and buildings' shares measured so far, by zoom and place. */
  const memo = new Map<string, Lean | number>();
  const stats: RoofStats = { roofs: 0, view: null, areas: {}, rescaled: 0 };

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
    t = { tex: null, img, ready: false, used: frame };
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
      // Mipmaps: roofs seen at a slant stay smooth (tiles are 256², a power of two).
      g.generateMipmap(g.TEXTURE_2D);
      g.texParameteri(g.TEXTURE_2D, g.TEXTURE_MIN_FILTER, g.LINEAR_MIPMAP_LINEAR);
      entry.ready = true;
      map?.triggerRepaint();
      // More imagery to measure the lean on.
      if (leanTimer) clearTimeout(leanTimer);
      leanTimer = setTimeout(measure, 250);
    };
    img.onerror = () => {
      entry.ready = false;
    };
    img.src = url(z, x, y);
    textures.set(key, t);
    // Keep the most recently used tiles; let the rest go.
    if (textures.size > o.maxTiles * 2) {
      const old = [...textures.entries()].sort((a, b) => a[1].used - b[1].used).slice(0, textures.size - o.maxTiles * 2);
      for (const [k, v] of old) {
        if (v.tex && gl) gl.deleteTexture(v.tex);
        v.img.onload = null;
        textures.delete(k);
      }
    }
    return t;
  }

  /** The imagery's edges at a mercator point (NaN where not loaded or not readable). */
  let last: { key: number; edges: Uint8Array[] | null } = { key: -1, edges: null };
  const edge: EdgeMap = (mx, my, level) => {
    const n = 2 ** zoom;
    const fx = mx * n;
    const fy = my * n;
    const tx = Math.floor(fx);
    const ty = Math.floor(fy);
    // Most reads fall in the tile the last one did.
    const key = tx * 1048576 + ty;
    let edges = last.key === key ? last.edges : null;
    if (last.key !== key) {
      const t = textures.get(`${zoom}/${tx}/${ty}`);
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

  /** Roofs for the buildings loaded now. */
  function build() {
    if (!map || !gl) return;
    const m = map;
    built = false;
    roofs = [];
    runs = [];
    if (m.getZoom() < 14.5 || !m.getSource(o.source)) {
      m.triggerRepaint();
      return;
    }
    // Imagery about as sharp as the ground's at this zoom (256-pixel tiles).
    zoom = Math.max(15, Math.min(19, Math.round(m.getZoom()) + 1));
    const cm = maplibregl.MercatorCoordinate.fromLngLat(m.getCenter());
    origin = [cm.x, cm.y];
    const camEle = camElevation();
    const bounds = m.getBounds();
    for (const f of m.querySourceFeatures(o.source, { sourceLayer: o.sourceLayer })) {
      const g = f.geometry;
      const polys = g.type === "Polygon" ? [g.coordinates] : g.type === "MultiPolygon" ? g.coordinates : [];
      const h = heightOf(f.properties ?? {});
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
        if (cx < bounds.getWest() - 0.01 || cx > bounds.getEast() + 0.01 || cy < bounds.getSouth() - 0.01 || cy > bounds.getNorth() + 0.01) continue;
        const ground = (m.queryTerrainElevation([cx, cy]) ?? 0) + camEle;
        const pm = perMetre(cy);
        const flat: number[] = [];
        const holes: number[] = [];
        const ring: [number, number][] = [];
        const box: [number, number, number, number] = [Infinity, Infinity, -Infinity, -Infinity];
        for (let r = 0; r < rings.length; r++) {
          if (r > 0) holes.push(flat.length / 2);
          // The closing point repeats the first: left out.
          for (let k = 0; k < rings[r].length - 1; k++) {
            const mc = maplibregl.MercatorCoordinate.fromLngLat([rings[r][k][0], rings[r][k][1]]);
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
        const centre = maplibregl.MercatorCoordinate.fromLngLat([cx, cy]);
        roofs.push({ flat, tri, z: (ground + h + LIFT_M) * pm, h, pm, ring, box, centre: [centre.x, centre.y] });
      }
    }
    stats.roofs = roofs.length;
    shifts = roofs.map(() => [0, 0]);
    // The imagery the tall buildings' roofs could have been photographed in, to measure the lean on.
    const n = 2 ** zoom;
    for (const r of roofs) {
      if (r.h < LEAN_MIN_H) continue;
      const reach = MAX_K * r.h * r.pm;
      for (let tx = Math.floor((r.box[0] - reach) * n); tx <= Math.floor((r.box[2] + reach) * n); tx++) {
        for (let ty = Math.floor((r.box[1] - reach) * n); ty <= Math.floor((r.box[3] + reach) * n); ty++) texture(`${zoom}/${tx}/${ty}`, zoom, tx, ty);
      }
    }
    measure();
  }

  /** The lean, by area, from the imagery loaded so far; then the roofs drawn with it. */
  function measure() {
    if (!map || !gl || !roofs.length) return;
    last = { key: -1, edges: null };
    const tall: { i: number; b: LeanBuilding; area: string }[] = [];
    const na = 2 ** LEAN_ZOOM;
    roofs.forEach((r, i) => {
      if (r.h >= LEAN_MIN_H) tall.push({ i, b: { ring: r.ring, h: r.h, pm: r.pm }, area: `${LEAN_ZOOM}/${Math.floor(r.centre[0] * na)}/${Math.floor(r.centre[1] * na)}` });
    });
    // A pixel of the imagery, mercator.
    const px = 1 / (2 ** zoom * 256);
    const off = !self.lean;
    const groups = new Map<string, LeanBuilding[]>();
    for (const t of tall) groups.set(t.area, [...(groups.get(t.area) ?? []), t.b]);
    if (memo.size > 20000) memo.clear();
    // What has been measured with enough imagery is kept: a pan or more
    // imagery measures only what is new.
    const confident = (l: Lean) => l.used >= LEAN_MIN_BUILDINGS && l.gain >= MIN_GAIN;
    const viewKey = `${zoom}|view|${[...groups.keys()].sort().join(",")}`;
    let view: Lean | null = null;
    if (tall.length && !off) {
      view = (memo.get(viewKey) as Lean | undefined) ?? estimateLean(tallest(tall.map((t) => t.b)), edge, px);
      if (confident(view)) memo.set(viewKey, view);
    }
    const areas: Record<string, Lean> = {};
    for (const [a, bs] of groups) {
      if (off || bs.length < LEAN_MIN_BUILDINGS) continue;
      const key = `${zoom}|area|${a}`;
      const l = (memo.get(key) as Lean | undefined) ?? estimateLean(tallest(bs), edge, px);
      // An area with too little imagery or contrast keeps the view's lean.
      if (confident(l)) {
        areas[a] = l;
        memo.set(key, l);
      }
    }
    stats.view = view;
    stats.areas = areas;
    let rescaled = 0;
    shifts = roofs.map((r) => {
      const a = `${LEAN_ZOOM}/${Math.floor(r.centre[0] * na)}/${Math.floor(r.centre[1] * na)}`;
      const k = (areas[a] ?? view)?.k ?? [0, 0];
      if (k[0] === 0 && k[1] === 0) return [0, 0];
      // A tall building's own share, where its outline says its height is off.
      let s = 1;
      if (r.h >= 8) {
        const key = `${zoom}|scale|${k[0].toFixed(3)},${k[1].toFixed(3)}|${r.ring[0][0]},${r.ring[0][1]}|${r.h}`;
        const known = memo.get(key) as number | undefined;
        if (known !== undefined) s = known;
        else {
          const b = { ring: r.ring, h: r.h, pm: r.pm };
          s = refineScale(b, k, edge, px);
          // Kept once its imagery is in.
          if (Number.isFinite(edgeScore(b.ring, 0, 0, edge, px))) memo.set(key, s);
        }
      }
      if (s !== 1) rescaled++;
      const m = r.h * r.pm * s;
      return [k[0] * m, k[1] * m];
    });
    stats.rescaled = rescaled;
    upload();
  }

  /** The roofs into the GPU's buffers, grouped by the imagery tile they read from. */
  function upload() {
    if (!map || !gl) return;
    const m = map;
    const n = 2 ** zoom;
    const cx = origin[0];
    const cy = origin[1];
    const data: number[] = [];
    const byTile = new Map<TileKey, { x: number; y: number; idx: number[]; d: number }>();
    roofs.forEach((r, i) => {
      const [sx, sy] = shifts[i] ?? [0, 0];
      const base = data.length / 5;
      for (let k = 0; k < r.flat.length; k += 2) data.push(r.flat[k], r.flat[k + 1], r.z, sx, sy);
      // Every tile its (shifted) roof reads from draws its part of it.
      for (let tx = Math.floor((r.box[0] + sx) * n); tx <= Math.floor((r.box[2] + sx) * n); tx++) {
        for (let ty = Math.floor((r.box[1] + sy) * n); ty <= Math.floor((r.box[3] + sy) * n); ty++) {
          const key = `${zoom}/${tx}/${ty}`;
          let t = byTile.get(key);
          if (!t) {
            t = { x: tx, y: ty, idx: [], d: Math.hypot((tx + 0.5) / n - cx, (ty + 0.5) / n - cy) };
            byTile.set(key, t);
          }
          for (const v of r.tri) t.idx.push(base + v);
        }
      }
    });
    // The tiles nearest the middle of the view, as many as are allowed.
    const tiles = [...byTile.entries()].sort((a, b) => a[1].d - b[1].d).slice(0, o.maxTiles);
    const total = tiles.reduce((s, [, t]) => s + t.idx.length, 0);
    const vertices = data.length / 5;
    runs = [];
    built = false;
    if (!total || (vertices > 65535 && !uint32)) {
      // Nothing to draw (or, on WebGL 1 without 32-bit indices, too many roofs at once).
      m.triggerRepaint();
      return;
    }
    wide = vertices > 65535;
    const indices = wide ? new Uint32Array(total) : new Uint16Array(total);
    let at = 0;
    for (const [key, t] of tiles) {
      indices.set(t.idx, at);
      runs.push({ key, x: t.x, y: t.y, start: at, count: t.idx.length });
      at += t.idx.length;
    }
    const g = gl;
    g.bindBuffer(g.ARRAY_BUFFER, vbo);
    g.bufferData(g.ARRAY_BUFFER, new Float32Array(data), g.DYNAMIC_DRAW);
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ibo);
    g.bufferData(g.ELEMENT_ARRAY_BUFFER, indices, g.DYNAMIC_DRAW);
    built = true;
    m.triggerRepaint();
  }

  const later = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(build, 120);
  };

  const self: RoofLayer = {
    id,
    type: "custom",
    renderingMode: "3d",
    stats,
    lean: true,
    refresh: () => measure(),
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
      vbo = g.createBuffer();
      ibo = g.createBuffer();
      // New buildings or terrain loaded, or the view settled: roofs again.
      m.on("idle", later);
      m.on("moveend", later);
      later();
    },
    onRemove(_m, g) {
      map?.off("idle", later);
      map?.off("moveend", later);
      if (timer) clearTimeout(timer);
      if (leanTimer) clearTimeout(leanTimer);
      for (const t of textures.values()) {
        if (t.tex) g.deleteTexture(t.tex);
        t.img.onload = null;
      }
      textures.clear();
      if (vbo) g.deleteBuffer(vbo);
      if (ibo) g.deleteBuffer(ibo);
      if (program) g.deleteProgram(program);
      map = null;
      gl = null;
    },
    render(g, args) {
      if (!built || !program || !loc || !map) return;
      frame++;
      const matrix = (Array.isArray(args) || ArrayBuffer.isView(args) ? args : (args as { defaultProjectionData?: { mainMatrix: ArrayLike<number> } }).defaultProjectionData?.mainMatrix) as ArrayLike<number>;
      // The matrix with the origin moved in (in double precision, then to the GPU).
      const mm = Array.from(matrix);
      const [ox, oy] = origin;
      const shifted = new Float32Array(16);
      for (let r = 0; r < 4; r++) {
        for (let c = 0; c < 4; c++) shifted[c * 4 + r] = mm[c * 4 + r];
        shifted[12 + r] = mm[r] * ox + mm[4 + r] * oy + mm[12 + r];
      }
      g.useProgram(program);
      g.uniformMatrix4fv(loc.matrix, false, shifted);
      // Heights are stored above sea level; the matrix is about the camera's ground.
      g.uniform1f(loc.z, -camElevation() * perMetre(map.getCenter().lat));
      g.uniform1f(loc.scale, 2 ** zoom);
      g.uniform1f(loc.brightness, o.brightness);
      g.uniform1i(loc.image, 0);
      g.bindBuffer(g.ARRAY_BUFFER, vbo);
      g.enableVertexAttribArray(loc.pos);
      g.vertexAttribPointer(loc.pos, 3, g.FLOAT, false, 20, 0);
      g.enableVertexAttribArray(loc.shift);
      g.vertexAttribPointer(loc.shift, 2, g.FLOAT, false, 20, 12);
      g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ibo);
      g.disable(g.CULL_FACE);
      g.disable(g.BLEND);
      g.activeTexture(g.TEXTURE0);
      const type = wide ? g.UNSIGNED_INT : g.UNSIGNED_SHORT;
      const bytes = wide ? 4 : 2;
      const n = 2 ** zoom;
      for (const r of runs) {
        const t = texture(r.key, zoom, r.x, r.y);
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
