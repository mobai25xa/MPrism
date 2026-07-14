export type ThemePreference = "system" | "light" | "dark";
export type ResolvedTheme = "light" | "dark";

const STORAGE_KEY = "mprism.theme";

export function readThemePreference(): ThemePreference {
  const value = localStorage.getItem(STORAGE_KEY);
  if (value === "light" || value === "dark" || value === "system") {
    return value;
  }
  return "system";
}

export function writeThemePreference(theme: ThemePreference): void {
  localStorage.setItem(STORAGE_KEY, theme);
}

export function resolveTheme(preference: ThemePreference): ResolvedTheme {
  if (preference === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }
  return preference;
}

/**
 * Apply fytheme dual theme.
 *
 * Token truth in @mobai6462/theme/base.css:
 * - :root = dark console defaults
 * - [data-theme=light] = light paper overrides
 *
 * Official docs set data-theme to "dark" | "light". Setting "dark" is
 * equivalent to the :root defaults (there is no [data-theme=dark] block).
 */
export function applyResolvedTheme(mode: ResolvedTheme): void {
  const root = document.documentElement;
  root.dataset.theme = mode;
  root.style.colorScheme = mode;
}
