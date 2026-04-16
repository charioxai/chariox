export const THEME_NAMES = ["opencode", "tokyonight", "catppuccin", "vercel", "sober"] as const
export type ThemeName = typeof THEME_NAMES[number]

const THEME_LABELS: Record<ThemeName, string> = {
  opencode: "OpenCode",
  tokyonight: "Tokyonight",
  catppuccin: "Catppuccin",
  vercel: "Vercel",
  sober: "Sober",
}

export function normalizeThemeName(value: unknown): ThemeName {
  return typeof value === "string" && THEME_NAMES.includes(value as ThemeName)
    ? value as ThemeName
    : "opencode"
}

export function themeLabel(value: unknown) {
  return THEME_LABELS[normalizeThemeName(value)]
}

export function themeOptions() {
  return THEME_NAMES.map((id) => ({
    id,
    label: THEME_LABELS[id],
  }))
}
