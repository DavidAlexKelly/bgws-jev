// src/shared/routing/terrainDatasets.ts
//
// Which terrain a scenario is fought on.
//
// Routing and trafficability come from two datasets built per area of
// operations: a cost raster (terrain_grid.bin) and a tiled pathfinding graph
// (manifest.json + tile_*.bin). Both RIDs used to be hard-coded to the Narva
// corridor, which was fine while NEXGEN was the only scenario — it is not fine
// now that the default scenario is Operation BALTIC SHIELD, fought around
// Kaliningrad, roughly 450 km south-west. Routing a mech battalion through
// Estonian terrain rasters for a Polish corridor produces confident nonsense.
//
// The mapping is by scenario rather than by operation because the planner knows
// its scenario id long before it has resolved an operation, and because
// NxgOperation.terrainName is the simulator's terrain (e.g. "kaliningrad"),
// which is a different thing from the pathfinding build we happen to have.

export interface TerrainDatasets {
  key: string;
  /** For logs and debug overlays. */
  label: string;
  /** Cost raster — see TerrainRaster. */
  rasterRid: string;
  /** Tiled pathfinding graph — see TiledRouter. */
  routingRid: string;
}

/** Kaliningrad oblast and the Polish–Lithuanian corridor. */
export const TERRAIN_KALININGRAD: TerrainDatasets = {
  key: "kaliningrad",
  label: "Kaliningrad / Baltic corridor",
  rasterRid: "ri.foundry.main.dataset.9d480ab9-bab2-4420-9d78-008b484cf28b",
  routingRid: "ri.foundry.main.dataset.d6e3fa39-94ca-4ec6-a272-b73df2ec79d5",
};

/** Narva corridor, north-east Estonia — the original NEXGEN area. */
export const TERRAIN_NARVA: TerrainDatasets = {
  key: "narva",
  label: "Narva corridor",
  rasterRid: "ri.foundry.main.dataset.594caffe-a93e-4226-901b-f6c0497842dd",
  routingRid: "ri.foundry.main.dataset.cb7bebcf-a89b-4774-814b-97f8128d72a8",
};

/**
 * Tracks DEFAULT_SCENARIO_ID in scenarioPlanner/shared/phaseColours.ts, which
 * is BALTIC SHIELD.
 */
export const DEFAULT_TERRAIN = TERRAIN_KALININGRAD;

const BY_SCENARIO: Record<string, TerrainDatasets> = {
  scenario_operation_baltic_shield: TERRAIN_KALININGRAD,
  scenario_nexgen_2026: TERRAIN_NARVA,
};

/**
 * Terrain for a scenario, falling back to the default.
 *
 * An unknown scenario gets Kaliningrad, which is right for anything authored
 * from the current default and merely wrong-but-visible for anything else —
 * routes will refuse to leave the map rather than silently mis-costing.
 */
export function terrainForScenario(scenarioId: string | undefined): TerrainDatasets {
  if (!scenarioId) return DEFAULT_TERRAIN;
  return BY_SCENARIO[scenarioId] ?? DEFAULT_TERRAIN;
}
