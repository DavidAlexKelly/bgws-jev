// ── realtime/world/environment.ts ────────────────────────────────────────
// Light and weather (MULTI_DOMAIN_PLAN.md §4.2): what time of day it is, and
// what that and the weather do to each kind of sensor, to flying and to
// small craft. Thermal crossover — dawn and dusk, when vehicles and ground
// are the same temperature — and bad weather are when assaults happen
// [ref R4.1].

import type { SensorKind } from "../catalogue/schema";
import { wfig } from "./figures";
import type { Environment } from "./types";

export const DEFAULT_ENVIRONMENT: Environment = {
  startHour: 10,
  visibilityKm: 20,
  cloudBaseM: 2000,
  cloudCover: 0.3,
  precipitation: 0,
  windMs: 4,
  seaState: 2,
};

/** Hour of the day (0–24) at sim time `time`. */
export function hourAt(env: Environment, time: number): number {
  return (((env.startHour + time / 3600) % 24) + 24) % 24;
}

export function isNight(env: Environment, time: number): boolean {
  const h = hourAt(env, time);
  return h < 6 || h >= 19;
}

/** Within an hour of dawn or dusk: thermal crossover. */
export function inCrossover(env: Environment, time: number): boolean {
  const h = hourAt(env, time);
  return Math.abs(h - 6.5) < 1 || Math.abs(h - 18.5) < 1;
}

/** Multiplier on a sensor's detection hazard from light and weather, and the range beyond which it sees nothing. */
export function sensorConditions(env: Environment, time: number, kind: SensorKind): { factor: number; maxRangeM: number } {
  const night = isNight(env, time);
  const wet = 1 - 0.5 * env.precipitation;
  switch (kind) {
    case "eye":
    case "optic":
    case "eo":
      return { factor: (night ? wfig("world/environment.NIGHT_DETECT") : 1) * wet, maxRangeM: env.visibilityKm * 1000 };
    case "thermal":
    case "irst":
      return { factor: (inCrossover(env, time) ? wfig("world/environment.CROSSOVER_THERMAL") : 1) * (1 - 0.3 * env.precipitation), maxRangeM: Infinity };
    case "radar":
    case "cbr":
      return { factor: 1 - 0.1 * env.precipitation, maxRangeM: Infinity };
    default:
      return { factor: 1, maxRangeM: Infinity };
  }
}

/**
 * The land engine's spotting hazard and range under this light and weather.
 * Land units have both eyes and thermal sights; they use the better of the
 * two, so the night costs less than it would unaided.
 */
export function landSightConditions(env: Environment, time: number): { factor: number; maxRangeM: number } {
  const eye = sensorConditions(env, time, "eye");
  const thermal = sensorConditions(env, time, "thermal");
  return { factor: Math.max(eye.factor, thermal.factor * 0.8), maxRangeM: isNight(env, time) ? Infinity : eye.maxRangeM };
}

/** Can a small multirotor drone fly in this? */
export function smallDronesFly(env: Environment): boolean {
  return env.windMs <= wfig("world/environment.DRONE_MAX_WIND_MS") && env.precipitation < 0.8;
}

/** Chance an electro-optical satellite pass sees the ground through the cloud. */
export function eoClearChance(env: Environment): number {
  return Math.max(0, 1 - env.cloudCover);
}

/** Small craft (USVs, fast boats) can operate in this sea. */
export function smallCraftOperate(env: Environment): boolean {
  return env.seaState <= 4;
}
