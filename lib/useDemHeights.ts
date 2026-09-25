// ── bgws/lib/useDemHeights.ts ──────────────────────────────────────────────
// Real bare-earth elevation for the board, as a SYNCHRONOUS sampler.
//
// WHY A HOOK AND NOT A CALL
//
// @acc/decho-elevation reads 2° GeoTIFF cells out of a Foundry dataset, so
// getting a height is unavoidably asynchronous the first time. The rules are
// not: `TerrainSampler.groundHeightM` is synchronous, and deliberately so —
// the engine asks for a height inside every range check, every sight line and
// every step of a route, and an async sampler would turn all of that async up
// to and including `runTurn`.
//
// The package is built for exactly this and says so: `HeightSampler` offers
// `warm(points)` to make cells resident and `heightAtLoaded(lon, lat)` to read
// them back with no promise attached. So the asynchrony lives HERE, in a hook
// that loads the cells under the board once, and the rules never see it.
//
// ⚠ NaN IS AN ANSWER, NOT AN ERROR. `heightAtLoaded` returns NaN for a cell
// that is ocean or simply not resident yet, and that is precisely what
// `lineOfSight` wants: it reads a non-finite height as `noCoverage` and
// declines the shot rather than inventing sea level. So this module never
// substitutes a number, and the board is honestly unplayable until the DEM has
// landed — which is better than a board that plays flat and looks real.

import { useEffect, useMemo, useState } from "react";

import { cellFor, cellKey, type DemSourceHandle } from "@acc/decho-elevation";

import { boardBounds, type LatLng } from "./board";

export interface DemHeights {
  /**
   * Height in metres at a point, or NaN where the DEM has nothing.
   *
   * Null until the cells under the board are resident, so a caller can tell
   * "no elevation yet" from "elevation says this is the sea".
   */
  heightAt: ((point: LatLng) => number) | null;
  isLoaded: boolean;
  /**
   * Whether the DEM actually has data under this board.
   *
   * ⚠ A DIFFERENT QUESTION FROM `isLoaded`, AND THE ONE THAT WAS MISSING.
   * `warm()` resolves happily for a cell that does not exist — most of the
   * planet is ocean and has no chunk, which the package is explicit about
   * treating as normal rather than as an error. So a board outside the DEM's
   * coverage looks exactly like a board over the sea: loaded, and every height
   * NaN.
   *
   * Reported separately because the two want opposite responses. Not loaded is
   * "wait"; no coverage is "this dataset does not contain your ground", and no
   * amount of waiting fixes it.
   */
  hasCoverage: boolean;
  /** The chunk this board needs, named as the dataset names it. */
  cell: string | null;
  /**
   * Why there is no elevation, asked of the SOURCE rather than inferred.
   *
   * ⚠ NaN CANNOT TELL YOU WHY. A height is NaN whether the chunk does not
   * exist, the fetch was refused, or the cell simply is not resident yet — and
   * the extension deliberately swallows its own failures so that losing the
   * relief does not lose the map. Inferring "the dataset has no data here"
   * from NaN is therefore a guess, and it was the wrong one: it blamed
   * coverage for what may be access.
   *
   * `loadCell` answers properly. A grid means covered; null means the dataset
   * genuinely has no chunk there, which the package treats as normal; a throw
   * means the read failed, and that is the 403 case worth naming.
   */
  diagnosis: string | null;
  /** Set when the DEM could not be reached at all. */
  error: string | null;
}

/**
 * Points to make resident before the board is playable.
 *
 * A 2° cell is roughly 220 km across and the board is 10 km, so it sits inside
 * one cell — except when it straddles a boundary, where it can touch four. The
 * corners plus the centre find every cell in either case, and warming a point
 * that shares a cell with another costs nothing: the loader is keyed by cell.
 */
function warmPoints(centre: LatLng): { lon: number; lat: number }[] {
  const bounds = boardBounds(centre);
  return [
    { lon: centre.lng, lat: centre.lat },
    { lon: bounds.west, lat: bounds.south },
    { lon: bounds.west, lat: bounds.north },
    { lon: bounds.east, lat: bounds.south },
    { lon: bounds.east, lat: bounds.north },
  ];
}

/**
 * Warm the DEM cells under the board and hand back a synchronous sampler.
 *
 * ⚠ TAKES A SOURCE RATHER THAN CREATING ONE, AND THAT IS NOT A STYLE CHOICE.
 * The map's elevation extension already owns a `DemSourceHandle`, and the
 * package hands it out through `onReady` saying exactly why: it is "how an
 * application gets at heightAt ... without constructing a second source". A
 * second source means a second cell cache, and a cell is 3-25 MB — so calling
 * `createDemSource()` here would double both the memory and the fetches to
 * read the same heights the map is already drawing.
 *
 * Pass `null` to stay out of the way entirely, which is how the other ground
 * sources avoid paying for a dataset they do not read.
 */
export function useDemHeights(
  source: DemSourceHandle | null,
  centre: LatLng,
): DemHeights {
  const [ready, setReady] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [cell, setCell] = useState<string | null>(null);
  const [diagnosis, setDiagnosis] = useState<string | null>(null);

  // Round the centre before using it as a dependency. The board recentres on
  // the raster's coverage, which can jitter in the last decimal between
  // renders — and every jitter would otherwise re-warm the DEM.
  const key = `${centre.lat.toFixed(3)},${centre.lng.toFixed(3)}`;

  useEffect(() => {
    if (!source) return;
    let cancelled = false;

    void (async () => {
      try {
        // ⚠ THE DATASET MUST BE A RESOURCE ON THE APPLICATION, not merely
        // readable by the user. `api:use-datasets-read` alone answers 403, and
        // the Elevation dataset is a THIRD one — the basemap's tiles and its
        // glyphs do not cover it. The package's own defaults.ts calls this out
        // as the most common way it appears broken, so the message says it.
        //
        // The extension swallows its own 403 on purpose — losing the relief is
        // better than losing the map — so a failure can reach us as a warm()
        // that never resolves usefully rather than as a throw. The readout
        // therefore keys off `isLoaded`, not only off `error`.
        await source.warm(warmPoints(centre));
        if (cancelled) return;

        // Ask the source what it actually knows about this board's chunk,
        // rather than reading a NaN and inventing a reason for it.
        const coord = cellFor(source.store.grid, centre.lng, centre.lat);
        const name = cellKey(coord.col, coord.row);
        setCell(name);
        try {
          const grid = await source.loadCell(coord);
          if (cancelled) return;
          setDiagnosis(
            grid
              ? null
              : `the dataset has no ${name}.tif — that chunk is genuinely absent, ` +
                "which for a DEM usually means ocean",
          );
        } catch (cause) {
          if (cancelled) return;
          // The likeliest cause by a wide margin, per the package's own notes.
          setDiagnosis(
            `${name}.tif could not be read (${
              cause instanceof Error ? cause.message : String(cause)
            }) — the chunk may well exist; check the Elevation dataset is added ` +
              "as a Resource on the application in Developer Console",
          );
        }

        setError(null);
        // A counter rather than a boolean: recentring the board warms a new
        // set of cells, and the sampler's identity has to change so that
        // anything memoised on it recomputes.
        setReady((n) => n + 1);
      } catch (cause) {
        if (cancelled) return;
        setError(
          `${cause instanceof Error ? cause.message : String(cause)} — check the ` +
            "Elevation dataset is added as a Resource on the application in " +
            "Developer Console; the read scope alone is not enough",
        );
      }
    })();

    return () => {
      cancelled = true;
    };
    // `centre` is covered by `key`; see the note there.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, key]);

  // ⚠ NOT DISPOSED HERE. The map's extension created this source and will
  // dispose it with the map; disposing it from a consumer would drop the cells
  // out from under the terrain mesh that is still drawing them.

  const heightAt = useMemo(() => {
    if (!source || ready === 0) return null;
    // ⚠ (lon, lat), AND THE BOARD SPEAKS (lat, lng). Transposing these does
    // not throw — it reads a height from somewhere else on the planet and
    // returns a plausible number — so the swap happens exactly here and
    // nowhere else.
    return (point: LatLng) => source.heightAtLoaded(point.lng, point.lat);
  }, [source, ready]);

  /**
   * Is there real data under the board, or only resident emptiness?
   *
   * One finite height among the warmed points is enough: a board straddling
   * the edge of the DEM is still worth playing, and `lineOfSight` already
   * declines the individual shots that cross into NaN.
   */
  const hasCoverage = useMemo(() => {
    if (!heightAt) return false;
    return warmPoints(centre).some((p) =>
      Number.isFinite(heightAt({ lat: p.lat, lng: p.lon })),
    );
    // `centre` is covered by `key`, as above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [heightAt, key]);

  return { heightAt, isLoaded: heightAt != null, hasCoverage, cell, diagnosis, error };
}
