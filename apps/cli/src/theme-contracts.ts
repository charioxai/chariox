export const DEFAULT_THEME_ID = "opencode"
export const THEME_NAMES = [
  "opencode",
  "tokyonight",
  "catppuccin",
  "vercel",
  "sober",
  "matrix",
  "gruvbox",
  "nord",
  "dracula",
  "monokai",
] as const

export type ThemeName = string

export const THEME_TOKEN_KEYS = [
  "primary",
  "secondary",
  "accent",
  "error",
  "warning",
  "success",
  "info",
  "text",
  "textMuted",
  "background",
  "backgroundPanel",
  "backgroundElement",
  "border",
  "borderActive",
  "borderSubtle",
  "syntaxComment",
  "syntaxKeyword",
  "syntaxFunction",
  "syntaxVariable",
  "syntaxString",
  "syntaxNumber",
  "syntaxType",
  "syntaxOperator",
  "syntaxPunctuation",
  "markdownHeading",
  "markdownLink",
  "markdownLinkText",
  "markdownCode",
  "markdownBlockQuote",
  "markdownEmph",
  "markdownStrong",
  "markdownListItem",
] as const

export type ThemeTokenKey = typeof THEME_TOKEN_KEYS[number]
export type ThemeTokenColors = Record<ThemeTokenKey, string>

export type ThemeDefinition = {
  id: ThemeName
  name: string
  colors: ThemeTokenColors
  source: "builtin" | "user"
  filePath?: string
}

export type ThemeRegistry = {
  themes: Map<ThemeName, ThemeDefinition>
  orderedIds: ThemeName[]
}
