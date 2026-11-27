import React, { useEffect, useRef, useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Power, Loader2, ShieldCheck } from "lucide-react";

export type OrbState = "idle" | "connecting" | "connected";

interface HeroOrbProps {
  state: OrbState;
  onToggle: () => void;
  disabled?: boolean;
  label: string;
  /** Smaller disc for dense layouts (simple-view hero); default size unchanged. */
  compact?: boolean;
}

/**
 * Sun disc — the primary control (theme/sun variant).
 *
 * The orb is a sun: off = night (dark disc, faint warm rim, static),
 * connecting = dawn (spinning gold ring), on = sunrise (radiant golden
 * disc, warm glow). The icon sits dark on gold when the sun is up.
 *
 * Spring physics on press (stiffness 380, damping 28).
 */
export const HeroOrb: React.FC<HeroOrbProps> = React.memo(
  ({ state, onToggle, disabled, label, compact = false }) => {
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
        ? "bg-orange-500/30"
        : state === "connecting"
        ? "bg-accent/25"
        : "bg-orange-500/10";

    /* The disc itself: night glass at rest, a radiant golden sun
       once the bypass is up. */
    const disc =
      state === "connected"
        ? "border-orange-200/50 bg-[radial-gradient(circle_at_50%_38%,#fde68a_0%,#fbbf24_35%,#f97316_65%,#e2342a_100%)] shadow-[0_0_44px_rgba(249,115,22,0.45),0_0_90px_rgba(226,52,42,0.18),inset_0_1px_0_rgba(255,255,255,0.35)]"
        : "border-orange-200/[0.14] bg-white/[0.03] shadow-[0_8px_40px_rgba(0,0,0,0.45),inset_0_1px_0_rgba(255,255,255,0.08)] transition-colors duration-500 group-hover:border-orange-200/[0.25]";

    const iconClass = compact ? "h-9 w-9" : "h-11 w-11";
    const icon =
      state === "connecting" ? (
        <Loader2 className={`${iconClass} animate-spin text-orange-100/90`} strokeWidth={1.75} />
      ) : state === "connected" ? (
        <ShieldCheck className={`${iconClass} text-amber-950`} strokeWidth={1.75} />
      ) : (
        <Power className={`${iconClass} text-orange-100/90`} strokeWidth={1.75} />
      );

    return (
      <div className={`flex flex-col items-center ${compact ? "gap-3" : "gap-4"}`}>
        <motion.button
          type="button"
          onClick={() => onToggle()}
          disabled={disabled}
          whileTap={{ scale: 0.92 }}
          whileHover={{ scale: 1.03 }}
          transition={{ type: "spring", stiffness: 380, damping: 28, mass: 0.8 }}
          className={`orb ${orbClass} group relative ${
            compact ? "h-32 w-32" : "h-40 w-40 md:h-44 md:w-44"
          } disabled:cursor-not-allowed disabled:opacity-60 cursor-pointer`}
          aria-label={label}
          title={label}
        >
          {/* halo */}
          <span
            className={`absolute inset-0 rounded-full blur-2xl transition-colors duration-700 ${halo}`}
          />
          {/* shockwave on connect */}
          {state === "connected" && shockKey > 0 && (
            <span key={shockKey} className="orb-shockwave" />
          )}
          {/* orbit rings around the risen sun (Gemini dawn port):
              two dashed rings counter-rotating, a small gold planet
              riding the inner one. Connected only. */}
          {state === "connected" && (
            <>
              <svg
                className="sun-orbit sun-orbit-cw"
                viewBox="0 0 100 100"
                aria-hidden="true"
              >
                <circle
                  cx="50"
                  cy="50"
                  r="48"
                  fill="none"
                  stroke="#fbbf24"
                  strokeOpacity="0.5"
                  strokeWidth="0.8"
                  strokeDasharray="14 10"
                />
                <circle cx="50" cy="2" r="1.6" fill="#fde68a" />
              </svg>
              <svg
                className="sun-orbit sun-orbit-ccw"
                viewBox="0 0 100 100"
                aria-hidden="true"
              >
                <circle
                  cx="50"
                  cy="50"
                  r="48"
                  fill="none"
                  stroke="#f97316"
                  strokeOpacity="0.35"
                  strokeWidth="0.6"
                  strokeDasharray="3 7"
                />
              </svg>
            </>
          )}
          {/* rotating ring while connecting */}
          {state === "connecting" && <span className="orb-ring" />}
          {/* sun disc */}
          <span className={`absolute inset-0 rounded-full border ${disc}`}>
            {/* icon — crossfades between states. Both the exiting and the
            entering icon are absolutely positioned over the same centered
            box: with popLayout the exiting icon was popped out of the flex
            flow and the pair showed up side by side (power left, shield
            right) for the 0.18s swap, which read as the icon jumping
            sideways on every toggle. Sync mode + identical boxes makes
            the swap a pure in-place crossfade. */}
            <span className="absolute inset-0 flex items-center justify-center">
              <AnimatePresence initial={false}>
                <motion.span
                  key={state}
                  initial={{ opacity: 0, scale: 0.55, rotate: -24 }}
                  animate={{ opacity: 1, scale: 1, rotate: 0 }}
                  exit={{ opacity: 0, scale: 0.55, rotate: 24 }}
                  transition={{ duration: 0.18, ease: [0.22, 1, 0.36, 1] }}
                  className="absolute inset-0 flex items-center justify-center"
                >
                  {icon}
                </motion.span>
              </AnimatePresence>
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
