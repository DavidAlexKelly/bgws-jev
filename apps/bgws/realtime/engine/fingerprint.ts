// ── fingerprint.ts ───────────────────────────────────────────────────────
// One short string for a whole game's event stream, computed the same way in
// a browser and in Node (MULTI_DOMAIN_PLAN.md W0: "the same seeded scenario
// gives byte-identical event streams in browser and Node").
//
// Every feed entry, in order, is written as JSON (`toJson`: the same text in
// both, -Infinity and byte arrays included) and folded into a running hash.
// Two runs agree on the fingerprint only if every entry agreed, in order.
// Plain JavaScript on purpose: no crypto API, which differs between the two
// (and is asynchronous in the browser). Not for security; for sameness.

/** JSON that keeps what plain JSON loses: non-finite numbers and byte arrays. */
export function toJson(value: unknown, indent?: number): string {
  return JSON.stringify(
    value,
    (_key, v: unknown) => {
      if (typeof v === "number" && !Number.isFinite(v)) return { $num: String(v) };
      if (v instanceof Uint8Array) return { $bytes: bytesToBase64(v) };
      return v;
    },
    indent,
  );
}

/** Read what `toJson` wrote. */
export function fromJson<T = unknown>(text: string): T {
  return JSON.parse(text, (_key, v: unknown) => {
    if (v && typeof v === "object" && !Array.isArray(v)) {
      const o = v as Record<string, unknown>;
      const keys = Object.keys(o);
      if (keys.length === 1 && typeof o.$num === "string") return Number(o.$num);
      if (keys.length === 1 && typeof o.$bytes === "string") return base64ToBytes(o.$bytes);
    }
    return v;
  }) as T;
}

function bytesToBase64(bytes: Uint8Array): string {
  let s = "";
  for (const b of bytes) s += String.fromCharCode(b);
  return btoa(s);
}

function base64ToBytes(text: string): Uint8Array {
  const s = atob(text);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

/** cyrb53: a fast, well-mixed 53-bit string hash. */
function cyrb53(text: string, seed: number): number {
  let h1 = 0xdeadbeef ^ seed;
  let h2 = 0x41c6ce57 ^ seed;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 2654435761);
    h2 = Math.imul(h2 ^ c, 1597334677);
  }
  h1 = Math.imul(h1 ^ (h1 >>> 16), 2246822507) ^ Math.imul(h2 ^ (h2 >>> 13), 3266489909);
  h2 = Math.imul(h2 ^ (h2 >>> 16), 2246822507) ^ Math.imul(h1 ^ (h1 >>> 13), 3266489909);
  return 4294967296 * (2097151 & h2) + (h1 >>> 0);
}

/** A running fingerprint of an event stream: add each entry in order. */
export class EventFingerprint {
  private chain = "start";
  private count = 0;

  add(entry: unknown): void {
    const text = `${this.chain}\n${toJson(entry)}`;
    this.chain = cyrb53(text, 1).toString(16).padStart(14, "0") + cyrb53(text, 2).toString(16).padStart(14, "0");
    this.count++;
  }

  /** The stream so far: its fingerprint and how many entries made it. */
  get value(): { fingerprint: string; entries: number } {
    return { fingerprint: this.chain, entries: this.count };
  }
}
