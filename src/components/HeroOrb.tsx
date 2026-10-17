import React, { useEffect, useRef, useState } from "react";
import { motion } from "framer-motion";
import { Power, Loader2, ShieldCheck } from "lucide-react";

export type OrbState = "idle" | "connecting" | "connected";

interface HeroOrbProps {
  state: OrbState;
  onToggle: () => void;
  disabled?: boolean;
  label: string;
}

/**
 * Glass power orb — the primary control, styled after the design mockup:
 * a dark translucent glass disc with a bright rim and top highlight,
 * not a flat colored circle.
 *
 * - idle: faint blue halo, gentle breathing
 * - connecting: rotating conic ring
 * - connected: emerald halo
 *
 * Spring physics on press (stiffness 380, damping 28).
 */
export const HeroOrb: React.FC<HeroOrbProps> = React.memo(
  ({ state, onToggle, disabled, label }) => {
    const [shockKey, setShockKey] = useState(0);
    const prevState = useRef<OrbState>(state);

    useEffect(() => {
      if (prevState.current !== "connected" && state === "connected") {
        setShockKey((k) => k + 1);
      }
      prevState.current = state;
    }, [state]);

    const orbClass =
      state === "connected"
        ? "orb-connected"
        : state === "connecting"
        ? "orb-connecting"
        : "orb-idle";

    const halo =
      state === "connected"
        ? "bg-emerald-500/25"
        : state === "connecting"
        ? "bg-sky-500/25"
        : "bg-sky-500/15";

    const icon =
      state === "connecting" ? (
        <Loader2 className="h-11 w-11 animate-spin text-white/90" strokeWidth={1.75} />
      ) : state === "connected" ? (
        <ShieldCheck className="h-11 w-11 text-emerald-300" strokeWidth={1.75} />
      ) : (
        <Power className="h-11 w-11 text-white/90" strokeWidth={1.75} />
      );

    return (
      <div className="flex flex-col items-center gap-4">
        <motion.button
          type="button"
          onClick={() => onToggle()}
          disabled={disabled}
          whileTap={{ scale: 0.92 }}
          whileHover={{ scale: 1.03 }}
          transition={{ type: "spring", stiffness: 380, damping: 28, mass: 0.8 }}
          className={`orb ${orbClass} group relative h-40 w-40 md:h-44 md:w-44 disabled:cursor-not-allowed disabled:opacity-60 cursor-pointer`}
          aria-label={label}
          title={label}
        >
          {/* halo */}
          <span
            className={`absolute inset-0 rounded-full blur-2xl transition-colors duration-700 ${halo} ${
              state === "idle" ? "orb-breathe" : ""
            }`}
          />
          {/* shockwave on connect */}
          {state === "connected" && shockKey > 0 && (
            <span key={shockKey} className="orb-shockwave" />
          )}
          {/* rotating ring while connecting */}
          {state === "connecting" && <span className="orb-ring" />}
          {/* glass disc */}
          <span className="absolute inset-0 rounded-full border border-white/[0.14] bg-white/[0.05] backdrop-blur-xl shadow-[0_8px_40px_rgba(0,0,0,0.45),inset_0_1px_0_rgba(255,255,255,0.12)] transition-colors duration-500 group-hover:border-white/[0.22]">
            {/* top highlight */}
            <span className="absolute inset-x-6 top-2 h-10 rounded-full bg-gradient-to-b from-white/[0.14] to-transparent blur-[6px]" />
            {/* icon */}
            <span className="absolute inset-0 flex items-center justify-center">
              {icon}
            </span>
          </span>
        </motion.button>
        <span className="text-xs font-semibold uppercase tracking-[0.2em] text-ink3">
          {label}
        </span>
      </div>
    );
  }
);
