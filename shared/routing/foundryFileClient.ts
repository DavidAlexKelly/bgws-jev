// src/shared/routing/foundryFileClient.ts
//
// Shared FoundryFileClient used by the terrain routing stack (TiledRouter and
// TerrainRaster). Previously this was defined inline inside MapWrapper, which
// meant any other component wanting terrain data had to duplicate it.
//
// Both useRouter() and useTerrainRaster() keep module-level singletons keyed on
// nothing but "first caller wins", so sharing one stable client instance across
// components guarantees the raster/router are loaded exactly once.

import { Files } from "@osdk/foundry.datasets";
import { platformClient } from "../../client";
import type { FoundryFileClient } from "./tiledRouter";

/**
 * Reads a file out of a Foundry dataset as an ArrayBuffer.
 *
 * NOTE: the Files API requires the OAuth application to have explicit
 * resource-level access to the dataset RID (Developer Console → app →
 * Resources) — the api:use-datasets-read scope alone is not sufficient.
 */
export const foundryFileClient: FoundryFileClient = {
  async getFileContent(datasetRid: string, filePath: string): Promise<ArrayBuffer> {
    if (!filePath) throw new Error(`[FoundryFileClient] filePath is ${String(filePath)}`);
    const response = await Files.content(platformClient, datasetRid, filePath);
    // Files.content returns a Blob — convert to ArrayBuffer
    if (response instanceof Blob) return response.arrayBuffer();
    // If it returns a Response (some OSDK versions), read its body
    if (response && typeof (response as Response).arrayBuffer === "function") {
      return (response as Response).arrayBuffer();
    }
    throw new Error(`[FoundryFileClient] unexpected response type for ${filePath}`);
  },
};
