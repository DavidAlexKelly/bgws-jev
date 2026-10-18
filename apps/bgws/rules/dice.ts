// ── bgws/rules/dice.ts ─────────────────────────────────────────────────────
// Every random number in the game, from one seeded source.
//
// WHY NOT Math.random()
// ---------------------
// Two reasons, and the CI code scan's ban is only the third. A game has to
// REPLAY: given a seed and the list of decisions, the same game must come out,
// or an after-action review is a different battle from the one that was
// fought. And the experiment harness compares rulesets by running the same
// scenario twice with the same seed — which is meaningless unless the dice are
// the same dice.
//
// xorshift128+ because it is four lines, has no dependencies, passes the
// statistical tests that matter for dice, and — unlike crypto randomness — can
// be replayed from a seed.

export interface DiceRoll {
  /** Individual die faces, in order. */
  dice: number[];
  /** Their sum, before any modifier. */
  total: number;
  /** Index of the first draw, so a roll can be located in a replay. */
  cursor: number;
}

export interface Rng {
  readonly seed: string;
  /** How many draws have been taken. Part of the game's saved state. */
  readonly cursor: number;
  d6(): DiceRoll;
  d66(): DiceRoll;
  /** Uniform integer in [0, n). For chit draws and tie-breaks. */
  int(n: number): number;
  /** Pick one, for an AI that needs to break a tie reproducibly. */
  pick<T>(items: readonly T[]): T;
}

/** FNV-1a: a string seed into four 32-bit words, deterministically. */
function seedState(seed: string): [number, number, number, number] {
  let h = 2166136261;
  const words: number[] = [];
  for (let w = 0; w < 4; w++) {
    for (let i = 0; i < seed.length; i++) {
      h ^= seed.charCodeAt(i) + w * 7919;
      h = Math.imul(h, 16777619);
    }
    words.push(h >>> 0);
  }
  // All-zero state is a fixed point for xorshift; any constant avoids it.
  if (words.every((x) => x === 0)) return [1, 2, 3, 4];
  return [words[0], words[1], words[2], words[3]];
}

/**
 * A seeded generator.
 *
 * `startCursor` lets a saved game resume exactly where it left off: the state
 * is re-derived from the seed and then advanced, which is slower than storing
 * the raw state and immune to the raw state's format ever changing.
 */
export function createRng(seed: string, startCursor = 0): Rng {
  let [a, b, c, d] = seedState(seed);
  let cursor = 0;

  const next = (): number => {
    // xorshift128
    const t = a ^ (a << 11);
    a = b;
    b = c;
    c = d;
    d = (d ^ (d >>> 19) ^ (t ^ (t >>> 8))) >>> 0;
    cursor += 1;
    return d / 0x1_0000_0000;
  };

  for (let i = 0; i < startCursor; i++) next();

  const die = (): number => Math.floor(next() * 6) + 1;

  return {
    seed,
    get cursor() {
      return cursor;
    },
    d6(): DiceRoll {
      const at = cursor;
      const face = die();
      return { dice: [face], total: face, cursor: at };
    },
    d66(): DiceRoll {
      const at = cursor;
      const first = die();
      const second = die();
      return { dice: [first, second], total: first + second, cursor: at };
    },
    int(n: number): number {
      return Math.floor(next() * Math.max(1, n));
    },
    pick<T>(items: readonly T[]): T {
      return items[Math.floor(next() * items.length)];
    },
  };
}
