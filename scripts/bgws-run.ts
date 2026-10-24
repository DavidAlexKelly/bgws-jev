// Play a BGWS real-time game headless, in Node (MULTI_DOMAIN_PLAN.md W0).
//
//   npm run bgws:run -- <file> [--until <seconds>] [--chunk <seconds>] [--events <out.jsonl>]
//
// <file> is a game record downloaded from the end-of-game screen
// (*.bgws-game.json), replayed exactly, or a scenario file
// (*.bgws-scenario.json), played with simple opening orders and the rules
// deciding. Prints the event stream's fingerprint: for a record, compare it
// with the one the browser showed.

import { readFileSync, writeFileSync } from "node:fs";
import { toJson } from "../src/apps/bgws/realtime/engine/fingerprint";
import type { RtLogEntry } from "../src/apps/bgws/realtime/engine/runner";
import { decodeRecord, GAME_RECORD_FORMAT, playRecord, recordFromScenario, type GameRecord } from "../src/apps/bgws/realtime/headless";
import { parseScenario } from "../src/apps/bgws/realtime/scenario";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

async function main(): Promise<void> {
  const file = process.argv.slice(2).find((a, i, all) => !a.startsWith("--") && !(i > 0 && all[i - 1].startsWith("--")));
  if (!file) {
    console.error("usage: npm run bgws:run -- <game record or scenario file> [--until <s>] [--chunk <s>] [--events <out.jsonl>]");
    process.exit(2);
  }
  const text = readFileSync(file, "utf8");
  let record: GameRecord;
  if (text.includes(`"${GAME_RECORD_FORMAT}"`)) {
    const { record: read, errors } = decodeRecord(text);
    if (!read) throw new Error(errors.join("\n"));
    record = read;
    if (record.deciders !== "rules") console.warn("note: Jev decided in this game; replaying with the rules, so it will differ.");
  } else {
    const { scenario, errors } = parseScenario(JSON.parse(text));
    if (!scenario) throw new Error(errors.join("\n"));
    record = recordFromScenario(scenario);
  }

  const events: RtLogEntry[] = [];
  const out = arg("events");
  const started = Date.now();
  const result = await playRecord(record, {
    // A record saved mid-game replays to the same moment, so the two can be compared.
    untilS: arg("until") ? Number(arg("until")) : record.played?.time,
    chunkS: arg("chunk") ? Number(arg("chunk")) : undefined,
    onEntry: out ? (entry) => events.push(entry) : undefined,
  });
  if (out) writeFileSync(out, events.map((e) => toJson(e)).join("\n") + "\n");

  const summary = {
    name: record.name,
    fingerprint: result.fingerprint,
    events: result.entries,
    endedAt: result.time,
    over: result.state.over ?? null,
    wallMs: Date.now() - started,
    ...(record.played
      ? {
          browser: record.played,
          // Only meaningful when both ran to the same point.
          sameAsBrowser: record.played.time === result.time ? record.played.fingerprint === result.fingerprint : "played to different times",
        }
      : {}),
  };
  console.log(JSON.stringify(summary, null, 2));
}

main().catch((e: unknown) => {
  console.error(e instanceof Error ? e.message : e);
  process.exit(1);
});
