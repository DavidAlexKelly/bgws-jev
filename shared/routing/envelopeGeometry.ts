/**
 * Concave-hull envelope geometry utilities.
 *
 * Builds movement-envelope polygons from reachable graph nodes so they can be
 * rendered on a Mapbox / Maplibre layer.
 */

/* ------------------------------------------------------------------ */
/*  Inline GeoJSON types (no @types/geojson dependency)               */
/* ------------------------------------------------------------------ */

export interface GeoJSONPolygonFeature {
  type: 'Feature';
  properties: Record<string, unknown>;
  geometry: { type: 'Polygon'; coordinates: [number, number][][] };
}

/* ------------------------------------------------------------------ */
/*  Chaikin corner-cutting smoothing                                  */
/* ------------------------------------------------------------------ */

/**
 * Apply one pass of Chaikin corner-cutting to a ring of 2-D points.
 * Each edge AB is replaced by two points at 25 % and 75 % along AB.
 */
function chaikinPass(ring: [number, number][]): [number, number][] {
  const out: [number, number][] = [];
  for (let i = 0; i < ring.length - 1; i++) {
    const [ax, ay] = ring[i];
    const [bx, by] = ring[i + 1];
    out.push([ax + 0.25 * (bx - ax), ay + 0.25 * (by - ay)]);
    out.push([ax + 0.75 * (bx - ax), ay + 0.75 * (by - ay)]);
  }
  // Close the ring
  out.push(out[0]);
  return out;
}

/**
 * Apply `passes` iterations of Chaikin corner-cutting smoothing.
 */
export function chaikinSmooth(
  ring: [number, number][],
  passes: number = 2,
): [number, number][] {
  let result = ring;
  for (let p = 0; p < passes; p++) {
    result = chaikinPass(result);
  }
  return result;
}

/* ------------------------------------------------------------------ */
/*  computeConcaveHull                                                */
/* ------------------------------------------------------------------ */

/**
 * Build a concave-hull polygon from a set of reachable graph nodes.
 *
 * Algorithm (sector-based boundary extraction + smoothing):
 * 1. Find centroid of all nodes.
 * 2. Divide 360° into 72 angular sectors (5° each) from centroid.
 * 3. For each sector pick the FURTHEST node → forms outer boundary.
 * 4. Fill empty sectors by interpolating between nearest populated neighbours.
 * 5. Apply 2 passes of Chaikin corner-cutting smoothing.
 * 6. Close ring, return as GeoJSON Feature<Polygon>.
 */
export function computeConcaveHull(
  nodes: Array<{ lat: number; lon: number; costSec: number }>,
  maxCostSec: number,
): GeoJSONPolygonFeature {
  const SECTORS = 72; // 5° each
  const TWO_PI = Math.PI * 2;

  // ---------- 1. Centroid ----------
  let cLat = 0;
  let cLon = 0;
  for (const n of nodes) {
    cLat += n.lat;
    cLon += n.lon;
  }
  cLat /= nodes.length;
  cLon /= nodes.length;

  // ---------- 2 & 3. Sector buckets – keep furthest node ----------
  const buckets: ([number, number] | null)[] = new Array(SECTORS).fill(null);
  const bucketDist: number[] = new Array(SECTORS).fill(0);

  for (const n of nodes) {
    const dx = n.lon - cLon;
    const dy = n.lat - cLat;
    const dist = Math.sqrt(dx * dx + dy * dy);
    let angle = Math.atan2(dy, dx); // radians, –π..π
    if (angle < 0) angle += TWO_PI;
    const idx = Math.min(Math.floor((angle / TWO_PI) * SECTORS), SECTORS - 1);
    if (dist > bucketDist[idx]) {
      buckets[idx] = [n.lon, n.lat];
      bucketDist[idx] = dist;
    }
  }

  // ---------- 4. Fill empty sectors by interpolation ----------
  for (let i = 0; i < SECTORS; i++) {
    if (buckets[i] !== null) continue;

    // Find nearest populated neighbours (wrapping)
    let prevIdx = -1;
    let nextIdx = -1;
    for (let d = 1; d < SECTORS; d++) {
      if (prevIdx === -1 && buckets[(i - d + SECTORS) % SECTORS] !== null) {
        prevIdx = (i - d + SECTORS) % SECTORS;
      }
      if (nextIdx === -1 && buckets[(i + d) % SECTORS] !== null) {
        nextIdx = (i + d) % SECTORS;
      }
      if (prevIdx !== -1 && nextIdx !== -1) break;
    }

    if (prevIdx !== -1 && nextIdx !== -1) {
      const prev = buckets[prevIdx]!;
      const next = buckets[nextIdx]!;

      // Distance (in sectors) from prevIdx → i and prevIdx → nextIdx (wrapping)
      const spanTotal =
        ((nextIdx - prevIdx + SECTORS) % SECTORS) || SECTORS;
      const spanToI = ((i - prevIdx + SECTORS) % SECTORS) || SECTORS;
      const t = spanToI / spanTotal;

      buckets[i] = [
        prev[0] + t * (next[0] - prev[0]),
        prev[1] + t * (next[1] - prev[1]),
      ];
    }
  }

  // Collect non-null boundary points and close ring
  let ring: [number, number][] = [];
  for (let i = 0; i < SECTORS; i++) {
    if (buckets[i] !== null) ring.push(buckets[i]!);
  }
  if (ring.length > 0) {
    ring.push(ring[0]); // close
  }

  // ---------- 5. Smooth ----------
  ring = chaikinSmooth(ring, 2);

  // ---------- 6. Return GeoJSON ----------
  return {
    type: 'Feature',
    properties: { maxCostSec },
    geometry: {
      type: 'Polygon',
      coordinates: [ring],
    },
  };
}

/* ------------------------------------------------------------------ */
/*  createReachabilityMask                                            */
/* ------------------------------------------------------------------ */

/**
 * Create an inverted polygon that covers the entire world EXCEPT the
 * provided envelope.  Useful for darkening / dimming unreachable areas
 * on the map.
 *
 * The world ring is the outer boundary; the envelope (reversed) is
 * punched out as an interior hole.
 */
export function createReachabilityMask(
  envelope: [number, number][][],
): GeoJSONPolygonFeature {
  const worldRing: [number, number][] = [
    [-180, -90],
    [180, -90],
    [180, 90],
    [-180, 90],
    [-180, -90],
  ];

  // The envelope's first ring reversed becomes the hole
  const hole: [number, number][] = [...envelope[0]].reverse();

  return {
    type: 'Feature',
    properties: {},
    geometry: {
      type: 'Polygon',
      coordinates: [worldRing, hole],
    },
  };
}

/* ------------------------------------------------------------------ */
/*  createCirclePolygon                                               */
/* ------------------------------------------------------------------ */

/**
 * Generate a circle polygon for air-unit movement envelopes where
 * graph-based routing is unnecessary.
 *
 * @param center  [lon, lat]
 * @param radiusKm  radius in kilometres
 * @param segments  number of polygon segments (higher = smoother)
 */
export function createCirclePolygon(
  center: [number, number],
  radiusKm: number,
  segments: number,
): GeoJSONPolygonFeature {
  const [lon, lat] = center;
  const DEG_PER_KM = 1 / 111.32;
  const cosLat = Math.cos((lat * Math.PI) / 180);

  const ring: [number, number][] = [];
  for (let i = 0; i <= segments; i++) {
    const angle = (2 * Math.PI * i) / segments;
    const dy = radiusKm * DEG_PER_KM * Math.sin(angle);
    const dx = (radiusKm / (111.32 * cosLat)) * Math.cos(angle);
    ring.push([lon + dx, lat + dy]);
  }
  // Ensure closure
  ring[ring.length - 1] = ring[0];

  return {
    type: 'Feature',
    properties: {},
    geometry: {
      type: 'Polygon',
      coordinates: [ring],
    },
  };
}
