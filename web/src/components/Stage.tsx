import { useCallback, useEffect, useRef, useState } from "react";

/** Where a token can stand: each holder on each chain, and the escrow in the shielded pool. */
export type Anchor = "aBase" | "bBase" | "aZ" | "bZ" | "escrow";

export type Token = {
  id: string;
  /** zec moves in the pool; usdc and msg (a signed message, e.g. a check-in) on Base. */
  kind: "zec" | "usdc" | "msg";
  label: string;
  at: Anchor;
  hidden?: boolean;
};

export type BasinGauge = { ltvBps: number; limitBps?: number; actionBps: number; floodBps: number };

/** One state of a policy simulation. Key order: holder, counterparty, policy signer. */
export type Frame = {
  caption: string;
  clock: string;
  tokens: Token[];
  keys: [boolean, boolean, boolean];
  watch?: string;
  basin: { label: string; gauge?: BasinGauge };
  hold?: number;
  liquidation?: boolean;
};

export type Parties = { a: string; b: string };

const H = 300;
const BASE = { top: 50, bottom: 124, mid: 96 };
const ZEC = { top: 170, bottom: 296, mid: 244 };

/** The two-lane stage: Base above, the Zcash shielded pool below, the policy signer between them. */
export function LaneStage({
  frame,
  prev,
  width,
  parties,
}: {
  frame: Frame;
  prev?: Frame;
  width: number;
  parties: Parties;
}) {
  const colA = Math.max(58, width * 0.14);
  const colB = width - colA;
  const colC = width / 2;
  const basinW = Math.min(170, width * 0.4);
  const basin = { x: colC - basinW / 2, y: 190, w: basinW, h: 84 };
  const positions = (f: Frame) => {
    const anchors: Record<Anchor, [number, number]> = {
      aBase: [colA, BASE.mid],
      bBase: [colB, BASE.mid],
      aZ: [colA, ZEC.mid],
      bZ: [colB, ZEC.mid],
      // Leave room for the basin's staff when it shows one.
      escrow: [f.basin.gauge ? colC - 26 : colC, basin.y + 54],
    };
    // Tokens sharing an anchor stack vertically.
    const counts = new Map<Anchor, number>();
    f.tokens.filter((t) => !t.hidden).forEach((t) => counts.set(t.at, (counts.get(t.at) ?? 0) + 1));
    const seen = new Map<Anchor, number>();
    const out = new Map<string, [number, number]>();
    for (const t of f.tokens) {
      const [x, y] = anchors[t.at];
      if (t.hidden) {
        out.set(t.id, [x, y]);
        continue;
      }
      const n = counts.get(t.at) ?? 1;
      const k = seen.get(t.at) ?? 0;
      seen.set(t.at, k + 1);
      out.set(t.id, [x, y + (k - (n - 1) / 2) * 26]);
    }
    return out;
  };
  const now = positions(frame);
  const before = prev ? positions(prev) : null;
  // The path each token just took, shown for as long as the frame holds.
  const trails = frame.tokens.flatMap((t) => {
    const p = prev?.tokens.find((q) => q.id === t.id);
    if (!p || !before || t.hidden || p.hidden || p.at === t.at) return [];
    const [x1, y1] = before.get(t.id)!;
    const [x2, y2] = now.get(t.id)!;
    return [{ id: t.id, kind: t.kind, x1, y1, x2, y2 }];
  });
  const key = (cx: number, cy: number, on: boolean) => (
    <g>
      <circle cx={cx} cy={cy} r={6} className="skey" />
      <circle cx={cx} cy={cy} r={6.5} className={`skey-disc${on ? " on" : ""}${frame.liquidation ? " flood" : ""}`} />
    </g>
  );

  return (
    <svg width={width} height={H} viewBox={`0 0 ${width} ${H}`} className="lane-stage" aria-hidden>
      <defs>
        <marker id="trail-head" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="8" markerHeight="8" orient="auto">
          <path d="M1 1 L7 4 L1 7" className="trail-head" />
        </marker>
      </defs>
      {/* holders */}
      {key(colA, 14, frame.keys[0])}
      <text x={colA} y={36} textAnchor="middle" className="holder">
        {parties.a}
      </text>
      {key(colB, 14, frame.keys[1])}
      <text x={colB} y={36} textAnchor="middle" className="holder">
        {parties.b}
      </text>

      {/* Base lane */}
      <line x1={0} x2={width} y1={BASE.top} y2={BASE.top} className="lane-rule" />
      <line x1={0} x2={width} y1={BASE.bottom} y2={BASE.bottom} className="lane-rule" />
      <line x1={0} x2={width} y1={BASE.top + 24} y2={BASE.top + 24} className="ledger-line" />
      <line x1={0} x2={width} y1={BASE.bottom - 10} y2={BASE.bottom - 10} className="ledger-line" />
      <text x={6} y={BASE.top + 16} className="lane-label">
        Base
      </text>
      <text x={width - 6} y={BASE.top + 16} textAnchor="end" className="clock">
        {frame.clock}
      </text>

      {/* policy signer */}
      <line
        x1={colC}
        x2={colC}
        y1={BASE.bottom}
        y2={138}
        className={frame.watch ? "watch-line on" : "watch-line"}
      />
      <text x={colC + 10} y={BASE.bottom + 12} className={frame.watch ? "watch-text on" : "watch-text"}>
        {frame.watch ?? ""}
      </text>
      {key(colC, 146, frame.keys[2])}
      <text x={colC + 12} y={150} className="holder">
        Policy signer
      </text>
      <text x={colC + 12} y={165} className="holder-sub">
        attested, in a TEE
      </text>

      {/* Zcash shielded pool */}
      <rect x={0} y={ZEC.top + 6} width={width} height={ZEC.bottom - ZEC.top - 6} className="pool" />
      <text x={6} y={ZEC.bottom - 6} className="lane-label">
        Zcash shielded pool
      </text>
      <rect x={basin.x} y={basin.y} width={basin.w} height={basin.h} className="basin" />
      <text x={colC} y={basin.y + 16} textAnchor="middle" className="basin-label">
        escrow, 2 of 3
      </text>
      {frame.basin.gauge && <BasinStaff gauge={frame.basin.gauge} x={basin.x + basin.w - 18} y={basin.y + 24} h={52} />}

      {/* trails, then tokens on top */}
      {trails.map((tr) => {
        const dir = Math.sign(tr.x2 - tr.x1) || 1;
        const dist = Math.abs(tr.x2 - tr.x1);
        if (dist < 60) return null;
        const pad = Math.min(58, dist * 0.3);
        return (
          <line
            key={`trail-${tr.id}-${frame.caption.length}`}
            x1={tr.x1 + dir * pad}
            y1={tr.y1}
            x2={tr.x2 - dir * pad}
            y2={tr.y2}
            className={`trail ${tr.kind}`}
            markerEnd="url(#trail-head)"
          />
        );
      })}
      {frame.tokens.map((t) => {
        const [x, y] = now.get(t.id)!;
        const w = t.label.length * 7.8 + 18;
        return (
          <g
            key={t.id}
            className={`token ${t.kind}`}
            style={{ transform: `translate(${x}px, ${y}px)`, opacity: t.hidden ? 0 : 1 }}
          >
            <rect x={-w / 2} y={-12} width={w} height={24} rx={3} />
            <text textAnchor="middle" dominantBaseline="central">
              {t.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function BasinStaff({ gauge, x, y, h }: { gauge: BasinGauge; x: number; y: number; h: number }) {
  const ly = (bps: number) => y + (1 - Math.min(Math.max(bps, 0), 10_000) / 10_000) * h;
  const flooded = gauge.ltvBps >= gauge.floodBps;
  return (
    <g>
      <rect x={x} y={y} width={10} height={h} className="staff" />
      <rect
        x={x}
        y={y}
        width={10}
        height={h}
        className="water-body water-grow"
        style={{ transform: `scaleY(${Math.min(Math.max(gauge.ltvBps, 0), 10_000) / 10_000})` }}
      />
      <rect x={x} y={y} width={10} height={h} className="staff-edge" />
      {gauge.limitBps !== undefined && (
        <line x1={x - 4} x2={x + 14} y1={ly(gauge.limitBps)} y2={ly(gauge.limitBps)} className="limit-line" />
      )}
      <line x1={x - 4} x2={x + 14} y1={ly(gauge.actionBps)} y2={ly(gauge.actionBps)} className="mini-action" />
      <line x1={x - 4} x2={x + 14} y1={ly(gauge.floodBps)} y2={ly(gauge.floodBps)} className="mini-flood" />
      <g className="water" style={{ transform: `translateY(${ly(gauge.ltvBps) - y}px)` }}>
        <line x1={x - 6} x2={x + 16} y1={y} y2={y} className={flooded ? "water-line flooded" : "water-line"} />
      </g>
      <text x={x - 8} y={ly(gauge.ltvBps)} textAnchor="end" dominantBaseline="middle" className={flooded ? "basin-ltv flood-text" : "basin-ltv"}>
        {(gauge.ltvBps / 100).toFixed(1)}&#8239;%
      </text>
    </g>
  );
}

/** Steps through frames; each frame holds for its own time. */
export function usePlayer(count: number, holds: number[], reduced: boolean) {
  const [index, setIndex] = useState(reduced ? count - 1 : 0);
  const [playing, setPlaying] = useState(false);
  const [played, setPlayed] = useState(false);

  useEffect(() => {
    if (!playing) return;
    if (index >= count - 1) {
      setPlaying(false);
      return;
    }
    const t = window.setTimeout(() => setIndex((i) => i + 1), holds[index] ?? 2000);
    return () => window.clearTimeout(t);
  }, [playing, index, count, holds]);

  useEffect(() => {
    if (reduced) {
      setPlaying(false);
      setIndex(count - 1);
    }
  }, [reduced, count]);

  const play = useCallback(() => {
    setIndex(0);
    setPlayed(true);
    setPlaying(!reduced);
  }, [reduced]);
  const pause = useCallback(() => setPlaying(false), []);
  const resume = useCallback(() => setPlaying(!reduced), [reduced]);

  return { index, playing, played, play, pause, resume, finished: index >= count - 1 };
}

/** True while at least `threshold` of the element is visible. */
export function useInView<T extends Element>(threshold = 0.5) {
  const ref = useRef<T>(null);
  const [inView, setInView] = useState(false);
  useEffect(() => {
    if (!ref.current) return;
    const io = new IntersectionObserver(([e]) => setInView(e.isIntersecting), { threshold });
    io.observe(ref.current);
    return () => io.disconnect();
  }, [threshold]);
  return { ref, inView };
}
