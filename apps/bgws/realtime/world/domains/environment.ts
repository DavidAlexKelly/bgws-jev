// ── world/domains/environment.ts ─────────────────────────────────────────
// Light and weather change by the hour in long games. A scenario sets the
// start; this drifts the weather slowly and deterministically from the dice,
// so a three-day offensive sees nights, rain and clear spells.

import { registerSystem } from "../step";

/** How often the weather is looked at again, s. */
const WEATHER_STEP_S = 3600;

registerSystem({
  name: "environment",
  order: 10,
  when: (c) => c.time > 0 && c.time % WEATHER_STEP_S === 0 && (c.config.scope?.durationMin ?? 0) > 360,
  run(c) {
    const env = c.world.env;
    // A random walk in each, bounded: it never jumps from clear to storm in an hour.
    const step = (value: number, size: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, value + (c.rng.int(201) / 100 - 1) * size));
    const next = {
      ...env,
      cloudCover: step(env.cloudCover, 0.15, 0, 1),
      precipitation: env.cloudCover > 0.7 ? step(env.precipitation, 0.2, 0, 1) : Math.max(0, env.precipitation - 0.2),
      windMs: step(env.windMs, 2, 0, 20),
    };
    next.visibilityKm = Math.max(1, 25 - 20 * next.precipitation - 5 * next.cloudCover);
    (c.world as { env: typeof env }).env = next;
    c.log({ side: "neutral", domain: "environment", kind: "weather", text: `weather: cloud ${Math.round(next.cloudCover * 100)}%, rain ${Math.round(next.precipitation * 100)}%, wind ${Math.round(next.windMs)} m/s, visibility ${Math.round(next.visibilityKm)} km` });
  },
});
