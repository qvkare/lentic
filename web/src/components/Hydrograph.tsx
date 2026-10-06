import { clock, pctStage } from "../format";
import type { Reading } from "../policy";

type Props = {
  readings: Reading[];
  actionBps: number;
  floodBps: number;
  width: number;
};

const H = 200;
const PAD = { top: 14, bottom: 26, left: 36 };
const GUTTER = 118; // stage labels sit right of the plot, never on the series

/** Loan-to-value per round with both stages drawn across it; the part above flood stage is red. */
export function Hydrograph({ readings, actionBps, floodBps, width }: Props) {
  const points = readings.filter((r) => r.ltvBps !== null) as (Reading & { ltvBps: number })[];
  const gutter = width < 480 ? 84 : GUTTER;
  const w = Math.max(width - PAD.left - gutter, 10);
  const h = H - PAD.top - PAD.bottom;
  const t0 = points[0]?.round ?? 0;
  const t1 = points[points.length - 1]?.round ?? 1;
  const x = (t: number) => PAD.left + (t1 === t0 ? w : ((t - t0) / (t1 - t0)) * w);
  const y = (bps: number) => PAD.top + (1 - Math.min(Math.max(bps, 0), 10_000) / 10_000) * h;
  const path = points.map((p, i) => `${i ? "L" : "M"}${x(p.round).toFixed(1)},${y(p.ltvBps).toFixed(1)}`).join("");

  return (
    <svg
      width={width}
      height={H}
      viewBox={`0 0 ${width} ${H}`}
      className="hydrograph"
      role="img"
      aria-label={`Loan-to-value over ${points.length} readings`}
    >
      <defs>
        <clipPath id="above-flood">
          <rect x={0} y={0} width={width} height={y(floodBps)} />
        </clipPath>
      </defs>
      {[0, 2500, 5000, 7500, 10_000].map((v) => (
        <g key={v}>
          <line x1={PAD.left} x2={PAD.left + w} y1={y(v)} y2={y(v)} className="grid" />
          <text x={PAD.left - 8} y={y(v)} textAnchor="end" dominantBaseline="middle" className="axis">
            {v / 100}
          </text>
        </g>
      ))}
      <line x1={PAD.left} x2={PAD.left + w} y1={y(actionBps)} y2={y(actionBps)} className="stage-action-line" />
      <line x1={PAD.left} x2={PAD.left + w} y1={y(floodBps)} y2={y(floodBps)} className="stage-flood-line" />
      <text x={PAD.left + w + 10} y={y(actionBps)} dominantBaseline="middle" className="stage-label">
        action {width < 480 ? "" : "stage "}
        {pctStage(actionBps)}
      </text>
      <text x={PAD.left + w + 10} y={y(floodBps)} dominantBaseline="middle" className="stage-label flood-text">
        flood {width < 480 ? "" : "stage "}
        {pctStage(floodBps)}
      </text>
      {points.length > 0 && (
        <>
          <path d={path} className="series" />
          <path d={path} className="series flooded" clipPath="url(#above-flood)" />
          <circle cx={x(t1)} cy={y(points[points.length - 1].ltvBps)} r={3} className="series-end" />
          <text x={PAD.left} y={H - 6} className="axis">
            {clock(t0)}
          </text>
          <text x={PAD.left + w} y={H - 6} textAnchor="end" className="axis">
            {clock(t1)}
          </text>
        </>
      )}
      {points.length === 0 && (
        <text x={PAD.left + 8} y={PAD.top + h / 2} className="axis">
          The hydrograph starts with the policy signer's first reading.
        </text>
      )}
    </svg>
  );
}
