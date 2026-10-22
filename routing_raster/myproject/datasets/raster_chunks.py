"""Pre-generate the routing raster: one chunk per 2-degree cell.

Format 2: each chunk is terrain.bin (ground), slope.bin and links.bin (river
walls and road links between pixels). See myproject/terrain/links.py.
"""

from __future__ import annotations

import json
import logging
import os
import shutil
import tempfile
from concurrent.futures import ProcessPoolExecutor, FIRST_COMPLETED, wait

from transforms.api import Input, LightweightInput, LightweightOutput, Output, transform

from myproject.terrain import build, grid, vector

log = logging.getLogger(__name__)

ELEVATION = "/Accenture/[DK] Project Space/Offline World/Elevation"
PMTILES = "/Accenture/[DK] Project Space/Offline World/[MAP] Chunked PMtiles"
# A new dataset: format 1 clients read roads from terrain.bin, which format 2
# no longer paints. Keep the old dataset until the client has moved over.
RASTERS = "/Accenture/[DK] Project Space/Offline World/[DK] Rasters v2"

# 50 m stays workable: rivers and roads no longer need to be a pixel wide.
PIXEL_M = 50.0

PLANES = ("terrain.bin", "slope.bin", "links.bin", "meta.json")

# Optional validation allowlist. Leave empty to process every cell that has
# both a PMTiles archive and a DEM.
ONLY_CELLS: frozenset[str] = frozenset()

# 8 x 32 GB, not 16 x 64 GB. Two reasons, both from the build history:
#
#   - A 16-core/64 GB container is one indivisible scheduling ask. The
#     2026-09-11 build sat in the queue for 23h58m before its container was
#     placed, then processed cells normally for 13 minutes. The wait was the
#     whole cost of that run; the compute was never the problem.
#   - Two earlier runs died OOMKilled after 4.4h and 5.5h. Sixteen workers
#     each holding a ~19.8M-pixel cell (elevation alone is 79 MB as float32,
#     before the slope intermediates, the label array and the geometry) is
#     what pushed a 64 GB container over.
#
# Halving both keeps ~4 GB/worker and asks for a slot that is far easier to
# place. Cost is ~2x wall on compute, which the optimisations below repay.
# The links plane adds one uint8 plane per cell (~20 MB), well inside that.
WORKERS = 8
MAX_IN_FLIGHT = WORKERS * 2
FAIL_FAST_AFTER = 5


@transform.using(
    output=Output(RASTERS),
    elevation=Input(ELEVATION),
    pmtiles=Input(PMTILES),
).with_resources(cpu_cores=WORKERS, memory_gb=32)
def compute(
    elevation: LightweightInput,
    pmtiles: LightweightInput,
    output: LightweightOutput,
):
    tiles_fs = pmtiles.filesystem()
    dem_fs = elevation.filesystem()
    out_fs = output.filesystem()

    archives = _archives_by_cell(tiles_fs)
    dems = _dems_by_cell(dem_fs)

    if not archives:
        raise ValueError("No per-cell PMTiles archives found. Expected z{zoom}/c{col}_r{row}.pmtiles")

    keys = sorted(set(archives) & set(dems))
    if ONLY_CELLS:
        keys = [key for key in keys if key in ONLY_CELLS]

    log.info(
        "%d archives, %d dems, %d both%s · %d workers · %.0f m/px",
        len(archives),
        len(dems),
        len(set(archives) & set(dems)),
        f", {len(keys)} selected" if ONLY_CELLS else "",
        WORKERS,
        PIXEL_M,
    )
    if not keys:
        raise ValueError("No cell has both an archive and a DEM")

    written_keys: list[str] = []
    failed = 0

    with tempfile.TemporaryDirectory() as work, ProcessPoolExecutor(max_workers=WORKERS) as pool:
        pending: dict = {}
        queue = list(keys)

        def submit_next() -> bool:
            if not queue:
                return False
            key = queue.pop(0)
            cell_dir = os.path.join(work, key)
            os.makedirs(cell_dir, exist_ok=True)
            archive = _stage(tiles_fs, archives[key], cell_dir, "cell.pmtiles")
            dem = _stage(dem_fs, dems[key], cell_dir, "cell.tif")
            future = pool.submit(_build_one, key, archive, dem, cell_dir, PIXEL_M)
            pending[future] = (key, cell_dir)
            return True

        while len(pending) < MAX_IN_FLIGHT and submit_next():
            pass

        while pending:
            done, _ = wait(list(pending), return_when=FIRST_COMPLETED)
            for future in done:
                key, cell_dir = pending.pop(future)
                try:
                    report = future.result()
                except Exception:
                    log.exception("cell %s failed; skipping", key)
                    failed += 1
                    shutil.rmtree(cell_dir, ignore_errors=True)
                    if not written_keys and failed >= FAIL_FAST_AFTER:
                        raise ValueError(
                            f"First {failed} cells failed with no successes — likely a code bug; stopping."
                        )
                else:
                    _publish(out_fs, key, cell_dir)
                    shutil.rmtree(cell_dir, ignore_errors=True)
                    written_keys.append(key)
                    _log_cell(key, report)

            # Refill once per drained batch, outside the per-future branches.
            # This used to sit at the end of the success path, so `continue` on
            # a failure skipped it: every failed cell permanently shrank the
            # in-flight window. Enough failures and the window reached zero,
            # `pending` drained and this loop exited with cells still queued —
            # never built, never reported, but still committed and still listed
            # in the manifest as if they were there.
            while len(pending) < MAX_IN_FLIGHT and submit_next():
                pass

        if queue:
            raise ValueError(
                f"{len(queue)} cells were never submitted — scheduler bug; "
                "refusing to commit a partial grid."
            )

    if not written_keys:
        raise ValueError("Every cell failed; not committing empty dataset.")

    # The manifest lists what was written, not what was attempted. It used to
    # publish every key in `keys`, so a failed cell became an entry the client
    # would look up and not find.
    _write_manifest(out_fs, sorted(written_keys))
    log.info("wrote %d chunks, %d cells failed", len(written_keys), failed)


def _log_cell(key: str, report: dict) -> None:
    counts = report["counts"]
    log.info(
        "%s: %dx%d · %s · walls %d · roads %d parts, %d links, %d pixels (%d over water), "
        "%d crossings, %d forced · water specks cleared %d",
        key,
        report["width"],
        report["height"],
        ", ".join(
            f"{name} {counts[name]}"
            for name in ("nodata", "water", "building", "rough", "open")
            if counts.get(name)
        )
        or "no classes",
        report["wall_edges"],
        report["road_parts"],
        report["road_links"],
        report["road_pixels"],
        report["road_water_pixels"],
        report["road_crossings"],
        report["road_forced"],
        report["water_specks_cleared"],
    )
    if report["road_forced"]:
        log.warning(
            "%s: %d road segments were forced through a wall away from any real crossing",
            key,
            report["road_forced"],
        )


def _build_one(key: str, archive: str, dem: str, cell_dir: str, pixel_m: float) -> dict:
    coords = grid.parse_cell_key(key)
    if coords is None:
        raise ValueError(f"{key} is not a cNNN_rNNN cell id")
    bounds = grid.cell_bounds(*coords)

    # Read a little beyond the cell: walls and roads near the edge are traced
    # from the geometry just across it (build.PAD_PX).
    features = list(vector.read_cell_features(archive, build.read_bounds(bounds, pixel_m), build.LAYERS))
    built = build.build_cell(key, bounds, features, dem, pixel_m)

    # Rasterio works north-to-south, while the client addresses binary rows
    # from the south edge. Flip every plane exactly once at the file boundary.
    for name, plane in (("terrain.bin", built.terrain), ("slope.bin", built.slope), ("links.bin", built.links)):
        with open(os.path.join(cell_dir, name), "wb") as handle:
            handle.write(build._south_to_north_bytes(plane))
    with open(os.path.join(cell_dir, "meta.json"), "w") as handle:
        handle.write(build.meta_for(key, built))

    return {
        "width": built.width,
        "height": built.height,
        "counts": built.counts,
        "wall_edges": built.wall_edges,
        "road_parts": built.road_parts,
        "road_links": built.road_links,
        "road_pixels": built.road_pixels,
        "road_water_pixels": built.road_water_pixels,
        "road_crossings": built.road_crossings,
        "road_forced": built.road_forced,
        "water_specks_cleared": built.water_specks_cleared,
        "features": len(features),
    }


def _stage(fs, path: str, cell_dir: str, name: str) -> str:
    local = os.path.join(cell_dir, name)
    with fs.open(path, "rb") as src, open(local, "wb") as dst:
        shutil.copyfileobj(src, dst)
    return local


def _publish(out_fs, key: str, cell_dir: str) -> None:
    for name in PLANES:
        local = os.path.join(cell_dir, name)
        text = name.endswith(".json")
        with open(local, "r" if text else "rb") as src, out_fs.open(f"raster/{key}/{name}", "w" if text else "wb") as dst:
            shutil.copyfileobj(src, dst)


def _write_manifest(out_fs, keys: list[str]) -> None:
    with out_fs.open("manifest.json", "w") as handle:
        handle.write(
            json.dumps(
                {
                    "format": build.FORMAT,
                    "grid": {
                        "originLon": grid.ORIGIN_LON,
                        "originLat": grid.ORIGIN_LAT,
                        "cellDeg": grid.CELL_DEG,
                    },
                    "pixelM": PIXEL_M,
                    "pathTemplate": "raster/{cell}",
                    "planes": [name for name in PLANES if name.endswith(".bin")],
                    "cells": list(keys),
                },
                indent=2,
            )
        )


def _archives_by_cell(fs) -> dict[str, str]:
    best: dict[str, tuple[int, str]] = {}
    for entry in fs.ls():
        name = getattr(entry, "path", entry)
        if not name.endswith(".pmtiles") or "/" not in name:
            continue
        folder, filename = name.rsplit("/", 1)
        if not folder.startswith("z"):
            continue
        try:
            zoom = int(folder[1:])
        except ValueError:
            continue
        key = filename[: -len(".pmtiles")]
        if grid.parse_cell_key(key) is None:
            continue
        if key not in best or zoom > best[key][0]:
            best[key] = (zoom, name)
    return {key: path for key, (_, path) in best.items()}


def _dems_by_cell(fs) -> dict[str, str]:
    dems: dict[str, str] = {}
    for entry in fs.ls():
        name = getattr(entry, "path", entry)
        if not name.endswith(".tif"):
            continue
        key = name.rsplit("/", 1)[-1][: -len(".tif")]
        if grid.parse_cell_key(key) is not None:
            dems[key] = name
    return dems
