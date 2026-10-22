"""One cell end to end: the three planes, on a synthetic DEM and features."""

from __future__ import annotations

import json
from collections import deque

import numpy as np
import pytest
import rasterio
import shapely
from rasterio.transform import from_bounds

from myproject.terrain import build
from myproject.terrain import classes as C
from myproject.terrain import links as L

# A small cell: 0.04 x 0.02 degrees at 54 N, about 52 x 44 pixels at 50 m.
BOUNDS = (18.60, 54.20, 18.64, 54.22)
PIXEL_M = 50.0


def lonlat(fx: float, fy: float):
    """A point by fractions of the cell: (0, 0) south-west, (1, 1) north-east."""
    west, south, east, north = BOUNDS
    return (west + fx * (east - west), south + fy * (north - south))


@pytest.fixture()
def dem(tmp_path):
    west, south, east, north = BOUNDS
    path = tmp_path / "cell.tif"
    width, height = 80, 60
    rows = np.linspace(0, 30, height, dtype=np.float32)[:, None]
    data = np.repeat(rows, width, axis=1)
    data[0:3, 0:3] = -9999  # a nodata corner
    with rasterio.open(
        path,
        "w",
        driver="GTiff",
        width=width,
        height=height,
        count=1,
        dtype="float32",
        crs="EPSG:4326",
        transform=from_bounds(west - 0.01, south - 0.01, east + 0.01, north + 0.01, width, height),
        nodata=-9999,
    ) as dst:
        dst.write(data, 1)
    return str(path)


def features():
    river = shapely.LineString([lonlat(-0.1, 0.52), lonlat(0.3, 0.47), lonlat(0.6, 0.55), lonlat(1.1, 0.5)])
    # A road crossing the river, and another running along it ~10 m north.
    crossing = shapely.LineString([lonlat(0.45, 0.05), lonlat(0.47, 0.95)])
    beside = shapely.LineString(river).offset_curve(0.0001)
    lake = shapely.Polygon([lonlat(0.75, 0.75), lonlat(0.9, 0.75), lonlat(0.9, 0.9), lonlat(0.75, 0.9)])
    house = shapely.Polygon([lonlat(0.1, 0.8), lonlat(0.15, 0.8), lonlat(0.15, 0.85), lonlat(0.1, 0.85)])
    return [
        {"layer": "water", "props": {"half_width_m": 6.0}, "geom": river},
        {"layer": "water", "props": {}, "geom": lake},
        {"layer": "roads", "props": {"kind": "road"}, "geom": crossing},
        {"layer": "roads", "props": {"kind": "road"}, "geom": beside},
        {"layer": "roads", "props": {"kind": "path"}, "geom": shapely.LineString([lonlat(0.1, 0.1), lonlat(0.2, 0.9)])},
        {"layer": "building", "props": {"kind": "building"}, "geom": house},
    ]


def test_build_cell_writes_three_consistent_planes(dem):
    built = build.build_cell("c000_r000", BOUNDS, features(), dem, PIXEL_M)
    h, w = built.height, built.width
    assert built.terrain.shape == built.slope.shape == built.links.shape == (h, w)
    assert built.links.dtype == np.uint8

    # Roads are never painted into the ground plane.
    assert built.counts["road"] == 0
    assert built.counts["water"] > 0 and built.counts["building"] > 0

    assert built.wall_edges > 0
    assert built.road_parts == 2  # the path is not a road surface
    assert built.road_crossings >= 1
    assert built.road_forced == 0

    meta = json.loads(build.meta_for("c000_r000", built))
    assert meta["format"] == build.FORMAT
    assert meta["links"]["bits"]["wallN"] == L.WALL_N
    assert meta["roads"]["forced"] == 0


def test_the_river_is_closed_off_road_and_open_by_the_bridge(dem):
    built = build.build_cell("c000_r000", BOUNDS, features(), dem, PIXEL_M)
    grid = L.LinkGrid(built.links.copy())
    h, w = built.height, built.width
    t_water = C.CLASS_NUMBER[C.WATER]

    def reach(start, roads):
        seen = {start}
        queue = deque([start])
        while queue:
            x, y = queue.popleft()
            for dx, dy in L.DIRECTIONS:
                nx, ny = x + dx, y + dy
                if not (0 <= nx < w and 0 <= ny < h) or (nx, ny) in seen:
                    continue
                by_road = roads and grid.linked(x, y, dx, dy)
                off_road = not grid.blocked(x, y, dx, dy) and built.terrain[ny, nx] != t_water
                if by_road or off_road:
                    seen.add((nx, ny))
                    queue.append((nx, ny))
        return seen

    south, north = (3, h - 3), (3, 3)  # rows run north to south in memory
    assert north not in reach(south, roads=False)
    assert north in reach(south, roads=True)


def test_files_are_flipped_to_south_first(dem):
    built = build.build_cell("c000_r000", BOUNDS, features(), dem, PIXEL_M)
    data = np.frombuffer(build._south_to_north_bytes(built.links), dtype=np.uint8).reshape(built.links.shape)
    assert np.array_equal(data[0], built.links[-1])


def test_read_bounds_cover_the_traced_margin():
    west, south, east, north = build.read_bounds(BOUNDS, PIXEL_M)
    assert west < BOUNDS[0] and south < BOUNDS[1] and east > BOUNDS[2] and north > BOUNDS[3]
