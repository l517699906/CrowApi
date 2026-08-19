import type { UiTheme } from "../types";

export const DARK_THEME_MEDIA_QUERY = "(prefers-color-scheme: dark)";

export function resolveUiTheme(theme: UiTheme, prefersDark: boolean): Exclude<UiTheme, "system"> {
    return theme === "system" ? (prefersDark ? "dark" : "light") : theme;
}

export function applyUiTheme(theme: UiTheme, prefersDark = window.matchMedia(DARK_THEME_MEDIA_QUERY).matches) {
    document.documentElement.dataset.theme = resolveUiTheme(theme, prefersDark);
}
