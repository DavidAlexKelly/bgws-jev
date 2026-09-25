import { describe, expect, it } from "vitest";

import { parseCsv } from "../../../shared/lib/csv";
import {
  buildImageIndexQuery,
  indexByAssetId,
  parseCount,
  parseImageIndex,
} from "./unitImageIndex";

// Real rows, copied from the built index.
const CSV = [
  "asset_id,image_path,media_item_rid,match_method",
  "tankmodels/germ_leopard_1a5,Germ_Leopard_1a5.jpg,ri.mio.main.media-item.1,display_name",
  "tankmodels/cn_cm11,tankmodels_cn_cm11.jpg,ri.mio.main.media-item.2,asset_id_flat",
].join("\n");

describe("the index query", () => {
  it("asks only for the assets on screen", () => {
    const query = buildImageIndexQuery(["humans/m24", "tankmodels/cn_cm11"]);
    expect(query).toContain("asset_id IN ('humans/m24', 'tankmodels/cn_cm11')");
    // Bounded by what was asked for: one row per asset is guaranteed by a
    // primary key check on the dataset.
    expect(query).toContain("LIMIT 2");
  });

  it("returns an empty string for no assets rather than a statement", () => {
    // A query with no WHERE would pull the whole index to answer a question
    // nobody asked. The client checks for this and skips the round trip.
    expect(buildImageIndexQuery([])).toBe("");
  });

  it("escapes a quote in an asset id", () => {
    expect(buildImageIndexQuery(["a'b"])).toContain("'a''b'");
  });
});

describe("parsing", () => {
  const rows = parseImageIndex(parseCsv(CSV));

  it("reads both naming conventions", () => {
    expect(rows).toHaveLength(2);
    expect(rows[0].matchMethod).toBe("display_name");
    expect(rows[1].matchMethod).toBe("asset_id_flat");
  });

  it("keeps the exact path, which is the point of the index", () => {
    expect(rows[1].imagePath).toBe("tankmodels_cn_cm11.jpg");
  });

  it("drops a row with no path instead of inventing one", () => {
    // Both columns are non-null by a FAIL check upstream, so a row missing
    // either means something went wrong there — not that this asset has no
    // image. Guessing a path would turn that into a confident 404.
    const table = parseCsv(
      ["asset_id,image_path,media_item_rid,match_method", "humans/m24,,ri.x,display_name"].join(
        "\n",
      ),
    );
    expect(parseImageIndex(table)).toEqual([]);
  });

  it("drops a row with no asset id", () => {
    const table = parseCsv(
      ["asset_id,image_path,media_item_rid,match_method", ",M24.jpg,ri.x,display_name"].join("\n"),
    );
    expect(parseImageIndex(table)).toEqual([]);
  });
});

describe("keying by asset", () => {
  it("makes the lookup the list and detail pane both need", () => {
    const byId = indexByAssetId(parseImageIndex(parseCsv(CSV)));
    expect(byId.get("tankmodels/cn_cm11")?.matchMethod).toBe("asset_id_flat");
    expect(byId.has("ships/uk_hood")).toBe(false);
  });
});

describe("counts", () => {
  it("reads a scalar count", () => {
    expect(parseCount(parseCsv(["n", "799"].join("\n")))).toBe(799);
  });

  it("treats no rows as zero rather than throwing", () => {
    expect(parseCount(parseCsv("n"))).toBe(0);
  });
});
