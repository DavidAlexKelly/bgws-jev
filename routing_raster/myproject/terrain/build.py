"""Build the terrain, slope and links planes for one 2-degree cell.

Three planes, each holding only what a pixel can actually represent:

  terrain.bin  the ground under a pixel: open, rough, very rough, building,
               water areas (lakes, wide rivers), nodata. No roads, and no thin
               rivers: neither is as wide as a pixel.
  slope.bin    unchanged.
  links.bin    the moves between neighbouring pixels: river WALLS and road
               LINKS (see links.py).

Rivers and roads used to share the terrain plane, so wherever they ran side
by side one of them had to lose its pixels: painting roads last cut holes in
the river, and painting water last cut the road. On separate planes nothing
competes, and the only place a road meets a river is where it really
crosses it.
"""

from __future__ import annotations

import json
import math
from dataclasses import dataclass
from typing import Any, Dict, List, Sequence, Tuple

import numpy as np
import rasterio
import shapely
from rasterio import features as rio_features
from rasterio.transform import from_bounds
from rasterio.warp import Resampling, reproject
from scipy import ndimage

from myproject.terrain import classes as C
from myproject.terrain import grid
from myproject.terrain import links as L

LAYERS = ("water", "landuse", "buildings", "building", "roads")

FORMAT = 2

SLOPE_NODATA = 255
SLOPE_SCALE = 250.0

# Pixels of margin traced around the cell for walls and roads. Moves near
# the edge are decided by geometry just across it, and two neighbouring
# cells trace the same geometry the same way, so their roads meet.
PAD_PX = 4


@dataclass
class BuiltCell:
    terrain: np.ndarray
    slope: np.ndarray
    links: np.ndarray
    width: int
    height: int
    pixel_m: float
    counts: Dict[str, int]
    wall_edges: int
    road_parts: int
    road_links: int
    road_pixels: int
    road_water_pixels: int
    road_crossings: int
    road_forced: int
    water_specks_cleared: int


def read_bounds(bounds: Tuple[float, float, float, float], pixel_m: float) -> Tuple[float, float, float, float]:
    """The cell grown by the traced margin plus one pixel: read features over this."""
    west, south, east, north = bounds
    height, width, _, _ = grid.shape_for_bounds(west, south, east, north, pixel_m)
    grow_x = (east - west) / width * (PAD_PX + 1)
    grow_y = (north - south) / height * (PAD_PX + 1)
    return (west - grow_x, south - grow_y, east + grow_x, north + grow_y)


def build_cell(
    key: str,
    bounds: Tuple[float, float, float, float],
    features: Sequence[Dict[str, Any]],
    dem_path: str,
    pixel_m: float,
) -> BuiltCell:
    west, south, east, north = bounds
    lat_c = 0.5 * (south + north)

    elev, transform, width, height = _load_elev_grid(dem_path, west, south, east, north, pixel_m)
    slope = _encode_slope(_slope_from_elev(elev, pixel_m))

    water_polys: List[Any] = []
    water_lines: List[Any] = []
    landuse_by_class: Dict[int, List[Any]] = {
        C.CLASS_NUMBER[C.ROUGH]: [],
        C.CLASS_NUMBER[C.VERY_ROUGH]: [],
        C.CLASS_NUMBER[C.OPEN]: [],
    }
    building_polys: List[Any] = []
    road_lines: List[Any] = []

    for feat in features:
        layer = feat["layer"]
        props = feat.get("props") or {}
        geom = feat["geom"]
        if geom is None or geom.is_empty:
            continue

        if layer == "water":
            _collect_water(geom, props, water_polys, water_lines, lat_c, pixel_m)
            continue

        if layer == "landuse":
            name = C.class_for_landuse(props)
            if name is None:
                continue
            cid = C.CLASS_NUMBER[name]
            if cid in landuse_by_class:
                _append_poly(landuse_by_class[cid], geom)
            continue

        if layer in ("buildings", "building"):
            kind = props.get("kind")
            if isinstance(kind, str) and kind.lower() in C.BUILDING_KINDS:
                _append_poly(building_polys, geom)
            continue

        if layer == "roads" and C.is_road_surface(props):
            # Bridge, tunnel and ford tags are not needed: a road may cross
            # water wherever its geometry crosses water (links.py).
            road_lines.extend(_as_lines(geom))

    # ── terrain: the ground under each pixel ────────────────────────────────
    t_nodata = C.CLASS_NUMBER[C.NODATA]
    t_water = C.CLASS_NUMBER[C.WATER]
    t_building = C.CLASS_NUMBER[C.BUILDING]
    t_open = C.CLASS_NUMBER[C.OPEN]

    terrain = np.full((height, width), t_nodata, dtype=np.uint8)
    terrain[np.isfinite(elev)] = t_open

    for cid, polys in landuse_by_class.items():
        if cid != t_open:
            _burn(terrain, polys, cid, transform)

    # Water areas by pixel centre, with no dilation: thin water no longer has
    # to be wide enough to block, because its outline is a wall.
    water_mask = _rasterize_mask(water_polys, transform, height, width, all_touched=False)
    water_specks_cleared = _clear_tiny_components(water_mask, max_px=2)
    terrain[water_mask > 0] = t_water

    _burn(terrain, building_polys, t_building, transform)
    terrain[~np.isfinite(elev)] = t_nodata

    # ── links: river walls and road links, on a padded lattice ─────────────
    to_lattice = _lattice_transform(bounds, width, height, PAD_PX)
    pad_w, pad_h = width + 2 * PAD_PX, height + 2 * PAD_PX
    box = (-2.0, -2.0, pad_w + 1.0, pad_h + 1.0)

    wall_sources = list(water_lines) + [shapely.boundary(p) for p in water_polys]
    wall_parts = _lattice_parts(wall_sources, to_lattice, box)
    road_parts = _lattice_parts(road_lines, to_lattice, box)

    padded = np.zeros((pad_h, pad_w), dtype=np.uint8)
    L.trace_walls(padded, L.segments_of(wall_parts))
    report = L.trace_roads(L.LinkGrid(padded), road_parts, wall_parts)

    padded_road = L.road_nodes(padded)
    inner = (slice(PAD_PX, PAD_PX + height), slice(PAD_PX, PAD_PX + width))
    links = np.ascontiguousarray(padded[inner])
    road_mask = padded_road[inner]

    # A road over a small DEM gap stays usable: give it the flat slope.
    slope[road_mask & (slope == SLOPE_NODATA)] = 0

    histogram = np.bincount(terrain.ravel(), minlength=256)
    counts = {name: int(histogram[int(number)]) for number, name in C.TERRAIN_CLASSES.items()}

    road_bits = links & L.ROAD_BITS
    return BuiltCell(
        terrain=terrain,
        slope=slope,
        links=links,
        width=width,
        height=height,
        pixel_m=pixel_m,
        counts=counts,
        wall_edges=int(np.count_nonzero(links & L.WALL_E)) + int(np.count_nonzero(links & L.WALL_N)),
        road_parts=report.roads,
        road_links=int(sum(np.count_nonzero(road_bits & b) for b in (L.ROAD_E, L.ROAD_N, L.ROAD_NE, L.ROAD_NW))),
        road_pixels=int(np.count_nonzero(road_mask)),
        road_water_pixels=int(np.count_nonzero(road_mask & (water_mask > 0))),
        road_crossings=report.crossings,
        road_forced=report.forced,
        water_specks_cleared=water_specks_cleared,
    )


def meta_for(key: str, built: BuiltCell) -> str:
    return (
        json.dumps(
            {
                "cell": key,
                "format": FORMAT,
                "terrainClasses": C.TERRAIN_CLASSES,
                "pixel_m": built.pixel_m,
                "width": built.width,
                "height": built.height,
                # Binary planes are flipped by raster_chunks._build_one before
                # writing, because clients index rows from the south edge.
                "rowOrder": "south-to-north",
                "terrain": {
                    "file": "terrain.bin",
                    "dtype": "uint8",
                    "note": "Ground only. Roads are in links.bin, never in this plane.",
                },
                "slope": {
                    "file": "slope.bin",
                    "dtype": "uint8",
                    "encoding": "slope_01 = value / 250; 255 = nodata",
                    "scale": SLOPE_SCALE,
                    "nodata": SLOPE_NODATA,
                },
                "links": {
                    "file": "links.bin",
                    "dtype": "uint8",
                    "bits": {
                        "wallE": L.WALL_E,
                        "wallN": L.WALL_N,
                        "roadE": L.ROAD_E,
                        "roadN": L.ROAD_N,
                        "roadNE": L.ROAD_NE,
                        "roadNW": L.ROAD_NW,
                    },
                    "rules": [
                        "Each move between neighbours is stored once, on the pixel it leaves going east or north.",
                        "An off-road orthogonal move is blocked by a wall on it.",
                        "An off-road diagonal move is blocked when both right-angle routes around its corner are walled.",
                        "A road move follows a road link, ignores walls and ignores the terrain class.",
                    ],
                },
                "stats": built.counts,
                "walls": {"edges": built.wall_edges},
                "roads": {
                    "parts": built.road_parts,
                    "links": built.road_links,
                    "pixels": built.road_pixels,
                    "pixelsOverWater": built.road_water_pixels,
                    "crossings": built.road_crossings,
                    "forced": built.road_forced,
                },
            },
            indent=2,
        )
        + "\n"
    )


# ── Feature collection ───────────────────────────────────────────────────────


def _collect_water(geom, props: dict, areas: List, lines: List, lat_c: float, pixel_m: float) -> None:
    if geom.geom_type in ("Polygon", "MultiPolygon"):
        _append_poly(areas, geom)
        return
    if geom.geom_type in ("LineString", "MultiLineString"):
        half_width_m = C.water_half_width_m(props)
        if half_width_m <= 0:
            return
        # Every waterway is a wall along its centreline. One at least a pixel
        # wide is also water ground, so the map and the costs see it.
        lines.extend(_as_lines(geom))
        if 2 * half_width_m >= pixel_m:
            buffered = _buffer_m(geom, half_width_m, lat_c)
            if buffered is not None and not buffered.is_empty:
                areas.append(buffered)
        return
    if geom.geom_type == "GeometryCollection":
        for sub in geom.geoms:
            _collect_water(sub, props, areas, lines, lat_c, pixel_m)


def _as_lines(geom) -> List[Any]:
    out: List[Any] = []
    if geom is None or geom.is_empty:
        return out
    geom_type = geom.geom_type
    if geom_type == "LineString":
        out.append(geom)
    elif geom_type == "MultiLineString":
        out.extend(part for part in geom.geoms if not part.is_empty)
    elif geom_type == "GeometryCollection":
        for part in geom.geoms:
            out.extend(_as_lines(part))
    elif geom_type in ("Polygon", "MultiPolygon"):
        out.extend(_as_lines(geom.boundary))
    return out


def _buffer_m(geom, half_width_m: float, lat_c: float):
    if geom is None or geom.is_empty or half_width_m <= 0:
        return None
    degrees = half_width_m / (111_320.0 * max(0.2, math.cos(math.radians(lat_c))))
    try:
        return geom.buffer(degrees)
    except Exception:
        return None


def _append_poly(bucket: List, geom) -> None:
    if geom is None or geom.is_empty:
        return
    if geom.geom_type in ("Polygon", "MultiPolygon"):
        bucket.append(geom)
    elif geom.geom_type == "GeometryCollection":
        for sub in geom.geoms:
            _append_poly(bucket, sub)


# ── Lattice coordinates ─────────────────────────────────────────────────────


def _lattice_transform(bounds, width: int, height: int, pad: int):
    """lon/lat -> lattice x/y: pixel centres on integers, y south, padded."""
    west, south, east, north = bounds
    sx = (east - west) / width
    sy = (north - south) / height

    def apply(coords: np.ndarray) -> np.ndarray:
        out = np.empty_like(coords)
        out[:, 0] = (coords[:, 0] - west) / sx - 0.5 + pad
        out[:, 1] = (north - coords[:, 1]) / sy - 0.5 + pad
        return out

    return apply


def _lattice_parts(geoms: Sequence[Any], to_lattice, box) -> np.ndarray:
    """LineString parts in lattice coordinates, clipped to the padded box."""
    geoms = [g for g in geoms if g is not None and not g.is_empty]
    if not geoms:
        return np.zeros(0, dtype=object)
    arr = shapely.transform(np.asarray(geoms, dtype=object), to_lattice)
    arr = shapely.clip_by_rect(arr, *box)
    parts = shapely.get_parts(arr)
    parts = parts[shapely.get_type_id(parts) == 1]  # LineString only
    return parts[~shapely.is_empty(parts)]


# ── Raster I/O ───────────────────────────────────────────────────────────────


def _south_to_north_bytes(plane: np.ndarray) -> bytes:
    """Encode a north-first Rasterio plane with the southern row first."""
    if plane.ndim != 2:
        raise ValueError("Raster plane must be two-dimensional")
    return plane[::-1, :].tobytes(order="C")


def _load_elev_grid(dem_path, west, south, east, north, pixel_m):
    with rasterio.open(dem_path) as src:
        elev_src = src.read(1).astype(np.float32)
        nodata = src.nodata
        if nodata is not None:
            elev_src = np.where(elev_src == nodata, np.nan, elev_src)
        elev_src = np.where(elev_src < -1000, np.nan, elev_src)
        elev_src = np.where(elev_src > 9000, np.nan, elev_src)
        elev_transform = src.transform

    height, width, _, _ = grid.shape_for_bounds(west, south, east, north, pixel_m)
    destination_transform = from_bounds(west, south, east, north, width, height)
    out = np.full((height, width), np.nan, dtype=np.float32)
    reproject(
        source=elev_src,
        destination=out,
        src_transform=elev_transform,
        src_crs="EPSG:4326",
        dst_transform=destination_transform,
        dst_crs="EPSG:4326",
        resampling=Resampling.bilinear,
        src_nodata=np.nan,
        dst_nodata=np.nan,
    )
    return out, destination_transform, width, height


def _slope_from_elev(elev: np.ndarray, pixel_m: float) -> np.ndarray:
    gradient_y, gradient_x = np.gradient(elev, pixel_m, pixel_m)
    slope = np.sqrt(gradient_x * gradient_x + gradient_y * gradient_y)
    return np.where(np.isfinite(elev), slope, np.nan).astype(np.float32)


def _encode_slope(slope: np.ndarray) -> np.ndarray:
    out = np.full(slope.shape, SLOPE_NODATA, dtype=np.uint8)
    valid = np.isfinite(slope)
    out[valid] = np.clip(np.rint(slope[valid] * SLOPE_SCALE), 0, 250).astype(np.uint8)
    return out


def _shapes(polys, value: int):
    shapes = []
    for geom in polys:
        if geom is None or geom.is_empty:
            continue
        if geom.geom_type in ("Polygon", "MultiPolygon"):
            shapes.append((geom, value))
        elif geom.geom_type == "GeometryCollection":
            for sub in geom.geoms:
                if sub.geom_type in ("Polygon", "MultiPolygon") and not sub.is_empty:
                    shapes.append((sub, value))
    return shapes


def _rasterize_mask(polys, transform, height, width, all_touched: bool = True) -> np.ndarray:
    mask = np.zeros((height, width), dtype=np.uint8)
    shapes = _shapes(polys, 1)
    if not shapes:
        return mask
    return rio_features.rasterize(
        shapes,
        out_shape=(height, width),
        transform=transform,
        fill=0,
        dtype="uint8",
        all_touched=all_touched,
    )


def _burn(grid_arr: np.ndarray, polys, class_id: int, transform) -> None:
    if not polys:
        return
    shapes = _shapes(polys, int(class_id))
    if not shapes:
        return
    if class_id == 0:
        # Rasterize cannot distinguish a burned zero from its zero fill.
        height, width = grid_arr.shape
        layer = _rasterize_mask(polys, transform, height, width)
        grid_arr[layer > 0] = 0
        return
    rio_features.rasterize(shapes, out=grid_arr, transform=transform, all_touched=True)


def _clear_tiny_components(mask: np.ndarray, max_px: int = 2) -> int:
    """Drop connected components of at most ``max_px`` pixels, in place."""
    labeled, component_count = ndimage.label(mask > 0)
    if component_count == 0:
        return 0
    sizes = np.bincount(labeled.ravel(), minlength=component_count + 1)
    small = sizes <= max_px
    small[0] = False
    if not small.any():
        return 0
    mask[small[labeled]] = 0
    return int(sizes[small].sum())
