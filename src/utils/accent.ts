/**
 * Accent color system: runtime overrides for the --c-accent family.
 *
 * The dark glass base theme (index.css) is never touched: only the accent
 * variables are overridden on documentElement, and Tailwind tokens read
 * those variables, so every accent surface recolors at once. Passing null
 * removes the overrides and restores the built-in violet from index.css.
 *
 * Derivation rules were checked against the built-in violet so the default
 * look reproduces from the fill color alone: fill-hover equals accent,
 * wash is fill at 0.18 alpha, line is accent at 0.38, the glow is accent
 * at 0.16 plus fill at 0.25, and accent2 is the fill hue-rotated for the
 * duotone gradients.
 */

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

export interface AccentPreset {
  id: string;
  /** Button fills and glows. */
  fill: string;
  /** Text, icons and hover fills. */
  accent: string;
  /** Exact soft tone when it differs from the derived one (violet only). */
  soft?: string;
}

export const DEFAULT_ACCENT_FILL = "#8B5CF6";

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "violet", fill: "#8B5CF6", accent: "#A78BFA", soft: "#C4B5FD" },
  { id: "cyan", fill: "#06B6D4", accent: "#22D3EE" },
  { id: "emerald", fill: "#10B981", accent: "#34D399" },
  { id: "amber", fill: "#F59E0B", accent: "#FBBF24" },
  { id: "pink", fill: "#EC4899", accent: "#F472B6" },
  { id: "blue", fill: "#3B82F6", accent: "#60A5FA" },
];

/** CSS variables of the accent family, in the order they are overridden. */
const ACCENT_VARS = [
  "--c-accent",
  "--c-accent-soft",
  "--c-accent-fill",
  "--c-accent-fill-hover",
  "--c-accent-contrast",
  "--c-accent-wash",
  "--c-accent-line",
  "--c-accent2",
  "--shadow-glow",
];

export function hexToRgb(hex: string): Rgb | null {
  const clean = hex.startsWith("#") ? hex.slice(1) : hex;
  if (clean.length !== 6 || !/^[0-9a-fA-F]{6}$/.test(clean)) return null;
  return {
    r: parseInt(clean.slice(0, 2), 16),
    g: parseInt(clean.slice(2, 4), 16),
    b: parseInt(clean.slice(4, 6), 16),
  };
}

function triplet(c: Rgb): string {
  return `${Math.round(c.r)} ${Math.round(c.g)} ${Math.round(c.b)}`;
}

function rgba(c: Rgb, alpha: number): string {
  return `rgba(${Math.round(c.r)}, ${Math.round(c.g)}, ${Math.round(c.b)}, ${alpha})`;
}

/** Mixes a color toward white by t (0 = unchanged, 1 = white). */
function lighten(c: Rgb, t: number): Rgb {
  return {
    r: c.r + (255 - c.r) * t,
    g: c.g + (255 - c.g) * t,
    b: c.b + (255 - c.b) * t,
  };
}

function rgbToHsl(c: Rgb): { h: number; s: number; l: number } {
  const r = c.r / 255;
  const g = c.g / 255;
  const b = c.b / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return { h: 0, s: 0, l };
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h = 0;
  if (max === r) h = ((g - b) / d + (g < b ? 6 : 0)) * 60;
  else if (max === g) h = ((b - r) / d + 2) * 60;
  else h = ((r - g) / d + 4) * 60;
  return { h, s, l };
}

function hslToRgb(h: number, s: number, l: number): Rgb {
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0;
  let g = 0;
  let b = 0;
  if (h < 60) {
    r = c;
    g = x;
  } else if (h < 120) {
    r = x;
    g = c;
  } else if (h < 180) {
    g = c;
    b = x;
  } else if (h < 240) {
    g = x;
    b = c;
  } else if (h < 300) {
    r = x;
    b = c;
  } else {
    r = c;
    b = x;
  }
  return {
    r: (r + m) * 255,
    g: (g + m) * 255,
    b: (b + m) * 255,
  };
}

function rotateHue(c: Rgb, degrees: number): Rgb {
  const { h, s, l } = rgbToHsl(c);
  return hslToRgb((h + degrees + 360) % 360, s, l);
}

/** Relative luminance (WCAG): white text needs a dark fill to stay readable. */
function luminance(c: Rgb): number {
  const linear = (v: number): number => {
    const s = v / 255;
    return s <= 0.03928 ? s / 12.92 : Math.pow((s + 0.055) / 1.055, 2.4);
  };
  return (
    0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
  );
}

/**
 * Applies an accent fill color (hex "#RRGGBB") or restores the built-in
 * violet when null. Returns false for a malformed value (nothing applied).
 */
export function applyAccent(fillHex: string | null): boolean {
  const root = document.documentElement;
  if (fillHex === null) {
    for (const v of ACCENT_VARS) root.style.removeProperty(v);
    return true;
  }
  const fill = hexToRgb(fillHex);
  if (!fill) return false;
  const preset = ACCENT_PRESETS.find(
    (p) => p.fill.toLowerCase() === fillHex.toLowerCase()
  );
  const accent = preset ? hexToRgb(preset.accent)! : lighten(fill, 0.28);
  const soft = preset?.soft
    ? hexToRgb(preset.soft)!
    : lighten(fill, 0.55);
  const contrast: Rgb =
    luminance(fill) > 0.35 ? { r: 0, g: 0, b: 0 } : { r: 255, g: 255, b: 255 };
  const accent2 = rotateHue(fill, 40);
  root.style.setProperty("--c-accent", triplet(accent));
  root.style.setProperty("--c-accent-soft", triplet(soft));
  root.style.setProperty("--c-accent-fill", triplet(fill));
  root.style.setProperty("--c-accent-fill-hover", triplet(accent));
  root.style.setProperty("--c-accent-contrast", triplet(contrast));
  root.style.setProperty("--c-accent-wash", rgba(fill, 0.18));
  root.style.setProperty("--c-accent-line", rgba(accent, 0.38));
  root.style.setProperty("--c-accent2", triplet(accent2));
  root.style.setProperty(
    "--shadow-glow",
    `0 0 0 1px ${rgba(accent, 0.16)}, 0 0 34px ${rgba(fill, 0.25)}`
  );
  return true;
}

/** The fill hex currently in effect: stored override or the built-in violet. */
export function currentAccentFill(stored: string | null | undefined): string {
  return stored ?? DEFAULT_ACCENT_FILL;
}

/**
 * Early-boot mirror of the accent in localStorage. The backend settings
 * file is the source of truth, but it only arrives after an async IPC
 * round trip, which used to flash one violet frame on every launch for
 * non-violet users. main.tsx applies this mirror synchronously before
 * the first render; App overwrites it with the backend value once
 * settings load, and mirrors every confirmed change back here. If the
 * settings fetch fails, the mirror is what keeps the saved accent from
 * silently falling back to violet.
 */
const ACCENT_STORAGE_KEY = "zapret2.accent_color";

export function readStoredAccent(): string | null {
  try {
    const v = localStorage.getItem(ACCENT_STORAGE_KEY);
    return v && hexToRgb(v) ? v : null;
  } catch {
    return null;
  }
}

export function mirrorAccentToStorage(color: string | null): void {
  try {
    if (color === null) localStorage.removeItem(ACCENT_STORAGE_KEY);
    else localStorage.setItem(ACCENT_STORAGE_KEY, color);
  } catch {
    // Storage unavailable (private mode, quota): the backend value still
    // applies on load; only the early frame is lost.
  }
}
