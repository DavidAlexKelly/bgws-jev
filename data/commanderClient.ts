// ── bgws/data/commanderClient.ts ───────────────────────────────────────────
// The one impure function behind an LLM commander.
//
// rules/llmCommander.ts builds the prompt and parses the reply, both pure and
// tested without a model. All it needs from the outside world is a
// `(prompt) => Promise<string>`. This provides it.
//
// ⚠ QUERIES ARE ONTOLOGY-SCOPED. That one fact cost four rounds of debugging.
//
// I called `/v2/functions/queries/{name}/execute` — the FUNCTIONS service.
// The real endpoint is
// `/v2/ontologies/{ontology}/queries/{name}/execute` — the ONTOLOGIES
// service. Different URL, different SDK package, and a 404 QueryNotFound
// that reads exactly like a function that was never published.
//
// It had been published the whole time. Along the way that 404 was blamed on
// version resolution, on @Function vs @Query, on the wrong ontology and on a
// missing OAuth scope. Only the @Function/@Query one was a real defect; the
// rest were a correct error message about the wrong address.
//
// This now uses the generated OSDK, which is what the function's own
// documentation page recommends and what `generateOrbatV2` in the orbatManager
// app already does. The SDK knows the ontology, the api name and the
// parameter types, so none of them can be wrong again.

import * as sdk from "@nxg-simulation-execution-testing/sdk";
import { bgwsCommanderTurn } from "@nxg-simulation-execution-testing/sdk";

import { client } from "../../../client";
import type { ModelCall } from "../rules/llmCommander";

/** Models the published query will accept. Kept in step with it by hand. */
export const COMMANDER_MODELS = ["claude-sonnet-4-6", "gpt-5-2"] as const;

export type CommanderModelName = (typeof COMMANDER_MODELS)[number];

/** Named for error messages; the SDK holds the real api name. */
export const COMMANDER_QUERY = "bgwsCommanderTurn";

/**
 * Which published query a call goes to. See llmfunctions.ts for the briefs.
 *
 *   turn     orders carried out exactly as written (Jev off)
 *   turnJev  orders Jev will carry out and may adapt (Jev on)
 *   decide   one escalated decision Jev was unsure about (Jev on)
 */
export type CommanderQueryKind = "turn" | "turnJev" | "decide";

const QUERY_NAMES: Record<CommanderQueryKind, string> = {
  turn: "bgwsCommanderTurn",
  turnJev: "bgwsCommanderTurnJev",
  decide: "bgwsCommanderDecide",
};

/**
 * The query to call, falling back to bgwsCommanderTurn while the SDK has not
 * been regenerated.
 *
 * ⚠ LOOKED UP BY NAME, NOT IMPORTED, ON PURPOSE. A named import of a query
 * the SDK does not have yet fails the BUILD, so the app would be unusable in
 * the gap between merging this and republishing the functions. Looked up at
 * runtime, the new queries simply take over once they exist, and until then
 * the old one answers — with the old brief, which is the only cost.
 */
function queryFor(kind: CommanderQueryKind): { query: typeof bgwsCommanderTurn; name: string } {
  const wanted = QUERY_NAMES[kind];
  const found = (sdk as unknown as Record<string, unknown>)[wanted];
  if (found) return { query: found as typeof bgwsCommanderTurn, name: wanted };
  if (kind !== "turn" && typeof console !== "undefined") {
    console.warn?.(
      `[commander] ${wanted} is not in the generated SDK yet; using ${COMMANDER_QUERY}. ` +
        "Publish llmfunctions.ts and regenerate the SDK to use it.",
    );
  }
  return { query: bgwsCommanderTurn, name: COMMANDER_QUERY };
}

export class CommanderCallError extends Error {
  constructor(
    message: string,
    /** True when it looks like a missing scope or resource grant, not a bug. */
    readonly permissionDenied: boolean,
  ) {
    super(message);
    this.name = "CommanderCallError";
  }
}

function looksLikePermissionProblem(err: unknown): boolean {
  const text = String((err as { message?: string })?.message ?? err ?? "");
  return /permission|denied|403|forbidden|scope|unauthor/i.test(text);
}

/**
 * Everything the platform actually said, not just the status line.
 *
 * The OSDK's default message is "Failed to fetch 400 Bad Request", which is
 * the one piece of information that does not help. Foundry returns an
 * errorName, an errorInstanceId and named parameters in the body, and those
 * name the real problem. Adding this turned a guessing game into a sequence
 * of one-line fixes, and it should have been the first change rather than the
 * fourth.
 */
function describeApiError(err: unknown): string {
  const error = err as {
    message?: string;
    errorName?: string;
    errorCode?: string;
    errorInstanceId?: string;
    parameters?: unknown;
    body?: unknown;
  };

  const parts: string[] = [];
  if (error?.errorCode) parts.push(String(error.errorCode));
  if (error?.errorName) parts.push(String(error.errorName));
  if (error?.message) parts.push(String(error.message));
  if (error?.parameters) {
    try {
      parts.push(JSON.stringify(error.parameters));
    } catch {
      // A parameter bag that will not serialise is not worth failing over.
    }
  }
  if (parts.length === 0 && error?.body) {
    try {
      parts.push(JSON.stringify(error.body));
    } catch {
      /* as above */
    }
  }
  if (error?.errorInstanceId) parts.push(`instance ${String(error.errorInstanceId)}`);

  return parts.length > 0 ? parts.join(" · ") : String(err);
}

/**
 * A ModelCall that runs the published commander query.
 *
 * The model and directive are bound here rather than passed per call, because
 * a side's commander does not change mid-game and the Commander interface
 * should not have to know about either.
 */
export function foundryModelCall(
  model: CommanderModelName,
  directive: string,
  kind: CommanderQueryKind = "turn",
): ModelCall {
  return async (prompt: string): Promise<string> => {
    const { query, name } = queryFor(kind);
    let result: unknown;
    try {
      result = await client(query).executeFunction({
        prompt,
        model,
        directive,
      });
    } catch (err) {
      const denied = looksLikePermissionProblem(err);
      throw new CommanderCallError(
        denied
          ? `No access to the "${name}" query. Add [NXG] BGWS LLM ` +
            "Commanders as a permitted resource on this app in Developer Console."
          : `Commander query failed: ${describeApiError(err)}`,
        denied,
      );
    }

    if (typeof result !== "string") {
      // The query's contract is a string. Anything else means the published
      // version and this caller have drifted, which is worth saying plainly
      // rather than coercing into a reply the parser will reject.
      throw new CommanderCallError(
        `Expected a string from ${name}, got ${typeof result}. ` +
          "The published query and this caller may have drifted.",
        false,
      );
    }
    return result;
  };
}
