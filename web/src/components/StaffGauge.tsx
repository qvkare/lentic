import { pct, pctStage } from "../format";

type Props = {
  /** Loan-to-value in basis points; null until the first reading. */
  ltvBps: number | null;
  actionBps: number;
  floodBps: number;
  width: number;
  height: number;
};

const TOP = 18;
const BOTTOM = 18;
const NUMERALS = 34; // column for staff numerals
const STAFF = 34; // staff strip width

/**
 * The station's staff gauge: 0 to 100 % loan-to-value, the action stage (margin call) as an ink
 * mark, the flood stage (liquidation) as the only coloured mark, and the water line at the
 * current reading. Water is one flat lake tint.
 */
export function StaffGauge({ ltvBps, actionBps, floodBps, width, height }: Props) {
  const h = height - TOP - BOTTOM;
  const y = (bps: number) => TOP + (1 - Math.min(Math.max(bps, 0), 10_000) / 10_000) * h;
  const x0 = NUMERALS;
  const x1 = x0 + STAFF;
  const flooded = ltvBps !== null && ltvBps >= floodBps;
  const above = ltvBps !== null && ltvBps > 10_000;
  const waterY = ltvBps === null ? TOP + h : y(ltvBps);

  const ticks = [];
  for (let v = 0; v <= 100; v += 2) {
    const major = v % 10 === 0;
    const ty = y(v * 100);
    ticks.push(
      <line key={v} x1={x0} x2={x0 + (major ? STAFF : STAFF / 2)} y1={ty} y2={ty} className={major ? "tick major" : "tick"} />,
    );
    if (major) {
      ticks.push(
        <text key={`n${v}`} x={x0 - 8} y={ty} className="staff-numeral" textAnchor="end" dominantBaseline="middle">
          {v}
        </text>,
      );
    }
  }

  const label = (bps: number, text: string, cls: string) => (
    <g className={cls}>
      <line x1={x0} x2={width} y1={y(bps)} y2={y(bps)} />
      <text x={width} y={y(bps) - 6} textAnchor="end" className="stage-label">
        {text}
      </text>
    </g>
  );

  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={
        ltvBps === null
          ? "Staff gauge without a reading yet"
          : `Staff gauge: water line at ${pct(ltvBps)}, action stage ${pctStage(actionBps)}, flood stage ${pctStage(floodBps)}`
      }
      className="staff-gauge"
    >
      <rect x={x0} y={TOP} width={STAFF} height={h} className="staff" />
      {ltvBps !== null && (
        <g className="water" style={{ transform: `translateY(${waterY}px)` }}>
          <rect x={0} y={0} width={width} height={TOP + h + BOTTOM} className="water-body" />
        </g>
      )}
      <rect x={x0} y={TOP} width={STAFF} height={h} className="staff-edge" />
      {ticks}
      {label(actionBps, "action stage", "stage stage-action")}
      {label(floodBps, "flood stage", "stage stage-flood")}
      {ltvBps !== null && (
        <g className="water" style={{ transform: `translateY(${waterY}px)` }}>
          <line x1={0} x2={width} y1={0} y2={0} className={flooded ? "water-line flooded" : "water-line"} />
          <text x={x1 + 10} y={16} className={flooded ? "water-label flooded" : "water-label"}>
            {above ? "water line above the staff" : `water line ${pct(ltvBps)}`}
          </text>
        </g>
      )}
      {ltvBps === null && (
        <text x={x1 + 10} y={TOP + h - 8} className="water-label">
          no reading yet
        </text>
      )}
    </svg>
  );
}

/** A 12 px staff for table rows. */
export function MiniGauge({ ltvBps, actionBps, floodBps }: { ltvBps: number | null; actionBps: number; floodBps: number }) {
  const H = 36;
  const y = (bps: number) => 2 + (1 - Math.min(Math.max(bps, 0), 10_000) / 10_000) * (H - 4);
  const flooded = ltvBps !== null && ltvBps >= floodBps;
  return (
    <svg width={14} height={H} viewBox={`0 0 14 ${H}`} aria-hidden className="mini-gauge">
      <rect x={1} y={2} width={12} height={H - 4} className="staff" />
      {ltvBps !== null && <rect x={1} y={y(ltvBps)} width={12} height={H - 2 - y(ltvBps)} className="water-body" />}
      <line x1={1} x2={13} y1={y(actionBps)} y2={y(actionBps)} className="mini-action" />
      <line x1={1} x2={13} y1={y(floodBps)} y2={y(floodBps)} className="mini-flood" />
      {ltvBps !== null && (
        <line x1={0} x2={14} y1={y(ltvBps)} y2={y(ltvBps)} className={flooded ? "water-line flooded" : "water-line"} />
      )}
      <rect x={1} y={2} width={12} height={H - 4} className="staff-edge" />
    </svg>
  );
}
