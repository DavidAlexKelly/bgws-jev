import { useEffect, useState } from 'react';
import { TiledRouter, type FoundryFileClient } from './tiledRouter';
import { DEFAULT_TERRAIN } from './terrainDatasets';

// Keyed by dataset for the same reason as the terrain raster: a single global
// router meant a scenario on different terrain kept routing on the first one
// loaded, which fails silently — every route just refuses to leave the map.
const routers = new Map<string, TiledRouter>();
const inits = new Map<string, Promise<TiledRouter>>();

/**
 * React hook providing the tiled router for a pathfinding dataset.
 *
 * @param datasetRid Tiled pathfinding graph to use; defaults to the default
 *                   scenario's (see terrainDatasets.ts).
 */
export function useRouter(
  client: FoundryFileClient | null,
  datasetRid: string = DEFAULT_TERRAIN.routingRid,
): {
  router: TiledRouter | null;
  isReady: boolean;
  error: Error | null;
} {
  const [isReady, setIsReady] = useState(routers.has(datasetRid));
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    if (!client) return;
    let cancelled = false;
    setIsReady(routers.has(datasetRid));
    setError(null);

    if (!inits.has(datasetRid)) {
      const r = new TiledRouter({ datasetRid, client });
      inits.set(
        datasetRid,
        r
          .init()
          .then(() => {
            routers.set(datasetRid, r);
            return r;
          })
          .catch((e) => {
            // Don't cache the failed promise — allow retry
            console.error(`[useRouter] router init failed for ${datasetRid}:`, e);
            inits.delete(datasetRid);
            throw e;
          }),
      );
    }

    inits
      .get(datasetRid)!
      .then(() => { if (!cancelled) setIsReady(true); })
      .catch((e) => {
        if (!cancelled) setError(e instanceof Error ? e : new Error(String(e)));
      });

    return () => { cancelled = true; };
  }, [client, datasetRid]);

  return { router: routers.get(datasetRid) ?? null, isReady, error };
}
