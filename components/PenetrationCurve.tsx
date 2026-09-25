/**
 * PenetrationCurve — a munition's penetration against range.
 *
 * WHY A CHART AND NOT FOUR MORE TABLE CELLS
 * -----------------------------------------
 * ⚠ THE SHAPE IS THE POINT. A shaped charge plots FLAT and a sabot round
 * plots DOWNWARD, and that single difference explains most of what armies buy
 * and why a Kornet detachment is dangerous at five kilometres where a tank
 * gun is merely accurate. In a table of four numbers the reader has to do the
 * subtraction and notice; on axes they cannot miss it.
 *
 * Drawn as inline SVG with no charting dependency, because the whole thing is
 * four points and an axis, and a dependency would be larger than the chart.
 *
 * Nulls are never plotted. A high-explosive round has no anti-armour
 * penetration to draw, which is a different statement from having run out of
 * it, and a zero on this chart would say the second thing.
 */

import type { CurvePoint } from "../data/curatedAssets";

const WIDTH = 260;
const HEIGHT = 96;
const PAD_LEFT = 34;
const PAD_BOTTOM = 18;
const PAD_TOP = 8;
const PAD_RIGHT = 8;

export function PenetrationCurve({
  points,
  armourMm,
  flat,
  label,
}: {
  points: CurvePoint[];
  /** Optional reference line: what this round is being asked to defeat. */
  armourMm?: number | null;
  /** Shaped charge — drawn in a different colour, since its flatness is real. */
  flat?: boolean;
  label?: string;
}) {
  if (points.length < 2) return null;

  const maxRange = Math.max(...points.map((p) => p.rangeM), 1);
  // The vertical scale includes the armour line when there is one, so a round
  // that cannot get through is visibly below it rather than cropped out.
  const maxPen = Math.max(...points.map((p) => p.penMm), armourMm ?? 0, 1);

  const x = (rangeM: number) =>
    PAD_LEFT + (rangeM / maxRange) * (WIDTH - PAD_LEFT - PAD_RIGHT);
  const y = (penMm: number) =>
    HEIGHT - PAD_BOTTOM - (penMm / maxPen) * (HEIGHT - PAD_TOP - PAD_BOTTOM);

  const line = points.map((p) => `${x(p.rangeM)},${y(p.penMm)}`).join(" ");
  const stroke = flat ? "#7ec9a3" : "#e8c547";

  return (
    <div style={{ marginTop: 6 }}>
      <svg
        width={WIDTH}
        height={HEIGHT}
        role="img"
        aria-label={
          label ??
          `penetration from ${points[0].penMm} mm at ${points[0].rangeM} m to ` +
            `${points[points.length - 1].penMm} mm at ${points[points.length - 1].rangeM} m`
        }
      >
        {/* Axes, drawn faintly — they orient the eye without competing with
            the line, which is the only thing here worth reading. */}
        <line
          x1={PAD_LEFT}
          y1={HEIGHT - PAD_BOTTOM}
          x2={WIDTH - PAD_RIGHT}
          y2={HEIGHT - PAD_BOTTOM}
          stroke="#2a3048"
        />
        <line
          x1={PAD_LEFT}
          y1={PAD_TOP}
          x2={PAD_LEFT}
          y2={HEIGHT - PAD_BOTTOM}
          stroke="#2a3048"
        />

        {armourMm != null && armourMm > 0 && (
          <>
            <line
              x1={PAD_LEFT}
              y1={y(armourMm)}
              x2={WIDTH - PAD_RIGHT}
              y2={y(armourMm)}
              stroke="#e07a5f"
              strokeDasharray="3 3"
            />
            <text x={WIDTH - PAD_RIGHT} y={y(armourMm) - 3} fontSize="8" fill="#e07a5f" textAnchor="end">
              {armourMm} mm armour
            </text>
          </>
        )}

        <polyline points={line} fill="none" stroke={stroke} strokeWidth="1.6" />
        {points.map((p) => (
          <circle key={p.rangeM} cx={x(p.rangeM)} cy={y(p.penMm)} r="2.2" fill={stroke} />
        ))}

        {/* Only the extremes are labelled. Four labels on a 260px chart is
            noise, and the ends are what a reader compares. */}
        <text x={PAD_LEFT - 4} y={y(points[0].penMm) + 3} fontSize="8" fill="#8a91a8" textAnchor="end">
          {Math.round(points[0].penMm)}
        </text>
        <text
          x={PAD_LEFT - 4}
          y={y(points[points.length - 1].penMm) + 3}
          fontSize="8"
          fill="#8a91a8"
          textAnchor="end"
        >
          {Math.round(points[points.length - 1].penMm)}
        </text>
        <text x={PAD_LEFT} y={HEIGHT - 6} fontSize="8" fill="#8a91a8">
          0
        </text>
        <text x={WIDTH - PAD_RIGHT} y={HEIGHT - 6} fontSize="8" fill="#8a91a8" textAnchor="end">
          {(maxRange / 1000).toFixed(0)} km
        </text>
      </svg>
    </div>
  );
}
