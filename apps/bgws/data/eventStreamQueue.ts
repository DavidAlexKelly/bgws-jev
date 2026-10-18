// ── bgws/data/eventStreamQueue.ts ──────────────────────────────────────────
// Rows for the event stream, queued and sent in batches in the background.
// Kept apart from eventStream.ts (which needs the Foundry client) so it can
// be tested without one. A failing stream never holds the game up: a batch
// is retried a couple of times, then dropped, and the reason kept.

import type { StreamRow } from "../realtime/engine/feed";

/** Send one batch of rows; throws on failure. */
export type PublishRecords = (rows: StreamRow[]) => Promise<void>;

export interface EventStreamStatus {
  queued: number;
  sent: number;
  dropped: number;
  lastError: string | null;
}

/** Queues rows and sends them in batches, in the background. */
export class EventStreamQueue {
  private queue: StreamRow[] = [];
  private timer: ReturnType<typeof setInterval> | null = null;
  private sending = false;
  private attempts = 0;
  sent = 0;
  dropped = 0;
  lastError: string | null = null;

  constructor(
    private readonly publish: PublishRecords,
    private readonly options: { intervalMs?: number; maxBatch?: number; maxQueue?: number; retries?: number } = {},
  ) {}

  push(row: StreamRow): void {
    this.queue.push(row);
    const max = this.options.maxQueue ?? 5000;
    if (this.queue.length > max) {
      // A stream that has been failing for a long time: keep the newest.
      this.dropped += this.queue.length - max;
      this.queue.splice(0, this.queue.length - max);
    }
  }

  start(): void {
    if (this.timer) return;
    this.timer = setInterval(() => void this.flush(), this.options.intervalMs ?? 1500);
  }

  /** Send what is queued, then stop. */
  async stop(): Promise<void> {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    while (this.queue.length && !this.sending) {
      const before = this.queue.length;
      await this.flush();
      if (this.queue.length >= before) break;
    }
  }

  async flush(): Promise<void> {
    if (this.sending || this.queue.length === 0) return;
    this.sending = true;
    const batch = this.queue.slice(0, this.options.maxBatch ?? 200);
    try {
      await this.publish(batch);
      this.queue.splice(0, batch.length);
      this.sent += batch.length;
      this.attempts = 0;
      this.lastError = null;
    } catch (err) {
      this.lastError = err instanceof Error ? err.message : String(err);
      this.attempts += 1;
      if (this.attempts > (this.options.retries ?? 2)) {
        this.queue.splice(0, batch.length);
        this.dropped += batch.length;
        this.attempts = 0;
      }
    } finally {
      this.sending = false;
    }
  }

  get status(): EventStreamStatus {
    return { queued: this.queue.length, sent: this.sent, dropped: this.dropped, lastError: this.lastError };
  }
}
