/**
 * App color system: runtime overrides for the accent family AND the
 * neutral glass surfaces.
 *
 * Picking a color recolors the whole app while keeping it dark: the
 * accent variables drive buttons, links, glows and gradients, and the
 * surface variables (app background, glass sidebar/bar/cards, inner
 * panels, inputs, overlay, aurora glows) are tinted toward the same
 * color. Passing null removes every override and restores the built-in
 * violet look from index.css byte for byte.
 *
 * Derivation rules were checked against the built-in violet so the
 * default look reproduces from the fill color alone: fill-hover equals
 * accent, wash is fill at 0.18 alpha, line is accent at 0.38, the glow
 * is accent at 0.16 plus fill at 0.25, and accent2 is the fill
 * hue-rotated for the duotone gradients.
 *
 * Surface tint rule: every neutral surface is mixed toward a DARK
 * version of the theme color (same hue, saturation capped at 0.85,
 * lightness pinned to 0.30), never toward the raw bright fill. That is
 * what keeps the app dark and the white text readable for bright fills
 * (cyan, amber) and for custom extremes (white, gray, black): the mix
 * target itself is dark, so no weight can wash a surface out. Note the
 * saturation is only capped, never floored: achromatic picks (white,
 * gray, black) honestly yield a neutral dark gray tint instead of
 * inventing a red hue from the arbitrary h=0 of grays. Weights are per
 * surface: the app backdrop shows the color most (0.50), glass panels
 * slightly less (0.45), solid panels 0.40, the modal overlay least
 * (0.30, it must stay near-black). Text (ink) colors are never touched.
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

/** CSS variables overridden by applyAccent, in the order they are set. */
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

/** Neutral surfaces tinted toward the theme color (see header). */
const SURFACE_VARS = [
  "--c-app",
  "--c-sidebar",
  "--c-card",
  "--c-cardin",
  "--c-raise",
  "--c-hovert",
  "--glass-sidebar-bg",
  "--glass-card-bg",
  "--glass-bar-bg",
  "--c-overlay",
  "--aurora-1",
  "--aurora-2",
  "--aurora-3",
  "--aurora-4",
  "--aurora-5",
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

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

/** Mixes two colors: w = 0 keeps a, w = 1 gives b. */
function mix(a: Rgb, b: Rgb, w: number): Rgb {
  return {
    r: a.r + (b.r - a.r) * w,
    g: a.g + (b.g - a.g) * w,
    b: a.b + (b.b - a.b) * w,
  };
}

/**
 * The dark mix target for surface tinting: the theme hue, saturation
 * capped at 0.85, lightness pinned to 0.30. Mixing any neutral surface
 * toward this can only darken-tint it, never wash it out, whatever
 * the user picks. Saturation is deliberately NOT floored: for
 * achromatic fills rgbToHsl reports h=0, and flooring s would invent
 * a red tint for white/gray/black picks.
 */
function darkTintSource(fill: Rgb): Rgb {
  const { h, s } = rgbToHsl(fill);
  return hslToRgb(h, clamp(s, 0, 0.85), 0.3);
}

/** Built-in violet surfaces from index.css :root (mix bases). */
const BASE_SURFACES: Record<string, Rgb> = {
  "--c-app": { r: 9, g: 7, b: 15 },
  "--c-sidebar": { r: 17, g: 13, b: 28 },
  "--c-card": { r: 23, g: 18, b: 38 },
  "--c-cardin": { r: 13, g: 10, b: 22 },
  "--c-raise": { r: 30, g: 24, b: 48 },
  "--c-hovert": { r: 39, g: 31, b: 62 },
  "--glass-sidebar-bg": { r: 26, g: 20, b: 44 },
  "--glass-card-bg": { r: 22, g: 17, b: 38 },
  "--glass-bar-bg": { r: 16, g: 12, b: 28 },
  "--c-overlay": { r: 5, g: 3, b: 10 },
};

/** Alphas preserved from index.css for the rgba surfaces. */
const SURFACE_ALPHA: Record<string, number> = {
  "--glass-sidebar-bg": 0.58,
  "--glass-card-bg": 0.5,
  "--glass-bar-bg": 0.55,
  "--c-overlay": 0.7,
};

/** Mix weight per surface (see header for the reasoning). */
const SURFACE_WEIGHT: Record<string, number> = {
  "--c-app": 0.5,
  "--glass-sidebar-bg": 0.45,
  "--glass-card-bg": 0.45,
  "--glass-bar-bg": 0.45,
  "--c-overlay": 0.3,
};

const DEFAULT_SURFACE_WEIGHT = 0.4;

/**
 * Applies a theme fill color (hex "#RRGGBB") or restores the built-in
 * violet when null. Returns false for a malformed value (nothing applied).
 * Besides the accent family it tints every neutral surface toward the
 * same color, so the whole app changes color while staying dark.
 */
export function applyAccent(fillHex: string | null): boolean {
  const root = document.documentElement;
  if (fillHex === null) {
    for (const v of [...ACCENT_VARS, ...SURFACE_VARS]) {
      root.style.removeProperty(v);
    }
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
  // Neutral surfaces: tint toward the dark theme color, never toward
  // the raw bright fill. Preset and custom fills go through the same
  // function, so a preset and its hex twin look identical.
  const source = darkTintSource(fill);
  for (const [name, base] of Object.entries(BASE_SURFACES)) {
    const w = SURFACE_WEIGHT[name] ?? DEFAULT_SURFACE_WEIGHT;
    const tinted = mix(base, source, w);
    const alpha = SURFACE_ALPHA[name];
    root.style.setProperty(
      name,
      alpha === undefined ? triplet(tinted) : rgba(tinted, alpha)
    );
  }
  // Aurora glows follow the vivid accent, not the dark tint source:
  // they are light, not surfaces, and their alphas are the built-in ones.
  root.style.setProperty("--aurora-1", rgba(fill, 0.26));
  root.style.setProperty("--aurora-2", rgba(accent2, 0.16));
  root.style.setProperty("--aurora-3", rgba(fill, 0.08));
  root.style.setProperty("--aurora-4", rgba(accent2, 0.13));
  root.style.setProperty("--aurora-5", rgba(fill, 0.18));
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
