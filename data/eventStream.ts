// ── bgws/data/eventStream.ts ───────────────────────────────────────────────
// The real-time feed, streamed to a Foundry stream as it happens.
//
// One row per feed line (realtime/engine/feed.ts, `streamRow`), sent in
// batches every second or so through Foundry's streams API — on
// accenture.palantirfoundry.com/api/, which the app's CSP (and the workspace
// preview's) already allows.
//
// NEVER IN THE GAME'S WAY. Rows are queued and sent in the background; a
// failed batch is retried a couple of times and then dropped, with the
// reason kept for the screen. The clock never waits for the stream.
//
// NEEDS, in Developer Console: the stream's dataset as a permitted resource,
// and the streams write scope (api:streams-write) on the app's client.

import { platformClient } from "@/client";

import type { PublishRecords } from "./eventStreamQueue";

/** The event-log stream. */
export const EVENT_STREAM_RID = "ri.foundry.main.dataset.032c716d-b571-4ca4-a683-e4e555757d24";
export const EVENT_STREAM_BRANCH = "master";

/**
 * Stream radio messages too. Off until the stream's schema has the `message`
 * column (docs/EVENT_STREAM.md): a row with a field the stream does not know
 * would be refused, and with it the whole batch.
 */
export const STREAM_MESSAGES = false;

/**
 * Publish to a Foundry stream with the app's own credentials.
 *
 * Foundry API v2, Streams: POST
 * /api/v2/highScale/streams/datasets/{datasetRid}/streams/{branch}/publishRecords
 * with {"records": [...]}. It is a preview endpoint, hence `preview=true`.
 * Timestamps go as epoch milliseconds.
 */
export function foundryStreamPublisher(datasetRid = EVENT_STREAM_RID, branch = EVENT_STREAM_BRANCH): PublishRecords {
  // The platform client's context: where Foundry is and a token for it.
  const ctx = platformClient as unknown as { baseUrl: string; tokenProvider: () => Promise<string> };
  return async (rows) => {
    const base = ctx.baseUrl.replace(/\/+$/, "");
    const url =
      `${base}/api/v2/highScale/streams/datasets/${encodeURIComponent(datasetRid)}` +
      `/streams/${encodeURIComponent(branch)}/publishRecords?preview=true`;
    const response = await fetch(url, {
      method: "POST",
      headers: { Authorization: `Bearer ${await ctx.tokenProvider()}`, "Content-Type": "application/json" },
      body: JSON.stringify({ records: rows }),
    });
    if (!response.ok) {
      const text = await response.text().catch(() => "");
      throw new Error(`stream ${response.status}: ${text.slice(0, 300) || response.statusText}`);
    }
  };
}
