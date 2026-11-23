import React from "react";

interface SpotlightCardProps {
  className?: string;
  children: React.ReactNode;
}

/**
 * Shared mousemove handler for spotlight surfaces: writes the cursor
 * position (relative to the element itself) straight to the DOM node,
 * so moving the mouse never re-renders. Attach it with the
 * `spotlight-card` class to any element — buttons and list rows
 * included, where a wrapper div would break the layout.
 */
export function handleSpotlightMove(e: React.MouseEvent<HTMLElement>) {
  const el = e.currentTarget;
  const r = el.getBoundingClientRect();
  el.style.setProperty("--mx", `${e.clientX - r.left}px`);
  el.style.setProperty("--my", `${e.clientY - r.top}px`);
}

/**
 * Card with a cursor-tracking gold glow (Gemini dawn port, see
 * .spotlight-card in index.css). The cursor position is written
 * straight to the DOM node, so moving the mouse never re-renders.
 * Renders a plain div carrying the given className, so it can stand
 * in for any card root (grid children included).
 */
export const SpotlightCard: React.FC<SpotlightCardProps> = React.memo(
  ({ className, children }) => {
    return (
      <div
        onMouseMove={handleSpotlightMove}
        className={`spotlight-card ${className ?? ""}`}
      >
        {children}
      </div>
    );
  }
);
