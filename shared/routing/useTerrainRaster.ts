import { useEffect, useState } from 'react';
import { TerrainRaster } from './terrainRaster';
import type { FoundryFileClient } from './tiledRouter';
import { DEFAULT_TERRAIN } from './terrainDatasets';

// One raster per dataset, not one raster full stop. The previous single
// global meant the first scenario to load won: switching to a scenario on
// different terrain silently kept the old grid, and every trafficability
// check was answered from the wrong part of the world.
const rasters = new Map<string, TerrainRaster>();
const loads = new Map<string, Promise<void>>();

/**
 * React hook providing the TerrainRaster for a dataset, loaded once.
 *
 * @param datasetRid Terrain cost raster to use; defaults to the default
 *                   scenario's (see terrainDatasets.ts).
 */
export function useTerrainRaster(
  client: FoundryFileClient | null,
  datasetRid: string = DEFAULT_TERRAIN.rasterRid,
): {
  raster: TerrainRaster | null;
  isLoaded: boolean;
  error: Error | null;
} {
  const [, forceRender] = useState(0);
  const [isLoaded, setIsLoaded] = useState(rasters.get(datasetRid)?.isLoaded ?? false);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    setIsLoaded(rasters.get(datasetRid)?.isLoaded ?? false);
    setError(null);

    if (!loads.has(datasetRid)) {
      const raster = new TerrainRaster(client, datasetRid);
      rasters.set(datasetRid, raster);
      loads.set(
        datasetRid,
        raster.load().catch((e) => {
          console.error(`[useTerrainRaster] load failed for ${datasetRid}:`, e);
          loads.delete(datasetRid);
          rasters.delete(datasetRid);
          throw e;
        }),
      );
    }

    loads
      .get(datasetRid)!
      .then(() => {
        if (cancelled) return;
        setIsLoaded(true);
        forceRender((n) => n + 1);
      })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e : new Error(String(e)));
      });

    return () => { cancelled = true; };
  }, [client, datasetRid]);

  return { raster: rasters.get(datasetRid) ?? null, isLoaded, error };
}
