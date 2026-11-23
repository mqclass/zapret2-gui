/**
 * App color system: runtime overrides for the accent family only.
 *
 * Picking a color recolors buttons, links, glows and gradients. The
 * black surfaces, the bone text and the dawn glow never change: they
 * are the sunrise theme, not the accent. Passing null removes every
 * override and restores the built-in sunrise look from index.css byte
 * for byte. The sun disc itself is hardcoded sunrise and never follows
 * the accent either.
 *
 * Derivation rules were checked against the built-in sunrise so the
 * default look reproduces from the fill color alone: fill-hover equals
 * accent, wash is fill at 0.18 alpha, line is accent at 0.38, the glow
 * is accent at 0.16 plus fill at 0.25, and accent2 is the fill
 * hue-rotated for the duotone gradients (the sunrise preset pins it
 * to the red rim instead).
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
  /** Exact soft tone when it differs from the derived one. */
  soft?: string;
  /** Exact duotone partner when it differs from the hue rotation. */
  accent2?: string;
}

export const DEFAULT_ACCENT_FILL = "#EA580C";

export const ACCENT_PRESETS: AccentPreset[] = [
  { id: "sunrise", fill: "#EA580C", accent: "#F97316", soft: "#FDBA74", accent2: "#E2342A" },
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
 * Applies a theme fill color (hex "#RRGGBB") or restores the built-in
 * sunrise when null. Returns false for a malformed value (nothing
 * applied). Only the accent family is overridden: the black surfaces,
 * the bone text and the dawn glow belong to the theme and never move.
 */
export function applyAccent(fillHex: string | null): boolean {
  const root = document.documentElement;
  if (fillHex === null) {
    for (const v of ACCENT_VARS) {
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
  const accent2 = preset?.accent2
    ? hexToRgb(preset.accent2)!
    : rotateHue(fill, 40);
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

/** The fill hex currently in effect: stored override or the built-in sunrise. */
export function currentAccentFill(stored: string | null | undefined): string {
  return stored ?? DEFAULT_ACCENT_FILL;
}

/**
 * Early-boot mirror of the accent in localStorage. The backend settings
 * file is the source of truth, but it only arrives after an async IPC
 * round trip, which used to flash one sunrise frame on every launch for
 * non-sunrise users. main.tsx applies this mirror synchronously before
 * the first render; App overwrites it with the backend value once
 * settings load, and mirrors every confirmed change back here. If the
 * settings fetch fails, the mirror is what keeps the saved accent from
 * silently falling back to sunrise.
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

/**
 * Unconfirmed custom-color draft, separate from the mirror above.
 *
 * The custom picker previews instantly but saves debounced (450 ms). If
 * the app window closes inside that window, the webview dies with the
 * timer and the draft would be lost: React unmount cleanup cannot be
 * relied on at process exit. So every drag step also writes the draft
 * here, and App reconciles it once on the next launch (applies it and
 * saves it to the backend, one shot). The key is cleared on flush,
 * on cancel, and on preset pick, so a stale draft can never override
 * a newer confirmed color: it only ever holds a value the backend
 * has not confirmed yet, and re-saving that same value is harmless
 * even if the app died after the backend write but before the clear.
 */
const ACCENT_DRAFT_KEY = "zapret2.accent_draft";

export function readDraftAccent(): string | null {
  try {
    const v = localStorage.getItem(ACCENT_DRAFT_KEY);
    return v && hexToRgb(v) ? v : null;
  } catch {
    return null;
  }
}

export function writeDraftAccent(color: string): void {
  try {
    localStorage.setItem(ACCENT_DRAFT_KEY, color);
  } catch {
    // Draft recovery is best-effort; the debounced save still runs.
  }
}

export function clearDraftAccent(): void {
  try {
    localStorage.removeItem(ACCENT_DRAFT_KEY);
  } catch {
    // Best-effort; a leftover draft is re-validated on next launch.
  }
}

/**
 * Clears the draft only if it still holds `value`. A queued save must
 * not wipe a newer draft: drag A, flush in flight, drag B, then A's
 * resolve must leave B alone, or B is lost if the app dies before
 * B's own flush. Cancel (preset pick) is the one place that clears
 * unconditionally, because the preset is the newer intent there.
 */
export function clearDraftAccentIf(value: string): void {
  try {
    const v = localStorage.getItem(ACCENT_DRAFT_KEY);
    if (v && v.toLowerCase() === value.toLowerCase()) {
      localStorage.removeItem(ACCENT_DRAFT_KEY);
    }
  } catch {
    // Best-effort.
  }
}
