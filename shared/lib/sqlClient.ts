// ── shared/lib/sqlClient.ts ────────────────────────────────────────────────
// Thin wrapper over Foundry's SQL Queries API (/v2/sqlQueries).
//
// BGWS uses it because its equipment profiles, asset cards and curated assets
// are datasets, not object types, and it wants a filtered handful of rows out
// of a few thousand rather than the whole table. SQL pushes the filtering
// server-side so the browser only receives what it will use.
//
// ⚠ Requires the OAuth scopes `api:sql-queries-execute` and
// `api:sql-queries-read` on the Developer Console application AND every
// dataset being queried to be added there as a permitted resource. Without
// both, every call fails with a 403 — surfaced to the user by the caller via
// SqlError.permissionDenied.
//
// Moved over from the Mapplications repo with BGWS.

import { SqlQueries } from "@osdk/foundry.sqlqueries";
import { client } from "../../client";
import { parseCsv, type Table } from "./csv";

/** How long to wait for a single query before giving up. */
const QUERY_TIMEOUT_MS = 180_000;
/** Gap between getStatus polls while a query is running. */
const STATUS_POLL_MS = 750;

export class SqlError extends Error {
  constructor(
    message: string,
    /** True when the failure looks like a missing scope / resource grant. */
    readonly permissionDenied: boolean
  ) {
    super(message);
    this.name = "SqlError";
  }
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function looksLikePermissionProblem(err: unknown): boolean {
  const text = String((err as { message?: string })?.message ?? err ?? "");
  return /permission|denied|403|forbidden|scope|unauthor/i.test(text);
}

/**
 * Execute a SQL query and return its rows as CSV-parsed strings.
 *
 * CSV rather than the default Arrow: it needs no extra runtime dependency and
 * the result sets here are shaped by the query (tens of thousands of rows at
 * most), not by the raw table size.
 */
export async function runSql(query: string, signal?: AbortSignal): Promise<Table> {
  let status;
  try {
    status = await SqlQueries.execute(client, {
      query,
      serializationFormat: "CSV",
    });
  } catch (err) {
    throw new SqlError(
      `Failed to submit SQL query: ${String((err as Error)?.message ?? err)}`,
      looksLikePermissionProblem(err)
    );
  }

  if (status.type === "failed") {
    throw new SqlError(status.errorMessage, looksLikePermissionProblem(status.errorMessage));
  }
  if (status.type === "canceled") {
    throw new SqlError("Query was canceled", false);
  }

  const queryId = status.queryId;
  const deadline = Date.now() + QUERY_TIMEOUT_MS;

  const abandon = () => {
    void SqlQueries.cancel(client, queryId).catch(() => undefined);
  };

  while (status.type === "running") {
    if (signal?.aborted) {
      abandon();
      throw new DOMException("Query aborted", "AbortError");
    }
    if (Date.now() > deadline) {
      abandon();
      throw new SqlError(`Query timed out after ${QUERY_TIMEOUT_MS / 1000}s`, false);
    }

    await sleep(STATUS_POLL_MS);

    try {
      status = await SqlQueries.getStatus(client, queryId);
    } catch (err) {
      throw new SqlError(
        `Failed to poll SQL query: ${String((err as Error)?.message ?? err)}`,
        looksLikePermissionProblem(err)
      );
    }

    if (status.type === "failed") {
      throw new SqlError(status.errorMessage, looksLikePermissionProblem(status.errorMessage));
    }
    if (status.type === "canceled") {
      throw new SqlError("Query was canceled", false);
    }
  }

  if (signal?.aborted) throw new DOMException("Query aborted", "AbortError");

  try {
    const response = await SqlQueries.getResults(client, queryId);
    return parseCsv(await response.text());
  } catch (err) {
    throw new SqlError(
      `Failed to read SQL results: ${String((err as Error)?.message ?? err)}`,
      looksLikePermissionProblem(err)
    );
  }
}

// Re-exported so a caller that already imports the client does not need a
// second import for it. The definition lives in ./sql, which is free of the
// client import and therefore safe for query builders under test.
export { sqlLiteral } from "./sql";
