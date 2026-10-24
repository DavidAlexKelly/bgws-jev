// The real-time engine's Web Worker (runnerHandle.ts spawns it). All the
// work is in workerHost.ts, which is tested without a worker.

import { hostRunner, type MessageEndpoint } from "./workerHost";

hostRunner(self as unknown as MessageEndpoint);
