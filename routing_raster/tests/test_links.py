"""Walls are watertight, roads are continuous, and the two only meet where a
road really crosses water."""

from __future__ import annotations

from collections import deque

import numpy as np
import pytest
import shapely

from myproject.terrain import links as L

N = 40


def lattice(size: int = N) -> L.LinkGrid:
    return L.LinkGrid(np.zeros((size, size), dtype=np.uint8))


def walls(grid: L.LinkGrid, *lines) -> np.ndarray:
    parts = np.array([shapely.LineString(line) for line in lines], dtype=object)
    L.trace_walls(grid.bits, L.segments_of(parts))
    return parts


def flood(grid: L.LinkGrid, start, roads: bool = False) -> set:
    """Every node reachable off-road (and by road too, if asked)."""
    seen = {start}
    queue = deque([start])
    while queue:
        x, y = queue.popleft()
        for dx, dy in L.DIRECTIONS:
            nx, ny = x + dx, y + dy
            if not (0 <= nx < grid.w and 0 <= ny < grid.h) or (nx, ny) in seen:
                continue
            if not grid.blocked(x, y, dx, dy) or (roads and grid.linked(x, y, dx, dy)):
                seen.add((nx, ny))
                queue.append((nx, ny))
    return seen


def road_flood(grid: L.LinkGrid, start) -> set:
    seen = {start}
    queue = deque([start])
    while queue:
        x, y = queue.popleft()
        for dx, dy in L.DIRECTIONS:
            nxt = (x + dx, y + dy)
            if nxt not in seen and grid.linked(x, y, dx, dy):
                seen.add(nxt)
                queue.append(nxt)
    return seen


def wiggle(seed: int, size: int = N, amplitude: float = 6.0):
    """A river from the west edge to the east edge, y as a function of x."""
    rng = np.random.default_rng(seed)
    xs = np.linspace(-2, size + 1, 60)
    ys = size / 2 + np.cumsum(rng.normal(0, amplitude / 4, len(xs)))
    ys = np.clip(ys, 5, size - 6)
    return list(zip(xs, ys))


def side_of(river, x, y) -> int:
    """+1 south of the river (larger y), -1 north."""
    xs, ys = zip(*river)
    return 1 if y > np.interp(x, xs, ys) else -1


# ── Walls ───────────────────────────────────────────────────────────────────


def test_a_straight_river_blocks_across_and_not_along():
    grid = lattice()
    walls(grid, [(-2, 19.5), (N + 1, 19.5)])
    assert grid.blocked(10, 20, 0, -1) and grid.blocked(10, 19, 0, 1)
    assert grid.blocked(10, 20, 1, -1) and grid.blocked(10, 20, -1, -1)
    assert not grid.blocked(10, 20, 1, 0) and not grid.blocked(10, 19, -1, 0)
    reach = flood(grid, (5, 30))
    assert all(y >= 20 for _, y in reach)


def test_a_one_pixel_diagonal_river_has_no_corner_gaps():
    grid = lattice()
    walls(grid, [(-1.3, -1.1), (N + 0.7, N + 0.9)])
    reach = flood(grid, (30, 5))
    assert all(y < x + 0.2 for x, y in reach), "slipped through a corner"


@pytest.mark.parametrize("seed", range(30))
def test_a_wiggling_river_is_watertight(seed):
    grid = lattice()
    river = wiggle(seed)
    walls(grid, river)
    start = (3, N - 2)
    assert side_of(river, *start) == 1
    reach = flood(grid, start)
    assert all(side_of(river, x, y) == 1 for x, y in reach)
    # And it does not wall off more than it should: the whole south side is open.
    south = {(x, y) for x in range(N) for y in range(N) if side_of(river, x, y) == 1}
    assert reach == south


# ── Roads ───────────────────────────────────────────────────────────────────


def test_a_road_is_one_connected_chain():
    grid = lattice()
    road = shapely.LineString([(2.2, 3.7), (17.9, 11.1), (30.4, 35.6)])
    report = L.trace_roads(grid, np.array([road], dtype=object), np.zeros(0, dtype=object))
    assert report.forced == 0
    reach = road_flood(grid, (2, 4))
    assert (30, 36) in reach


def test_a_road_crosses_a_river_only_where_it_really_does():
    grid = lattice()
    wall_parts = walls(grid, [(-2, 19.5), (N + 1, 19.5)])
    road = shapely.LineString([(10.2, 3.3), (12.7, 36.1)])
    report = L.trace_roads(grid, np.array([road], dtype=object), wall_parts)
    assert report.crossings == 1 and report.forced == 0
    assert (13, 36) in road_flood(grid, (10, 3))
    # Off-road the river is still closed, away from the bridge and at it.
    assert all(y >= 20 for _, y in flood(grid, (30, 30)))


@pytest.mark.parametrize("seed", range(30))
@pytest.mark.parametrize("offset", [0.15, 0.4, 0.8, -0.15, -0.4, -0.8])
def test_a_road_beside_a_river_neither_breaks_nor_crosses(seed, offset):
    grid = lattice()
    river = wiggle(seed)
    wall_parts = walls(grid, river)
    # The road runs along one bank, closer than a pixel to the water.
    road = shapely.LineString(river).offset_curve(offset)
    road = shapely.clip_by_rect(road, 0.5, 0.5, N - 1.5, N - 1.5)
    parts = [p for p in shapely.get_parts(road) if p.length > 3]
    assert all(not p.intersects(shapely.LineString(river)) for p in parts)

    report = L.trace_roads(grid, np.array(parts, dtype=object), wall_parts)
    assert report.crossings == 0
    assert report.forced == 0

    # Continuous: each part's road links join its first vertex to its last.
    for part in parts:
        (x0, y0), (x1, y1) = part.coords[0], part.coords[-1]
        start = L.snap_vertices(np.array([[x0, y0]]), grid, shapely.STRtree(wall_parts))[0]
        end = L.snap_vertices(np.array([[x1, y1]]), grid, shapely.STRtree(wall_parts))[0]
        assert tuple(end) in road_flood(grid, tuple(start))

    # Never a way across: off-road plus road from either bank stays on it.
    for start, side in (((3, N - 2), 1), ((3, 1), -1)):
        reach = flood(grid, start, roads=True)
        assert all(side_of(river, x, y) == side for x, y in reach)


def test_roads_meeting_at_a_vertex_share_a_pixel():
    grid = lattice()
    a = shapely.LineString([(3.3, 20.2), (20.4, 20.4)])
    b = shapely.LineString([(20.4, 20.4), (20.9, 35.2)])
    L.trace_roads(grid, np.array([a, b], dtype=object), np.zeros(0, dtype=object))
    assert (21, 35) in road_flood(grid, (3, 20))


def test_road_nodes_covers_both_ends_of_every_link():
    grid = lattice(6)
    grid.link(2, 2, 1, 0)
    grid.link(2, 2, 0, -1)
    grid.link(2, 2, 1, -1)
    grid.link(2, 2, -1, -1)
    grid.link(4, 4, -1, 1)
    mask = L.road_nodes(grid.bits)
    on = {(x, y) for y, x in zip(*np.nonzero(mask))}
    assert on == {(2, 2), (3, 2), (2, 1), (3, 1), (1, 1), (4, 4), (3, 5)}
