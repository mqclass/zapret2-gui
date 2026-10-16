/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        brand: {
          50: "#eff6ff",
          100: "#dbeafe",
          500: "#3b82f6",
          600: "#2563eb",
          700: "#1d4ed8",
          800: "#1e40af",
          900: "#1e3a8a",
        },
        // ── Dual-theme design tokens (driven by CSS vars in src/index.css) ──
        // Solid colors are RGB triplets: rgb(var(--x) / <alpha-value>) keeps
        // Tailwind opacity modifiers (bg-card/40, …) working in both themes.
        // Surface scale: app background → sidebar → card → inner → raised → hover tint
        app: "rgb(var(--c-app) / <alpha-value>)",
        sidebar: "rgb(var(--c-sidebar) / <alpha-value>)",
        card: "rgb(var(--c-card) / <alpha-value>)",
        cardin: "rgb(var(--c-cardin) / <alpha-value>)",
        raise: "rgb(var(--c-raise) / <alpha-value>)",
        hovert: "rgb(var(--c-hovert) / <alpha-value>)",
        overlay: "var(--c-overlay)",
        // Borders
        line: "rgb(var(--c-line) / <alpha-value>)",
        linestrong: "rgb(var(--c-linestrong) / <alpha-value>)",
        // Text scale: ink (primary) → ink5 (faint)
        ink: "rgb(var(--c-ink) / <alpha-value>)",
        ink2: "rgb(var(--c-ink2) / <alpha-value>)",
        ink3: "rgb(var(--c-ink3) / <alpha-value>)",
        ink4: "rgb(var(--c-ink4) / <alpha-value>)",
        ink5: "rgb(var(--c-ink5) / <alpha-value>)",
        // Accent scale (theme-specific: calm blue on macOS, neon lime on Hyprland)
        accent: {
          DEFAULT: "rgb(var(--c-accent) / <alpha-value>)",
          soft: "rgb(var(--c-accent-soft) / <alpha-value>)",
          fill: "rgb(var(--c-accent-fill) / <alpha-value>)",
          "fill-hover": "rgb(var(--c-accent-fill-hover) / <alpha-value>)",
          contrast: "rgb(var(--c-accent-contrast) / <alpha-value>)",
          wash: "var(--c-accent-wash)",
          line: "var(--c-accent-line)",
        },
        // Danger scale (stop buttons, destructive actions)
        danger: {
          fill: "rgb(var(--c-danger-fill) / <alpha-value>)",
          "fill-hover": "rgb(var(--c-danger-fill-hover) / <alpha-value>)",
          wash: "var(--c-danger-wash)",
          line: "var(--c-danger-line)",
        },
        // Tinted washes
        okwash: "var(--c-ok-wash)",
        warnwash: "var(--c-warn-wash)",
        warnline: "var(--c-warn-line)",
      },
      fontFamily: {
        sans: ["var(--font-sans)"],
        mono: ['"JetBrains Mono"', '"Fira Code"', "Consolas", "monospace"],
      },
      borderRadius: {
        card: "var(--r-card)",
        panel: "var(--r-panel)",
        control: "var(--r-control)",
        chip: "var(--r-chip)",
      },
      boxShadow: {
        card: "var(--shadow-card)",
        pop: "var(--shadow-pop)",
        glow: "var(--shadow-glow)",
      },
    },
  },
  plugins: [],
};
