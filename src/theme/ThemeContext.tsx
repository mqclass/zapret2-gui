import React, {
  createContext,
  useContext,
  useState,
  useEffect,
  useCallback,
} from "react";

export type ThemeId = "macos" | "hyprland";

interface ThemeContextValue {
  theme: ThemeId;
  setTheme: (theme: ThemeId) => void;
}

const STORAGE_KEY = "zapret2_theme";
const DEFAULT_THEME: ThemeId = "macos";

const ThemeContext = createContext<ThemeContextValue>({
  theme: DEFAULT_THEME,
  setTheme: () => {},
});

function readStoredTheme(): ThemeId {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "macos" || saved === "hyprland") {
      return saved;
    }
  } catch {
    /* storage unavailable — fall through to default */
  }
  return DEFAULT_THEME;
}

function applyThemeAttribute(theme: ThemeId) {
  document.documentElement.dataset.theme = theme;
}

/**
 * Call once, synchronously, before first render to avoid a theme flash.
 * (index.html also inlines the same logic for the pre-bundle paint.)
 */
export function initTheme(): ThemeId {
  const theme = readStoredTheme();
  applyThemeAttribute(theme);
  return theme;
}

export const ThemeProvider: React.FC<{ children: React.ReactNode }> = ({
  children,
}) => {
  const [theme, setThemeState] = useState<ThemeId>(() => {
    const initial = readStoredTheme();
    applyThemeAttribute(initial);
    return initial;
  });

  const setTheme = useCallback((next: ThemeId) => {
    setThemeState(next);
    applyThemeAttribute(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* storage unavailable — theme still applies for this session */
    }
  }, []);

  // Keep the attribute in sync if something else mutated it.
  useEffect(() => {
    applyThemeAttribute(theme);
  }, [theme]);

  return (
    <ThemeContext.Provider value={{ theme, setTheme }}>
      {children}
    </ThemeContext.Provider>
  );
};

export const useTheme = () => useContext(ThemeContext);

export const THEME_META: { id: ThemeId; accent: string }[] = [
  { id: "macos", accent: "#0a84ff" },
  { id: "hyprland", accent: "#a3e635" },
];
