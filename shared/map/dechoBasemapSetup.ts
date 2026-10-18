/**
 * Binds this application's credentials and env vars to @acc/decho-basemap, and
 * describes the two basemaps this app uses.
 *
 * The library deliberately knows nothing about `import.meta.env` or
 * `@/client` — that is what makes it publishable — so everything
 * environment-shaped lives here. It is also the one place to look when a map
 * points at the wrong dataset.
 */

import {
  ASSET_STORE,
  PLANET_STORE,
  THEATRE_STORE,
  configureBasemap,
  createBasemap,
  type AssetStore,
  type FixedGridStore,
  type ManifestStore,
  type MaplibreLike,
  type BasemapHandle,
} from "@acc/decho-basemap";

import { auth, foundryUrl, platformClient } from "@/client";

const env = import.meta.env;

function trimmed(value: unknown): string | undefined {
  const s = typeof value === "string" ? value.trim() : "";
  return s === "" ? undefined : s;
}

/** Chunked planet basemap, used by /globe. */
export const planetStore: ManifestStore = {
  ...PLANET_STORE,
  datasetRid: trimmed(env.VITE_GLOBE_DATASET_RID) ?? PLANET_STORE.datasetRid,
  mediaSetRid: trimmed(env.VITE_GLOBE_MEDIASET_RID) ?? PLANET_STORE.mediaSetRid,
};

/** Northern-Nordic theatre basemap: Scenario Planner, ORBAT, composition. */
export const theatreStore: FixedGridStore = {
  ...THEATRE_STORE,
  datasetRid: trimmed(env.VITE_TILES_DATASET_RID) ?? THEATRE_STORE.datasetRid,
};

/** Glyphs and sprite. Unset dataset = no labels, which is a valid state. */
export const assetStore: AssetStore | null = trimmed(
  env.VITE_MAP_ASSETS_DATASET_RID,
)
  ? {
      datasetRid: trimmed(env.VITE_MAP_ASSETS_DATASET_RID) as string,
      spritePath: trimmed(env.VITE_MAP_SPRITE_PATH),
    }
  : ASSET_STORE;

let done = false;

/** Idempotent: safe to call from several entry points. */
export function setupBasemap(): void {
  if (done) return;
  done = true;

  configureBasemap({
    foundryUrl,
    // createPublicOauthClient returns a callable that resolves to the token.
    getToken: () => auth(),
    platformClient,
  });
}

/**
 * The theatre basemap, wired the way every non-globe map in this app expects.
 *
 * `protocol` and `sourceId` keep the previous names so the layer lists and any
 * style references built around "protomaps" keep working, and so two maps
 * mounted at once share one protocol registration (it is refcounted).
 *
 * Callers spread `styleFragment()` into their style, `attach(map)` to enable
 * neighbour prefetch, and `dispose()` on unmount.
 */
/**
 * There is no createPlanetBasemap() counterpart, deliberately.
 *
 * Every consumer of the planet store is a React map (/globe and the Scenario
 * Planner), and both render <DechoBasemap tiles={planetStore} assets={assetStore}
 * protocol="foundry-planet" />, which does all of this and owns the teardown
 * too. A second, imperative path would only be a copy waiting to drift.
 *
 * Note also that the planet source takes no `bounds`. THEATRE_STORE's bbox is a
 * real constraint (there is no data outside it); imposing one on a store with
 * global coverage would clamp a basemap that has data everywhere.
 */
export function createTheatreBasemap(
  maplibregl: MaplibreLike,
): Promise<BasemapHandle> {
  return createBasemap(maplibregl, {
    tiles: theatreStore,
    assets: assetStore,
    protocol: "foundry-pmtiles",
    sourceId: "protomaps",
    sourceOptions: {
      bounds: [
        THEATRE_STORE.bbox.minLon,
        THEATRE_STORE.bbox.minLat,
        THEATRE_STORE.bbox.maxLon,
        THEATRE_STORE.bbox.maxLat,
      ],
    },
  });
}
