// ── bgws/data/unitImagePaths.ts ────────────────────────────────────────────
// Which media-set paths might hold an asset's photograph.
//
// Pure, and separate from unitImages.ts for the reason that file cannot be:
// fetching needs the client, the client reads <meta> tags at module load, and
// anything importing it is untestable. The ordering is the part worth testing.

/**
 * Extensions worth trying, most likely first.
 *
 * Every file in the set today is a .jpg; png is kept as a cheap hedge and
 * jpeg was dropped after it cost a third of every failed lookup and matched
 * nothing. A miss costs one request per candidate, so the list is short on
 * purpose.
 */
const EXTENSIONS = ["jpg", "png"] as const;

/**
 * Paths worth trying, in order.
 *
 * The Unit Images media set was filled from two directions and its filenames
 * show it: most items are named after an asset's DISPLAY NAME with spaces
 * turned into underscores (`Germ_Leopard_1a5.jpg`, `Colt_9_Mm_Smg.jpg`), and
 * some after the ASSET ID, whose useful part is the last segment of a path
 * like `tankmodels/germ_leopard_1a5`.
 *
 * Display name first, because that is the convention most of the set follows.
 * Case is preserved and matters: `M24.jpg` is a display name, `m24.jpg` would
 * be an asset id, and trying them in the wrong order finds the wrong
 * photograph or none.
 */
export function imageCandidates(assetId: string, displayName: string): string[] {
  const names: string[] = [];

  const fromDisplay = displayName.trim().replace(/\s+/g, "_");
  if (fromDisplay) names.push(fromDisplay);

  const bare = assetId.split("/").pop() ?? "";
  if (bare) names.push(bare);
  // The full path too: a media set path may contain directories, and the ids
  // in this source are themselves paths.
  if (assetId && assetId !== bare) names.push(assetId);

  const candidates: string[] = [];
  for (const name of names) {
    for (const extension of EXTENSIONS) {
      const path = `${name}.${extension}`;
      if (!candidates.includes(path)) candidates.push(path);
    }
  }
  return candidates;
}
