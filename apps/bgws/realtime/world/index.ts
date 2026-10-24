// ── realtime/world/index.ts ──────────────────────────────────────────────
// Every domain of the world layer, loaded for its side effects: each module
// registers its systems (world/step.ts), its action handlers and trigger
// rules (world/actions.ts) and its world-creation hooks (world/create.ts).
// The order here does not matter; each system carries its own `order`.

import "./domains/environment";
