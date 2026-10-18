// ── bgws/components/Die.tsx ────────────────────────────────────────────────
// A die that looks like a die.
//
// The pip characters in the Unicode block render at whatever weight and
// baseline the user's font decides, which in practice means "small grey
// smudge" — and the dice are the evidence. A wargame that hides its rolls in
// prose is asking to be taken on trust, so they are drawn: a face, its pips,
// and the modifiers that changed what the face was worth.

const PIP_LAYOUT: Record<number, [number, number][]> = {
  // Pip positions on a 3x3 grid, as [column, row], 0-2.
  1: [[1, 1]],
  2: [
    [0, 0],
    [2, 2],
  ],
  3: [
    [0, 0],
    [1, 1],
    [2, 2],
  ],
  4: [
    [0, 0],
    [2, 0],
    [0, 2],
    [2, 2],
  ],
  5: [
    [0, 0],
    [2, 0],
    [1, 1],
    [0, 2],
    [2, 2],
  ],
  6: [
    [0, 0],
    [2, 0],
    [0, 1],
    [2, 1],
    [0, 2],
    [2, 2],
  ],
};

export interface DieProps {
  value: number;
  size?: number;
  /** Dice that decided something are worth looking at more than the rest. */
  emphasis?: boolean;
}

export function Die({ value, size = 26, emphasis = false }: DieProps): JSX.Element {
  const pips = PIP_LAYOUT[value] ?? [];
  const pip = Math.max(3, Math.round(size / 7));
  const inset = Math.round(size * 0.18);
  const span = size - inset * 2 - pip;

  return (
    <span
      role="img"
      aria-label={`die showing ${value}`}
      title={`${value}`}
      style={{
        position: "relative",
        display: "inline-block",
        width: size,
        height: size,
        borderRadius: Math.round(size / 6),
        background: emphasis ? "#f2efe6" : "#d8dcea",
        boxShadow: "inset 0 -1px 0 rgba(0,0,0,0.25)",
        border: "1px solid #0d1017",
        verticalAlign: "middle",
      }}
    >
      {pips.map(([column, row], i) => (
        <span
          key={i}
          style={{
            position: "absolute",
            left: inset + (span * column) / 2,
            top: inset + (span * row) / 2,
            width: pip,
            height: pip,
            borderRadius: "50%",
            background: "#11141d",
          }}
        />
      ))}
    </span>
  );
}

/** A roll, as dice, with the arithmetic beside it rather than instead of it. */
export function DiceRow({
  dice,
  total,
  size = 26,
}: {
  dice: readonly number[];
  total?: number;
  size?: number;
}): JSX.Element {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
      {dice.map((die, i) => (
        <Die key={i} value={die} size={size} />
      ))}
      {dice.length > 0 && total !== undefined && (
        <span style={{ color: "#8a91a8", fontSize: 11 }}>
          {dice.join(" + ")} = {total}
        </span>
      )}
    </span>
  );
}
