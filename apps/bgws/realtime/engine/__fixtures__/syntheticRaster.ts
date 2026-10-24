// Test fixture: a small terrain raster, as the Foundry dataset holds it.
// For tests only; nothing in the app imports it.

import { TerrainRaster } from "../../../../../shared/routing/terrainRaster";
import { TerrainFlag } from "../../../../../shared/routing/types";

/**
 * A terrain_grid.bin, as the dataset holds it: a 32-byte header and one flag
 * per cell. Open ground with blocks of forest and marsh, a road, and a river
 * along the north edge (wide enough to survive the loader's noise removal).
 */
export function syntheticRasterFile(): ArrayBuffer {
  const minLat = 54.0;
  const minLon = 18.3;
  const cellSize = 0.002;
  const numRows = 200;
  const numCols = 300;
  const buf = new ArrayBuffer(32 + numRows * numCols);
  const view = new DataView(buf);
  view.setFloat64(0, minLat, true);
  view.setFloat64(8, minLon, true);
  view.setFloat64(16, cellSize, true);
  view.setUint32(24, numRows, true);
  view.setUint32(28, numCols, true);
  const data = new Uint8Array(buf, 32);
  for (let r = 0; r < numRows; r++) {
    for (let c = 0; c < numCols; c++) {
      const block = ((r >> 3) + 2 * (c >> 3)) % 7;
      data[r * numCols + c] =
        r < 6 ? 255 : c === 150 ? TerrainFlag.Road : block === 0 ? TerrainFlag.Forest : block === 3 ? TerrainFlag.Marsh : TerrainFlag.Open;
    }
  }
  return buf;
}

export async function loadedRaster(): Promise<TerrainRaster> {
  const file = syntheticRasterFile();
  const raster = new TerrainRaster({ getFileContent: () => Promise.resolve(file.slice(0)) }, "ri.test.raster");
  await raster.load();
  return raster;
}
