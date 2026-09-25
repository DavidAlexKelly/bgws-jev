// ── bgws/data/unitImageIndex.ts ────────────────────────────────────────────
// [SIM] L6 unit_image_index: which assets have a photograph, and where.
//
// This replaces guessing. The previous approach built up to nine candidate
// filenames per asset and asked the media set for each in turn until one
// answered, which meant:
//
//   * up to nine round trips per asset, most of them deliberate 404s;
//   * no way to ask "which assets have images?" at all — the answer only
//     existed one asset at a time, after a network call, so it could never be
//     a filter;
//   * a 404 and a broken call looked alike, so a missing Resource grant
//     showed up as "this tank has no photograph".
//
// The index is built by a transform that lists the media set once. 799 of the
// 2,258 assets have an image. The path here is exact, so the browser makes one
// call and only when it has something to fetch.
//
// Pure: types, query builders, row parsing. The Foundry call is in
// ./unitImagesClient.

import { asNumber, asText, columnIndex, type Table } from "../../../shared/lib/csv";
import { sqlLiteral } from "../../../shared/lib/sql";

/** [SIM] L6 unit_image_index. */
export const UNIT_IMAGE_INDEX_RID =
  "ri.foundry.main.dataset.73b31f66-55d9-42e9-b2e2-26685fd33d1c";

/** Which of the media set's four naming conventions found the file. */
export type ImageMatchMethod =
  | "display_name"
  | "asset_id_path"
  | "asset_id_flat"
  | "asset_id_basename";

export interface UnitImageRow {
  assetId: string;
  imagePath: string;
  mediaItemRid: string | null;
  matchMethod: ImageMatchMethod | null;
}

const COLUMNS = ["asset_id", "image_path", "media_item_rid", "match_method"].join(", ");

export function buildImageIndexQuery(assetIds: string[]): string {
  // No ids means no query worth running. Returning a statement that matches
  // everything would pull the whole index to answer a question nobody asked.
  if (!assetIds.length) return "";

  return [
    `SELECT ${COLUMNS}`,
    `FROM \`${UNIT_IMAGE_INDEX_RID}\``,
    `WHERE asset_id IN (${assetIds.map(sqlLiteral).join(", ")})`,
    `LIMIT ${assetIds.length}`,
  ].join("\n");
}

export function parseImageIndex(table: Table): UnitImageRow[] {
  const at = columnIndex(table);
  const rows: UnitImageRow[] = [];

  for (const row of table.rows) {
    const assetId = asText(row[at.asset_id]);
    const imagePath = asText(row[at.image_path]);
    // Both are non-null by a FAIL check on the dataset, so a row missing
    // either means something has gone wrong upstream rather than that this
    // asset has no image. Dropping it is right; inventing a path is not.
    if (!assetId || !imagePath) continue;

    rows.push({
      assetId,
      imagePath,
      mediaItemRid: asText(row[at.media_item_rid]),
      matchMethod: (asText(row[at.match_method]) as ImageMatchMethod | null) ?? null,
    });
  }

  return rows;
}

/** Keyed by asset id, for the detail pane and the list's thumbnails. */
export function indexByAssetId(rows: UnitImageRow[]): Map<string, UnitImageRow> {
  return new Map(rows.map((row) => [row.assetId, row]));
}

/** How many assets have an image, for the filter chip's count. */
export function buildImageCountQuery(): string {
  return [
    "SELECT COUNT(DISTINCT asset_id) AS n",
    `FROM \`${UNIT_IMAGE_INDEX_RID}\``,
  ].join("\n");
}

export function parseCount(table: Table): number {
  const at = columnIndex(table);
  const first = table.rows[0];
  if (!first) return 0;
  return asNumber(first[at.n]) ?? 0;
}
