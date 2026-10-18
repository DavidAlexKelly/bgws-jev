// ── bgws/data/unitImages.ts ────────────────────────────────────────────────
// Photographs for assets, from the Unit Images media set.
//
// The path candidates and their ordering live in ./unitImagePaths, which is
// pure and tested. This file is the part that talks to Foundry.
//
// TWO THINGS THIS FILE GOT WRONG ONCE, BOTH WORTH KEEPING WRITTEN DOWN:
//
// 1. `getRidByPath` is a PREVIEW endpoint and returns 400 without
//    `preview: true`. The installed types say so — the function is marked
//    @beta and its query parameters include `preview` — and the first version
//    of this file omitted it, so every lookup failed before it ever searched.
//    `read` is @public in this version and needs no such flag.
//
// 2. The catch swallowed that 400 and reported it as "no image", which is the
//    precise failure this codebase warns about elsewhere: a broken call and an
//    absent file looked identical, and the UI confidently said the asset had
//    no photograph. Only a genuine not-found is a miss now; anything else is
//    surfaced, and is NOT cached, so fixing the cause does not need a reload.

import { MediaSets } from "@osdk/foundry.mediasets";

import { platformClient } from "@/client";
import { imageCandidates } from "./unitImagePaths";

export { imageCandidates };

/** Unit Images, in /Accenture/[DK] Project Space/Units. */
export const UNIT_IMAGES_MEDIA_SET_RID =
  "ri.mio.main.media-set.55817bd4-fc18-4b74-9ccd-82fe9af58b54";

/**
 * Mildata Images — photographs for the curated equipment tables.
 *
 * A SECOND set rather than more items in the first, because the two
 * catalogues name their assets differently and the lookups are therefore
 * different kinds of operation. Unit Images is searched by GUESSING up to nine
 * filename conventions from an asset id; this one is addressed EXACTLY, from
 * the `image_filename` the curated platform profile carries. No guessing, so
 * no wrong photograph on a near-miss.
 */
export const MILDATA_IMAGES_MEDIA_SET_RID =
  "ri.mio.main.media-set.4b3fc034-58fa-4ff7-a8d9-2514cf3e55b2";

// Object URLs are held rather than blobs: an <img src> can use them directly.
// Bounded, because a session that browsed every asset would otherwise pin
// every photograph it had seen in memory.
const MAX_CACHED_IMAGES = 250;

export interface ResolvedImage {
  /** Null means "looked, found nothing". */
  url: string | null;
  /** Which candidate matched, for the detail panel's provenance line. */
  path?: string;
  /**
   * Set when the lookup FAILED rather than came up empty — a missing Resource
   * grant, a preview flag the API wanted, a network fault. The difference
   * matters: one means this asset has no photograph, the other means we cannot
   * tell.
   */
  error?: string;
}

const cache = new Map<string, ResolvedImage>();
const inFlight = new Map<string, Promise<ResolvedImage>>();

function remember(key: string, entry: ResolvedImage): ResolvedImage {
  // A failure is never cached: it is usually configuration, and configuration
  // gets fixed while the page is open.
  if (entry.error) return entry;

  cache.set(key, entry);
  while (cache.size > MAX_CACHED_IMAGES) {
    const oldest = cache.keys().next().value as string | undefined;
    if (oldest === undefined) break;
    const evicted = cache.get(oldest);
    if (evicted?.url) URL.revokeObjectURL(evicted.url);
    cache.delete(oldest);
  }
  return entry;
}

/** A 404 means this path holds nothing. Everything else is a real failure. */
function isNotFound(error: unknown): boolean {
  const status = (error as { statusCode?: number })?.statusCode;
  if (status === 404) return true;
  const name = String((error as { errorName?: string })?.errorName ?? "");
  return /NotFound/i.test(name);
}

function describe(error: unknown): string {
  const status = (error as { statusCode?: number })?.statusCode;
  const name = (error as { errorName?: string })?.errorName;
  const message = (error as { message?: string })?.message;
  return [status && `HTTP ${status}`, name, message].filter(Boolean).join(" · ");
}

type Attempt = { url: string | null; error?: string };

async function fetchOne(mediaSetRid: string, path: string): Promise<Attempt> {
  let mediaItemRid: string | undefined;

  try {
    const located = await MediaSets.getRidByPath(platformClient, mediaSetRid, {
      mediaItemPath: path,
      // Required: the endpoint is in preview and answers 400 without it.
      preview: true,
    });
    mediaItemRid = located?.mediaItemRid;
  } catch (error) {
    if (isNotFound(error)) return { url: null };
    return { url: null, error: describe(error) };
  }

  if (!mediaItemRid) return { url: null };

  try {
    const response = await MediaSets.read(platformClient, mediaSetRid, mediaItemRid);
    return { url: URL.createObjectURL(await response.blob()) };
  } catch (error) {
    if (isNotFound(error)) return { url: null };
    return { url: null, error: describe(error) };
  }
}

/**
 * The image for an asset, or null when the set has none.
 *
 * Concurrent callers for the same asset share one request: a list row and a
 * detail pane both asking at once is the normal case, not an edge one.
 */
export async function resolveUnitImage(
  assetId: string,
  displayName: string,
): Promise<ResolvedImage> {
  const key = `${assetId}|${displayName}`;

  const cached = cache.get(key);
  if (cached) return cached;

  const existing = inFlight.get(key);
  if (existing) return existing;

  const request = (async () => {
    for (const path of imageCandidates(assetId, displayName)) {
      const attempt = await fetchOne(UNIT_IMAGES_MEDIA_SET_RID, path);
      // A failing endpoint fails for every candidate; trying the other eight
      // turns one broken call into nine and buries the cause in the console.
      if (attempt.error) return remember(key, { url: null, error: attempt.error });
      if (attempt.url) return remember(key, { url: attempt.url, path });
    }
    return remember(key, { url: null });
  })();

  inFlight.set(key, request);
  try {
    return await request;
  } finally {
    inFlight.delete(key);
  }
}

/**
 * The image at a path already known to exist.
 *
 * Preferred over resolveUnitImage now that [SIM] L6 unit_image_index exists:
 * the index was built by listing the media set, so the path is a fact rather
 * than a guess and one call replaces up to nine. A 404 here is therefore not
 * "no photograph" — it means the index and the media set have diverged, which
 * is worth surfacing rather than swallowing.
 *
 * resolveUnitImage is kept for the case where the index has no row, and as a
 * fallback while the index is not yet built on this branch.
 */
export async function resolveUnitImageByPath(path: string): Promise<ResolvedImage> {
  const key = `path|${path}`;

  const cached = cache.get(key);
  if (cached) return cached;

  const existing = inFlight.get(key);
  if (existing) return existing;

  const request = (async () => {
    const attempt = await fetchOne(UNIT_IMAGES_MEDIA_SET_RID, path);
    if (attempt.error) return remember(key, { url: null, error: attempt.error });
    if (attempt.url) return remember(key, { url: attempt.url, path });
    return remember(key, {
      url: null,
      error: `the index names ${path} but the media set has no such item`,
    });
  })();

  inFlight.set(key, request);
  try {
    return await request;
  } finally {
    inFlight.delete(key);
  }
}

/**
 * The photograph for a curated platform, addressed exactly.
 *
 * ⚠ THE PATH IS THE FILENAME PLUS ".png", AND THAT IS NOT A GUESS. The images
 * were converted to PNG without dropping their original extension, so
 * "Alvis FV4333 Stormer.jpg" is stored as "Alvis FV4333 Stormer.jpg.png" and
 * "BAe Systems EC-37B Compass Call.webp" as "...webp.png". The bare filename
 * is tried second, so the set can be re-exported without the double extension
 * and this keeps working.
 *
 * A miss here is a genuine "no photograph for this platform" — 13 of the 146
 * land rows have no filename at all — and is distinguished from a failed
 * lookup, same as everywhere else in this file.
 */
export async function resolveMildataImage(imageFilename: string): Promise<ResolvedImage> {
  const key = `mildata|${imageFilename}`;

  const cached = cache.get(key);
  if (cached) return cached;

  const existing = inFlight.get(key);
  if (existing) return existing;

  const request = (async () => {
    for (const path of [`${imageFilename}.png`, imageFilename]) {
      const attempt = await fetchOne(MILDATA_IMAGES_MEDIA_SET_RID, path);
      // A broken endpoint breaks for both candidates; trying the second turns
      // one failure into two and hides the cause.
      if (attempt.error) return remember(key, { url: null, error: attempt.error });
      if (attempt.url) return remember(key, { url: attempt.url, path });
    }
    return remember(key, { url: null });
  })();

  inFlight.set(key, request);
  try {
    return await request;
  } finally {
    inFlight.delete(key);
  }
}

/** For a hard refresh, and to release the object URLs. */
export function clearImageCache(): void {
  for (const entry of cache.values()) {
    if (entry.url) URL.revokeObjectURL(entry.url);
  }
  cache.clear();
  inFlight.clear();
}
