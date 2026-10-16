// Applies the persisted UI theme before first paint to avoid a flash of the
// wrong theme. Loaded synchronously from <head> so it runs before rendering.
// (Extracted from an inline <script> so the app can ship a strict
// Content-Security-Policy without 'unsafe-inline'. Mirrors ThemeContext.initTheme.)
try {
  var t = localStorage.getItem("zapret2_theme");
  document.documentElement.dataset.theme =
    t === "hyprland" || t === "macos" ? t : "macos";
} catch (e) {
  document.documentElement.dataset.theme = "macos";
}
