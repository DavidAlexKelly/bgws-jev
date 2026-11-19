// Satellite roofs: in 3D over satellite imagery, each building's top shows
// the imagery of its own roof rather than a flat colour.
//
// A MapLibre custom layer, so it shares the map's depth buffer: roofs hide
// behind nearer buildings and hills as the blocks do. The roofs are the
// buildings MapLibre has loaded for the view (the pack's basemap), each
// triangulated at its top: the building's height above the terrain at its
// centroid, as MapLibre places its blocks, and a little above, so the roof
// covers the block's plain top. Each roof is drawn once for every imagery
// tile it touches, with that tile as its texture and anything outside the
// tile cut away, so a roof across a tile edge joins up.
//
// The imagery is a photograph from above, not a true orthophoto: the roof of
// a tall building can sit a few metres off its footprint, so its edge may
// show some ground or wall.

import earcut from "earcut";
import maplibregl from "maplibre-gl";

/** Height of a building as the blocks draw it (see `buildingBlocks`). */
function heightOf(p: Record<string, unknown>): number {
  const h = Number(p.height ?? p.est_height ?? 6);
  return Number.isFinite(h) && h > 0 ? h : 6;
}

/** Raised this far over the block's top, m, so the roof covers it. */
const LIFT_M = 0.25;

type TileKey = string;
interface Tex {
  tex: WebGLTexture | null;
  img: HTMLImageElement;
  ready: boolean;
  used: number;
}

const VS = `
uniform mat4 u_matrix;
uniform float u_z_offset;
attribute vec3 a_pos;
varying vec2 v_pos;
void main() {
  v_pos = a_pos.xy;
  gl_Position = u_matrix * vec4(a_pos.xy, a_pos.z + u_z_offset, 1.0);
}`;

const FS = `
precision highp float;
uniform sampler2D u_image;
uniform vec2 u_tile_min;
uniform float u_scale;
uniform float u_brightness;
varying vec2 v_pos;
void main() {
  vec2 uv = (v_pos - u_tile_min) * u_scale;
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

/** The custom layer. Add it after the building blocks. */
export function roofLayer(id: string, o: RoofOptions): maplibregl.CustomLayerInterface {
  let map: maplibregl.Map | null = null;
  let gl: WebGLRenderingContext | WebGL2RenderingContext | null = null;
  let program: WebGLProgram | null = null;
  let vbo: WebGLBuffer | null = null;
  let ibo: WebGLBuffer | null = null;
  let loc: { pos: number; matrix: WebGLUniformLocation | null; z: WebGLUniformLocation | null; image: WebGLUniformLocation | null; tileMin: WebGLUniformLocation | null; scale: WebGLUniformLocation | null; brightness: WebGLUniformLocation | null } | null = null;
  // Built geometry: positions relative to `origin` (mercator), and per-tile index runs.
  let origin: [number, number] = [0, 0];
  let zoom = 0;
  let runs: { key: TileKey; x: number; y: number; start: number; count: number }[] = [];
  let built = false;
  let frame = 0;
  let uint32 = false;
  /** Whether the built indices are 32-bit. */
  let wide = false;
  const textures = new Map<TileKey, Tex>();
  let timer: ReturnType<typeof setTimeout> | null = null;

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
    };
    img.onerror = () => {
      entry.ready = false;
    };
    img.src = url(z, x, y);
    textures.set(key, t);
    // Keep the most recently drawn tiles; let the rest go.
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

  /** Roofs for the buildings loaded now, grouped by the imagery tile they fall in. */
  function build() {
    if (!map || !gl) return;
    const m = map;
    built = false;
    runs = [];
    if (m.getZoom() < 14.5 || !m.getSource(o.source)) {
      m.triggerRepaint();
      return;
    }
    // Imagery about as sharp as the ground's at this zoom (256-pixel tiles).
    zoom = Math.max(15, Math.min(19, Math.round(m.getZoom()) + 1));
    const n = 2 ** zoom;
    const c = m.getCenter();
    const cm = maplibregl.MercatorCoordinate.fromLngLat(c);
    origin = [cm.x, cm.y];
    const camEle = camElevation();
    const bounds = m.getBounds();
    const feats = m.querySourceFeatures(o.source, { sourceLayer: o.sourceLayer });
    const pos: number[] = [];
    const byTile = new Map<TileKey, { x: number; y: number; idx: number[]; d: number }>();
    for (const f of feats) {
      const g = f.geometry;
      const polys = g.type === "Polygon" ? [g.coordinates] : g.type === "MultiPolygon" ? g.coordinates : [];
      const top = heightOf(f.properties ?? {}) + LIFT_M;
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
        const z = (ground + top) * perMetre(cy);
        const flat: number[] = [];
        const holes: number[] = [];
        let minX = Infinity;
        let minY = Infinity;
        let maxX = -Infinity;
        let maxY = -Infinity;
        for (let r = 0; r < rings.length; r++) {
          const ring = rings[r];
          if (r > 0) holes.push(flat.length / 2);
          // The closing point repeats the first: left out for the triangulation.
          for (let k = 0; k < ring.length - 1; k++) {
            const mc = maplibregl.MercatorCoordinate.fromLngLat([ring[k][0], ring[k][1]]);
            flat.push(mc.x - origin[0], mc.y - origin[1]);
            if (r === 0) {
              minX = Math.min(minX, mc.x);
              maxX = Math.max(maxX, mc.x);
              minY = Math.min(minY, mc.y);
              maxY = Math.max(maxY, mc.y);
            }
          }
        }
        const tri = earcut(flat, holes.length ? holes : undefined, 2);
        if (!tri.length) continue;
        const base = pos.length / 3;
        for (let k = 0; k < flat.length; k += 2) pos.push(flat[k], flat[k + 1], z);
        // Every imagery tile the roof touches draws its part of it.
        for (let tx = Math.floor(minX * n); tx <= Math.floor(maxX * n); tx++) {
          for (let ty = Math.floor(minY * n); ty <= Math.floor(maxY * n); ty++) {
            const key = `${zoom}/${tx}/${ty}`;
            let t = byTile.get(key);
            if (!t) {
              t = { x: tx, y: ty, idx: [], d: Math.hypot((tx + 0.5) / n - cm.x, (ty + 0.5) / n - cm.y) };
              byTile.set(key, t);
            }
            for (const i of tri) t.idx.push(base + i);
          }
        }
      }
    }
    // The tiles nearest the middle of the view, as many as are allowed.
    const tiles = [...byTile.entries()].sort((a, b) => a[1].d - b[1].d).slice(0, o.maxTiles);
    const total = tiles.reduce((s, [, t]) => s + t.idx.length, 0);
    if (!total) {
      m.triggerRepaint();
      return;
    }
    const vertices = pos.length / 3;
    if (vertices > 65535 && !uint32) {
      // WebGL 1 without 32-bit indices: too many roofs to draw at once here.
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
    g.bufferData(g.ARRAY_BUFFER, new Float32Array(pos), g.DYNAMIC_DRAW);
    g.bindBuffer(g.ELEMENT_ARRAY_BUFFER, ibo);
    g.bufferData(g.ELEMENT_ARRAY_BUFFER, indices, g.DYNAMIC_DRAW);
    built = true;
    m.triggerRepaint();
  }

  const later = () => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(build, 120);
  };

  return {
    id,
    type: "custom",
    renderingMode: "3d",
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
      g.vertexAttribPointer(loc.pos, 3, g.FLOAT, false, 0, 0);
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
    },
  };
}
