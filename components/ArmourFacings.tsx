/**
 * ArmourFacings — protection at each aspect, kinetic and chemical side by side.
 *
 * ⚠ THE ASYMMETRY IS THE WHOLE STORY. A Challenger 2 is 700 mm at the front,
 * 140 at the side and 30 at the roof: a factor of more than twenty between
 * the aspect a tank presents when it is fighting and the one it presents when
 * it is outmanoeuvred. The old profile carried a single frontal figure, so
 * every shot met the glacis and flanking was decoration.
 *
 * Kinetic and chemical are shown as separate columns rather than one number,
 * because they diverge by a factor of three on the same plate — that side is
 * 140 mm against a sabot round and 400 against a shaped charge — and a reader
 * interested in what an RPG does is looking at the other column.
 *
 * The bar is proportional to the heaviest aspect on THIS vehicle, not to a
 * catalogue-wide maximum. The question it answers is "where is this one
 * weak", which is a question about itself.
 */

import type { Facing } from "../data/curatedAssets";

export function ArmourFacings({ facings }: { facings: Facing[] }) {
  const present = facings.filter((f) => f.keMm != null || f.ceMm != null);
  if (present.length === 0) return null;

  const max = Math.max(...present.flatMap((f) => [f.keMm ?? 0, f.ceMm ?? 0]), 1);

  return (
    <div>
      <div style={{ display: "flex", ...headerRow }}>
        <span style={{ width: 84, flexShrink: 0 }}>aspect</span>
        <span style={{ width: 92, flexShrink: 0 }}>kinetic</span>
        <span style={{ width: 92, flexShrink: 0 }}>shaped charge</span>
      </div>
      {present.map((facing) => (
        <div key={facing.aspect} style={{ display: "flex", alignItems: "center", marginBottom: 3 }}>
          <span style={{ width: 84, flexShrink: 0, fontSize: 11, color: "#8a91a8" }}>
            {facing.aspect}
          </span>
          <Bar value={facing.keMm} max={max} colour="#e8c547" />
          <Bar value={facing.ceMm} max={max} colour="#7ec9a3" />
        </div>
      ))}
      <div style={{ fontSize: 10, color: "#6a7292", marginTop: 4 }}>
        RHA-equivalent millimetres. Kinetic resists sabot; shaped charge resists
        HEAT, and reactive armour raises it further.
      </div>
    </div>
  );
}

function Bar({
  value,
  max,
  colour,
}: {
  value: number | null;
  max: number;
  colour: string;
}) {
  return (
    <span style={{ width: 92, flexShrink: 0, display: "flex", alignItems: "center", gap: 4 }}>
      <span
        style={{
          display: "inline-block",
          height: 7,
          // Zero is real here — an unarmoured truck genuinely stops nothing —
          // so it draws as a hairline rather than vanishing, which would read
          // as "unknown".
          width: value == null ? 0 : Math.max(1, (value / max) * 46),
          background: colour,
          borderRadius: 1,
        }}
      />
      <span style={{ fontSize: 11, color: value == null ? "#6a7292" : "#e9ecfb" }}>
        {value == null ? "—" : value}
      </span>
    </span>
  );
}

const headerRow: React.CSSProperties = {
  fontSize: 10,
  color: "#6a7292",
  textTransform: "uppercase",
  letterSpacing: 0.4,
  marginBottom: 4,
};
