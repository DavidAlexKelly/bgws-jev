"""River walls and road links: the movement plane of a routing chunk.

Rivers and roads are thinner than a pixel, so they do not live on pixels.
They live on the moves between neighbouring pixel centres:

  - A river is a WALL on every move whose centre-to-centre segment it
    crosses. Walls are traced from the river's own geometry, so they are
    watertight at any pixel size and need no width.
  - A road is a chain of LINKS between neighbouring pixels. Travel by road
    follows links and nothing else, so a river or a building sharing the
    road's pixels cannot cut it.
  - The one exception is decided here, at build time: a road link may cross
    a wall only near a point where the road's geometry really crosses the
    water (a bridge, a ford, a culvert). Everywhere else the road is traced
    around the walls, so a road beside a river never becomes a way across.

THE LATTICE
-----------
Coordinates are in pixel units with pixel centres on the integers: x runs
east (column), y runs south (row of a north-first array). Node (x, y) is the
centre of pixel [row y, col x].

THE BITS (one uint8 per pixel)
------------------------------
Each move between neighbours is stored once, on the pixel it leaves going
east or north. The directions are geographic, so flipping the rows when the
file is written changes nothing about what a bit means.

  WALL_E   0x01  a wall on the move to the east neighbour
  WALL_N   0x02  a wall on the move to the north neighbour
  ROAD_E   0x04  a road link to the east neighbour
  ROAD_N   0x08  a road link to the north neighbour
  ROAD_NE  0x10  a road link to the north-east neighbour
  ROAD_NW  0x20  a road link to the north-west neighbour

A diagonal off-road move is blocked when both of the two right-angle routes
around its corner are walled. That is exactly the case in which the walls
separate the two pixels, so a one-pixel diagonal river cannot be slipped
through at its corners.
"""

from __future__ import annotations

import math
from collections import deque
from dataclasses import dataclass
from typing import Callable, List, Sequence, Tuple

import numpy as np
import shapely
from shapely import STRtree

WALL_E = 0x01
WALL_N = 0x02
ROAD_E = 0x04
ROAD_N = 0x08
ROAD_NE = 0x10
ROAD_NW = 0x20
ROAD_BITS = ROAD_E | ROAD_N | ROAD_NE | ROAD_NW

# Lattice directions (dx east, dy south), all eight.
DIRECTIONS: Tuple[Tuple[int, int], ...] = (
    (1, 0), (1, -1), (0, -1), (-1, -1), (-1, 0), (-1, 1), (0, 1), (1, 1),
)

# Which pixel owns a road link in each direction, and with which bit.
_ROAD_OWNER = {
    (1, 0): (0, 0, ROAD_E),
    (-1, 0): (-1, 0, ROAD_E),
    (0, -1): (0, 0, ROAD_N),
    (0, 1): (0, 1, ROAD_N),
    (1, -1): (0, 0, ROAD_NE),
    (-1, 1): (-1, 1, ROAD_NE),
    (-1, -1): (0, 0, ROAD_NW),
    (1, 1): (1, 1, ROAD_NW),
}

# A wall line passing exactly through a pixel centre would leave it on both
# sides at once. A shift far below a pixel makes that case impossible.
_NUDGE = (1.31e-6, 2.17e-6)

# How close (pixels) a road link must be to a real road/water crossing to be
# allowed through a wall.
CROSSING_RADIUS = 1.5

# Road samples per pixel of road length.
_SAMPLE_STEP = 0.5

# Search margins (pixels) around a road segment that has to go round water.
_CONNECT_MARGINS = (3, 12)

# How far (pixels) a road vertex may be snapped to stay on its side of the water.
_SNAP_REACH = 3


@dataclass
class RoadReport:
    roads: int = 0
    crossings: int = 0
    # Segments that could not reach their next vertex without crossing a wall
    # away from any real crossing. The road is kept continuous and the count
    # is reported; it should be zero or close to it.
    forced: int = 0


class LinkGrid:
    """The movement bits of a lattice, with the move rules on top."""

    def __init__(self, bits: np.ndarray):
        if bits.dtype != np.uint8 or bits.ndim != 2:
            raise ValueError("bits must be a 2-D uint8 array")
        self.bits = bits
        self.h, self.w = bits.shape
        self._flat = bits.reshape(-1)

    def _has(self, x: int, y: int, mask: int) -> bool:
        return 0 <= x < self.w and 0 <= y < self.h and bool(self._flat[y * self.w + x] & mask)

    def wall(self, x: int, y: int, dx: int, dy: int) -> bool:
        """A wall on the orthogonal move from (x, y) by (dx, dy)."""
        if dx == 1:
            return self._has(x, y, WALL_E)
        if dx == -1:
            return self._has(x - 1, y, WALL_E)
        if dy == -1:
            return self._has(x, y, WALL_N)
        return self._has(x, y + 1, WALL_N)

    def blocked(self, x: int, y: int, dx: int, dy: int) -> bool:
        """Is the off-road move from (x, y) by (dx, dy) across a river?"""
        if dx == 0 or dy == 0:
            return self.wall(x, y, dx, dy)
        via_x = self.wall(x, y, dx, 0) or self.wall(x + dx, y, 0, dy)
        via_y = self.wall(x, y, 0, dy) or self.wall(x, y + dy, dx, 0)
        return via_x and via_y

    def link(self, x: int, y: int, dx: int, dy: int) -> None:
        ox, oy, bit = _ROAD_OWNER[(dx, dy)]
        X, Y = x + ox, y + oy
        if 0 <= X < self.w and 0 <= Y < self.h:
            self._flat[Y * self.w + X] |= bit

    def linked(self, x: int, y: int, dx: int, dy: int) -> bool:
        ox, oy, bit = _ROAD_OWNER[(dx, dy)]
        return self._has(x + ox, y + oy, bit)


# ── Walls ────────────────────────────────────────────────────────────────────


def segments_of(parts: Sequence) -> np.ndarray:
    """All segments of some LineStrings as an (S, 4) array x0, y0, x1, y1."""
    parts = np.asarray(parts, dtype=object)
    if parts.size == 0:
        return np.zeros((0, 4))
    coords, index = shapely.get_coordinates(parts, return_index=True)
    if len(coords) < 2:
        return np.zeros((0, 4))
    same = index[1:] == index[:-1]
    return np.hstack([coords[:-1][same], coords[1:][same]])


def _integer_crossings(a0, a1, b0, b1):
    """Where segments cross the lines a = k for integer k.

    Counts integers with min(a0, a1) < k <= max(a0, a1), so a vertex lying on
    a line is counted once between the two segments that share it.
    """
    lo = np.floor(np.minimum(a0, a1)) + 1
    hi = np.floor(np.maximum(a0, a1))
    n = np.clip(hi - lo + 1, 0, None).astype(np.int64)
    total = int(n.sum())
    if total == 0:
        return np.zeros(0, dtype=np.int64), np.zeros(0)
    seg = np.repeat(np.arange(len(a0)), n)
    offset = np.arange(total) - np.repeat(np.cumsum(n) - n, n)
    k = lo[seg] + offset
    t = (k - a0[seg]) / (a1[seg] - a0[seg])
    b = b0[seg] + t * (b1[seg] - b0[seg])
    return k.astype(np.int64), b


def trace_walls(bits: np.ndarray, segments: np.ndarray) -> int:
    """Mark a wall on every centre-to-centre move a segment crosses.

    Returns the number of wall bits that were newly set.
    """
    if len(segments) == 0:
        return 0
    h, w = bits.shape
    before = int(np.count_nonzero(bits & (WALL_E | WALL_N)))
    x0 = segments[:, 0] + _NUDGE[0]
    y0 = segments[:, 1] + _NUDGE[1]
    x1 = segments[:, 2] + _NUDGE[0]
    y1 = segments[:, 3] + _NUDGE[1]

    # Crossing y = k cuts the east move from node (floor(x), k).
    rows, xs = _integer_crossings(y0, y1, x0, x1)
    cols = np.floor(xs).astype(np.int64)
    keep = (rows >= 0) & (rows < h) & (cols >= 0) & (cols < w)
    bits[rows[keep], cols[keep]] |= WALL_E

    # Crossing x = k cuts the north move from node (k, floor(y) + 1).
    cols, ys = _integer_crossings(x0, x1, y0, y1)
    rows = np.floor(ys).astype(np.int64) + 1
    keep = (rows >= 0) & (rows < h) & (cols >= 0) & (cols < w)
    bits[rows[keep], cols[keep]] |= WALL_N

    return int(np.count_nonzero(bits & (WALL_E | WALL_N))) - before


# ── Snapping road vertices to the right side of the water ───────────────────


def snap_vertices(points: np.ndarray, grid: LinkGrid, wall_tree: STRtree | None) -> np.ndarray:
    """The pixel centre each road vertex belongs to.

    Normally the nearest centre. Where a wall line runs through the square
    around the vertex, the nearest centre may be across the water from it,
    so the nearest centre whose straight line from the vertex meets no wall
    geometry is taken instead.
    """
    snapped = np.floor(points + 0.5).astype(np.int64)
    if wall_tree is None or len(points) == 0:
        return snapped

    i = np.floor(points[:, 0]).astype(np.int64)
    j = np.floor(points[:, 1]).astype(np.int64)
    bits = grid.bits
    h, w = bits.shape

    def has(xs, ys, mask):
        inside = (xs >= 0) & (xs < w) & (ys >= 0) & (ys < h)
        out = np.zeros(len(xs), dtype=bool)
        out[inside] = (bits[ys[inside], xs[inside]] & mask) > 0
        return out

    # The square with corners (i, j) .. (i + 1, j + 1) is cut if any of its
    # four sides carries a wall.
    dirty = (
        has(i, j, WALL_E)
        | has(i, j + 1, WALL_E)
        | has(i, j + 1, WALL_N)
        | has(i + 1, j + 1, WALL_N)
    )
    for n in np.nonzero(dirty)[0]:
        px, py = float(points[n, 0]), float(points[n, 1])
        cx, cy = int(snapped[n, 0]), int(snapped[n, 1])
        # Out to three pixels: at a sharp bend the vertex's side of the water
        # can be a sliver with no pixel centre in the 3 x 3 around it.
        candidates = sorted(
            ((cx + ox, cy + oy) for ox in range(-_SNAP_REACH, _SNAP_REACH + 1) for oy in range(-_SNAP_REACH, _SNAP_REACH + 1)),
            key=lambda c: (c[0] - px) ** 2 + (c[1] - py) ** 2,
        )
        for cand in candidates:
            # Walls were traced from the geometry shifted by _NUDGE; shifting
            # the probe back by it asks the same question the walls answered,
            # even for a centre a millionth of a pixel from the water.
            probe = shapely.linestrings(
                [(px - _NUDGE[0], py - _NUDGE[1]), (cand[0] - _NUDGE[0], cand[1] - _NUDGE[1])]
            )
            if len(wall_tree.query(probe, predicate="intersects")) == 0:
                snapped[n] = cand
                break
    return snapped


# ── Real crossings ───────────────────────────────────────────────────────────


def road_crossings(road_parts: np.ndarray, wall_parts: np.ndarray, wall_tree: STRtree | None) -> List[np.ndarray]:
    """For each road part, the points where it really meets water geometry."""
    out: List[np.ndarray] = [np.zeros((0, 2)) for _ in range(len(road_parts))]
    if wall_tree is None or len(road_parts) == 0:
        return out
    road_index, wall_index = wall_tree.query(road_parts, predicate="intersects")
    if len(road_index) == 0:
        return out
    meets = shapely.intersection(road_parts[road_index], wall_parts[wall_index])
    coords, which = shapely.get_coordinates(meets, return_index=True)
    owners = road_index[which]
    order = np.argsort(owners, kind="stable")
    owners, coords = owners[order], coords[order]
    starts = np.searchsorted(owners, np.arange(len(road_parts)), side="left")
    ends = np.searchsorted(owners, np.arange(len(road_parts)), side="right")
    for r in np.nonzero(ends > starts)[0]:
        out[r] = coords[starts[r]:ends[r]]
    return out


# ── Roads ────────────────────────────────────────────────────────────────────


def trace_roads(
    grid: LinkGrid,
    road_parts: np.ndarray,
    wall_parts: np.ndarray,
) -> RoadReport:
    """Draw every road part as a chain of links that respects the walls."""
    report = RoadReport()
    road_parts = np.asarray(road_parts, dtype=object)
    wall_parts = np.asarray(wall_parts, dtype=object)
    if road_parts.size == 0:
        return report
    wall_tree = STRtree(wall_parts) if wall_parts.size else None
    crossings = road_crossings(road_parts, wall_parts, wall_tree)

    coords, index = shapely.get_coordinates(road_parts, return_index=True)
    snapped = snap_vertices(coords, grid, wall_tree)
    bounds = np.searchsorted(index, np.arange(len(road_parts) + 1))

    for r in range(len(road_parts)):
        a, b = bounds[r], bounds[r + 1]
        if b - a < 1:
            continue
        report.roads += 1
        report.crossings += len(crossings[r])
        report.forced += _trace_one(grid, coords[a:b], snapped[a:b], crossings[r])
    return report


def _trace_one(grid: LinkGrid, pts: np.ndarray, nodes: np.ndarray, cross: np.ndarray) -> int:
    """Follow one polyline across the lattice, linking as it goes."""
    near_r2 = CROSSING_RADIUS * CROSSING_RADIUS
    cross_list = [(float(x), float(y)) for x, y in cross]

    def allowed(x: int, y: int, dx: int, dy: int) -> bool:
        if not grid.blocked(x, y, dx, dy):
            return True
        mx, my = x + dx * 0.5, y + dy * 0.5
        return any((cx - mx) ** 2 + (cy - my) ** 2 <= near_r2 for cx, cy in cross_list)

    forced = 0
    cx, cy = int(nodes[0, 0]), int(nodes[0, 1])
    for i in range(len(pts) - 1):
        x0, y0 = float(pts[i, 0]), float(pts[i, 1])
        x1, y1 = float(pts[i + 1, 0]), float(pts[i + 1, 1])
        steps = max(1, int(math.ceil(max(abs(x1 - x0), abs(y1 - y0)) / _SAMPLE_STEP)))
        for s in range(1, steps + 1):
            px = x0 + (x1 - x0) * s / steps
            py = y0 + (y1 - y0) * s / steps
            tx, ty = math.floor(px + 0.5), math.floor(py + 0.5)
            if tx == cx and ty == cy:
                continue
            dx = (tx > cx) - (tx < cx)
            dy = (ty > cy) - (ty < cy)
            if allowed(cx, cy, dx, dy):
                grid.link(cx, cy, dx, dy)
                cx, cy = cx + dx, cy + dy
                continue
            # The straight step is walled: take the open step that gets
            # closest to the road, or hold position along the bank.
            here = (cx - px) ** 2 + (cy - py) ** 2
            best = None
            for ddx, ddy in DIRECTIONS:
                d = (cx + ddx - px) ** 2 + (cy + ddy - py) ** 2
                if d < here - 1e-9 and (best is None or d < best[0]) and allowed(cx, cy, ddx, ddy):
                    best = (d, ddx, ddy)
            if best is not None:
                grid.link(cx, cy, best[1], best[2])
                cx, cy = cx + best[1], cy + best[2]

        ex, ey = int(nodes[i + 1, 0]), int(nodes[i + 1, 1])
        if (cx, cy) != (ex, ey):
            forced += _connect(grid, cx, cy, ex, ey, allowed)
            cx, cy = ex, ey
    return forced


def _route(grid: LinkGrid, sx: int, sy: int, ex: int, ey: int, margin: int, allowed) -> dict | None:
    """Breadth-first search from start to end inside a box, on the lattice."""
    lo_x = max(0, min(sx, ex) - margin)
    hi_x = min(grid.w - 1, max(sx, ex) + margin)
    lo_y = max(0, min(sy, ey) - margin)
    hi_y = min(grid.h - 1, max(sy, ey) + margin)
    came = {(sx, sy): None}
    queue = deque([(sx, sy)])
    while queue:
        x, y = queue.popleft()
        if (x, y) == (ex, ey):
            return came
        for dx, dy in DIRECTIONS:
            nx, ny = x + dx, y + dy
            if (nx, ny) in came or not (lo_x <= nx <= hi_x and lo_y <= ny <= hi_y):
                continue
            if allowed(x, y, dx, dy):
                came[(nx, ny)] = (x, y)
                queue.append((nx, ny))
    return None


def _connect(grid: LinkGrid, sx: int, sy: int, ex: int, ey: int, allowed: Callable[[int, int, int, int], bool]) -> int:
    """Link (sx, sy) to (ex, ey) by the shortest open route nearby.

    Returns 0 when a route was found, 1 when none was and a straight chain
    of links was drawn through the walls to keep the road continuous.
    """
    # A small window first; a wider one for a road that follows the bank
    # round a tongue of water whose tip is several pixels away.
    for margin in _CONNECT_MARGINS:
        came = _route(grid, sx, sy, ex, ey, margin, allowed)
        if came is not None:
            node = (ex, ey)
            while came[node] is not None:
                px, py = came[node]
                grid.link(px, py, node[0] - px, node[1] - py)
                node = (px, py)
            return 0

    x, y = sx, sy
    while (x, y) != (ex, ey):
        dx = (ex > x) - (ex < x)
        dy = (ey > y) - (ey < y)
        grid.link(x, y, dx, dy)
        x, y = x + dx, y + dy
    return 1


# ── Whole-plane helpers ─────────────────────────────────────────────────────


def road_nodes(bits: np.ndarray) -> np.ndarray:
    """Pixels touched by at least one road link, as a bool mask."""
    h, w = bits.shape
    out = (bits & ROAD_BITS) > 0
    # A link stored on a neighbour also touches this pixel.
    east = (bits & ROAD_E) > 0
    out[:, 1:] |= east[:, :-1]
    north = (bits & ROAD_N) > 0
    out[:-1, :] |= north[1:, :]
    ne = (bits & ROAD_NE) > 0
    out[:-1, 1:] |= ne[1:, :-1]
    nw = (bits & ROAD_NW) > 0
    out[:-1, :-1] |= nw[1:, 1:]
    return out
