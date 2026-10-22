import React from "react";

interface ZapretLogoProps {
  className?: string;
  glow?: boolean;
}

/**
 * zapret2 mark: a symmetric shield with a crisp "Z" — the bypass glyph.
 * All coordinates are on a 32×32 grid, mirrored around x=16, so the
 * geometry stays optically balanced at any size.
 */
export const ZapretLogo: React.FC<ZapretLogoProps> = ({
  className = "h-6 w-6",
  glow = false,
}) => {
  return (
    <div
      className={`relative flex items-center justify-center shrink-0 ${className}`}
    >
      {glow && (
        <div className="absolute inset-0 rounded-panel bg-gradient-to-tr from-accent/40 to-accent-fill/40 blur-md pointer-events-none" />
      )}
      <svg
        viewBox="0 0 32 32"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className="w-full h-full relative z-10"
        role="img"
        aria-label="zapret2"
      >
        <defs>
          <linearGradient
            id="zapretShieldGrad"
            x1="5.5"
            y1="3"
            x2="26.5"
            y2="29"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" stopColor="#a78bfa" />
            <stop offset="55%" stopColor="#8b5cf6" />
            <stop offset="100%" stopColor="#d946ef" />
          </linearGradient>
          <linearGradient
            id="zapretZGrad"
            x1="11"
            y1="11.5"
            x2="21"
            y2="20.5"
            gradientUnits="userSpaceOnUse"
          >
            <stop offset="0%" stopColor="#c4b5fd" />
            <stop offset="100%" stopColor="#f0abfc" />
          </linearGradient>
        </defs>

        {/* Shield — symmetric around x=16 */}
        <path
          d="M16 3 L26.5 7.2 V15 C26.5 21.8 22 26.8 16 29 C10 26.8 5.5 21.8 5.5 15 V7.2 Z"
          fill="url(#zapretShieldGrad)"
          fillOpacity="0.16"
          stroke="url(#zapretShieldGrad)"
          strokeWidth="1.8"
          strokeLinejoin="round"
        />

        {/* Z glyph — centered, round caps */}
        <path
          d="M11 11.5 H21 L11 20.5 H21"
          stroke="url(#zapretZGrad)"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round"
        />

        {/* Signal nodes on the Z corners */}
        <circle cx="11" cy="11.5" r="1.4" fill="#a78bfa" />
        <circle cx="21" cy="11.5" r="1.4" fill="#c4b5fd" />
        <circle cx="11" cy="20.5" r="1.4" fill="#818cf8" />
        <circle cx="21" cy="20.5" r="1.4" fill="#f0abfc" />
      </svg>
    </div>
  );
};
