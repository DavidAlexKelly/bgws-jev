"""Test setup: put routing_raster on the path and stand in for the modules
that live in the real project (classes, grid, vector) when they are absent.

The stand-ins only provide what build.py uses. In the real project the real
modules are imported instead and nothing here is used.
"""

from __future__ import annotations

import math
import os
import sys
import types

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)


def _stub_classes() -> types.ModuleType:
    m = types.ModuleType("myproject.terrain.classes")
    m.NODATA, m.WATER, m.ROAD, m.BUILDING = "nodata", "water", "road", "building"
    m.ROUGH, m.VERY_ROUGH, m.OPEN = "rough", "very_rough", "open"
    names = [m.NODATA, m.WATER, m.ROAD, m.BUILDING, m.ROUGH, m.VERY_ROUGH, m.OPEN]
    m.CLASS_NUMBER = {name: i for i, name in enumerate(names)}
    m.TERRAIN_CLASSES = {i: name for i, name in enumerate(names)}
    m.BUILDING_KINDS = {"building"}
    m.class_for_landuse = lambda props: props.get("class")
    m.is_road_surface = lambda props: props.get("kind") == "road"
    m.water_half_width_m = lambda props: float(props.get("half_width_m", 5.0))
    return m


def _stub_grid() -> types.ModuleType:
    m = types.ModuleType("myproject.terrain.grid")
    m.ORIGIN_LON, m.ORIGIN_LAT, m.CELL_DEG = -180.0, -90.0, 2.0

    def shape_for_bounds(west, south, east, north, pixel_m):
        lat_c = 0.5 * (south + north)
        width = max(1, int(round((east - west) * 111_320.0 * math.cos(math.radians(lat_c)) / pixel_m)))
        height = max(1, int(round((north - south) * 111_320.0 / pixel_m)))
        return height, width, (east - west) / width, (north - south) / height

    m.shape_for_bounds = shape_for_bounds
    m.parse_cell_key = lambda key: None
    m.cell_bounds = lambda col, row: None
    return m


def _install() -> None:
    try:
        import myproject.terrain.classes  # noqa: F401
        import myproject.terrain.grid  # noqa: F401
        return
    except ImportError:
        pass
    sys.modules["myproject.terrain.classes"] = _stub_classes()
    sys.modules["myproject.terrain.grid"] = _stub_grid()
    sys.modules.setdefault("myproject.terrain.vector", types.ModuleType("myproject.terrain.vector"))


_install()
