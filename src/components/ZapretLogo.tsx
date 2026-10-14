import React from "react";

interface ZapretLogoProps {
  className?: string;
  glow?: boolean;
}

export const ZapretLogo: React.FC<ZapretLogoProps> = ({ className = "h-6 w-6", glow = false }) => {
  return (
    <div className={`relative flex items-center justify-center shrink-0 ${className}`}>
      {glow && (
        <div className="absolute inset-0 rounded-xl bg-gradient-to-tr from-cyan-500/30 to-blue-600/30 blur-md pointer-events-none" />
      )}
      <svg
        viewBox="0 0 32 32"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className="w-full h-full relative z-10 drop-shadow-sm"
      >
        <defs>
          <linearGradient id="zapretShieldGrad" x1="2" y1="2" x2="30" y2="30" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#06b6d4" />
            <stop offset="50%" stopColor="#3b82f6" />
            <stop offset="100%" stopColor="#6366f1" />
          </linearGradient>
          <linearGradient id="zapretInnerGrad" x1="8" y1="6" x2="24" y2="26" gradientUnits="userSpaceOnUse">
            <stop offset="0%" stopColor="#38bdf8" />
            <stop offset="100%" stopColor="#818cf8" />
          </linearGradient>
        </defs>

        {/* Shield Frame Outer */}
        <path
          d="M16 2.5L27 6.8V15C27 22.2 22.3 27.8 16 29.5C9.7 27.8 5 22.2 5 15V6.8L16 2.5Z"
          fill="url(#zapretShieldGrad)"
          fillOpacity="0.18"
          stroke="url(#zapretShieldGrad)"
          strokeWidth="1.75"
          strokeLinejoin="round"
        />

        {/* Inner Cyber Bypass 'Z' Network Geometry */}
        <path
          d="M10.5 10.5H21.5L12 21.5H21.5"
          stroke="url(#zapretInnerGrad)"
          strokeWidth="2.2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />

        {/* High-tech Signal Nodes */}
        <circle cx="21.5" cy="10.5" r="1.5" fill="#38bdf8" />
        <circle cx="10.5" cy="10.5" r="1.5" fill="#06b6d4" />
        <circle cx="12" cy="21.5" r="1.5" fill="#6366f1" />
        <circle cx="21.5" cy="21.5" r="1.5" fill="#818cf8" />
        <circle cx="16" cy="16" r="1.2" fill="#a5b4fc" />
      </svg>
    </div>
  );
};
