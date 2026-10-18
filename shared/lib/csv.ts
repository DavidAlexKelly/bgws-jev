// ── shared/lib/csv.ts ──────────────────────────────────────────────────────
// Pure CSV helpers for SQL results.
//
// Kept free of any import of ../../client so it can be unit tested without
// a DOM — client.ts reads its configuration from <meta> tags at module load.
//
// Moved over from the Mapplications repo with BGWS.

/** A parsed CSV result set. */
export interface Table {
  columns: string[];
  rows: string[][];
}

/**
 * RFC-4180-ish CSV parser. Foundry quotes any field containing a comma, quote
 * or newline and escapes embedded quotes by doubling them, so a naive
 * split(",") corrupts unit names like `INF.Rifle platoon [388], det`.
 */
export function parseCsv(text: string): Table {
  const rows: string[][] = [];
  let row: string[] = [];
  let field = "";
  let inQuotes = false;
  let i = 0;

  while (i < text.length) {
    const c = text[i];

    if (inQuotes) {
      if (c === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        inQuotes = false;
        i += 1;
        continue;
      }
      field += c;
      i += 1;
      continue;
    }

    if (c === '"') {
      inQuotes = true;
      i += 1;
      continue;
    }
    if (c === ",") {
      row.push(field);
      field = "";
      i += 1;
      continue;
    }
    if (c === "\r") {
      i += 1;
      continue;
    }
    if (c === "\n") {
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      i += 1;
      continue;
    }

    field += c;
    i += 1;
  }

  // Trailing record with no newline terminator.
  if (field.length > 0 || row.length > 0) {
    row.push(field);
    rows.push(row);
  }

  const columns = rows.shift() ?? [];
  return { columns, rows };
}

/** Column-name → index lookup, so callers are not order-sensitive. */
export function columnIndex(table: Table): Record<string, number> {
  const index: Record<string, number> = {};
  table.columns.forEach((name, i) => {
    index[name.trim().toLowerCase()] = i;
  });
  return index;
}

export function asNumber(value: string | undefined): number | null {
  if (value == null || value === "") return null;
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

/** CSV has no null: Foundry writes an empty field, and so does a literal "". */
export function asText(value: string | undefined): string | null {
  if (value == null || value === "") return null;
  return value;
}

/**
 * CSV has no boolean either. Spark writes `true`/`false`; be liberal about
 * what is accepted and strict about what counts as true.
 */
export function asBoolean(value: string | undefined): boolean | null {
  if (value == null || value === "") return null;
  const v = value.trim().toLowerCase();
  if (v === "true" || v === "t" || v === "1") return true;
  if (v === "false" || v === "f" || v === "0") return false;
  return null;
}

/**
 * Spark renders an array column as `["a","b"]` in CSV. Parsed leniently: the
 * exact quoting has changed between Spark versions and a capability list is
 * not worth a build failure.
 */
export function asStringArray(value: string | undefined): string[] {
  if (value == null || value === "") return [];
  const inner = value.trim().replace(/^\[/, "").replace(/\]$/, "");
  if (inner === "") return [];
  return inner
    .split(",")
    .map((part) => part.trim().replace(/^"/, "").replace(/"$/, ""))
    .filter((part) => part !== "");
}
