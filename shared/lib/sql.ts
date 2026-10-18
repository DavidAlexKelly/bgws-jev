// ── shared/lib/sql.ts ──────────────────────────────────────────────────────
// Pure SQL text helpers.
//
// Separate from sqlClient.ts for the same reason csv.ts is: sqlClient imports
// ../../client, which reads its configuration from <meta> tags at module load
// and therefore cannot be imported by anything a unit test touches. A query
// BUILDER is exactly the sort of thing worth unit testing, so the pieces it
// needs live here.

/**
 * Quote a value for inclusion in a SQL string literal.
 *
 * Callers here build queries from identifiers that came out of Foundry in the
 * first place, so this is belt and braces rather than a live risk — but a
 * dataset is not a trust boundary, and a display name containing an
 * apostrophe would otherwise produce a syntax error at best.
 */
export function sqlLiteral(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}
