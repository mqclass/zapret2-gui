import React, { useMemo } from "react";

interface SparklineProps {
  /** Rolling history, oldest → newest. Nulls are gaps. */
  values: (number | null)[];
  width?: number;
  height?: number;
  /** Max expected value for scaling; defaults to max of values. */
  max?: number;
  className?: string;
}

/**
 * Lightweight SVG sparkline with neon glow stroke and soft area fill.
 * Purely presentational — feed it a rolling history buffer.
 */
export const Sparkline: React.FC<SparklineProps> = React.memo(
  ({ values, width = 120, height = 36, max, className }) => {
    const { path, area } = useMemo(() => {
      const nums = values.filter((v): v is number => v !== null && isFinite(v));
      if (nums.length < 2) return { path: "", area: "" };
      const hi = max ?? Math.max(...nums, 1);
      const stepX = width / (values.length - 1);
      // Map to points, skipping nulls (break the line).
      const pts: (string | null)[] = values.map((v, i) => {
        if (v === null || !isFinite(v)) return null;
        const x = (i * stepX).toFixed(1);
        const y = (height - 3 - (Math.min(v, hi) / hi) * (height - 8)).toFixed(1);
        return `${x},${y}`;
      });
      let d = "";
      let a = "";
      let started = false;
      let firstX = "";
      let lastX = "";
      for (const p of pts) {
        if (p === null) {
          started = false;
          continue;
        }
        const [x] = p.split(",");
        if (!started) {
          d += `M${p}`;
          firstX = x;
          started = true;
        } else {
          d += ` L${p}`;
        }
        lastX = x;
      }
      if (d) {
        a = `${d} L${lastX},${height} L${firstX},${height} Z`;
      }
      return { path: d, area: a };
    }, [values, width, height, max]);

    if (!path) {
      return (
        <svg width={width} height={height} className={`sparkline ${className ?? ""}`}>
          <line
            x1={0}
            y1={height / 2}
            x2={width}
            y2={height / 2}
            stroke="currentColor"
            strokeOpacity={0.25}
            strokeDasharray="3 4"
          />
        </svg>
      );
    }
    return (
      <svg width={width} height={height} className={`sparkline ${className ?? ""}`}>
        <path d={area} className="sparkline-area" />
        <path d={path} className="sparkline-path" />
      </svg>
    );
  }
);
