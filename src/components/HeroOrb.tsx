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
 * Massive hero toggle orb: the primary power control.
 *
 * - idle: breathing core, accent fill
 * - connecting: rotating conic neon ring + staggered pulse ripples
 * - connected: shockwave on entry + continuous ambient aura
 *
 * Spring physics on press (stiffness 380, damping 28 per spec).
 * All ambient motion is transform/opacity only (60/120fps safe).
 */
export const HeroOrb: React.FC<HeroOrbProps> = React.memo(
  ({ state, onToggle, disabled, label }) => {
    const [shockKey, setShockKey] = useState(0);
    const prevState = useRef<OrbState>(state);

    // Fire the shockwave exactly once when we land in `connected`.
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

    const coreBg =
      state === "connected"
        ? "bg-emerald-500"
        : "bg-accent-fill";

    return (
      <div className="flex flex-col items-center gap-3">
        <motion.button
          type="button"
          onClick={onToggle}
          disabled={disabled}
          whileTap={{ scale: 0.9 }}
          transition={{ type: "spring", stiffness: 380, damping: 28, mass: 0.8 }}
          className={`orb ${orbClass} h-36 w-36 md:h-40 md:w-40 disabled:cursor-not-allowed disabled:opacity-60`}
          aria-label={label}
          title={label}
        >
          {state === "connecting" && (
            <>
              <span className="orb-ring" />
              <span className="orb-ripple" />
              <span className="orb-ripple" />
              <span className="orb-ripple" />
            </>
          )}
          {state === "connected" && <span className="orb-aura" />}
          {state === "connected" && shockKey > 0 && (
            <span key={shockKey} className="orb-shockwave" />
          )}
          <span
            className={`orb-core h-24 w-24 md:h-28 md:w-28 ${coreBg} text-white shadow-lg`}
          >
            {state === "connecting" ? (
              <Loader2 className="h-10 w-10 animate-spin" />
            ) : state === "connected" ? (
              <ShieldCheck className="h-10 w-10" />
            ) : (
              <Power className="h-10 w-10" />
            )}
          </span>
        </motion.button>
        <span className="text-[11px] font-semibold uppercase tracking-[0.18em] text-ink4">
          {label}
        </span>
      </div>
    );
  }
);
