import path from "node:path"
import { readFile } from "node:fs/promises"
import {
  themeDirectories,
  themeFiles,
  type ThemeLoadOptions,
  type ThemeLoadWarning,
} from "./theme-file-source.js"

export {
  themeDirectories,
  type ThemeLoadWarning,
} from "./theme-file-source.js"

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

type PartialThemeTokenColors = Partial<Record<ThemeTokenKey, string>>

const BUILTIN_THEME_DEFINITIONS = [
  defineTheme("opencode", "OpenCode", {
    primary: "#fab283",
    secondary: "#5c9cf5",
    accent: "#9d7cd8",
    error: "#e06c75",
    warning: "#f5a742",
    success: "#7fd88f",
    info: "#56b6c2",
    text: "#eeeeee",
    textMuted: "#808080",
    background: "#0a0a0a",
    backgroundPanel: "#141414",
    backgroundElement: "#1e1e1e",
    border: "#484848",
    borderActive: "#606060",
    borderSubtle: "#3c3c3c",
    syntaxComment: "#6e7681",
    syntaxKeyword: "#fab283",
    syntaxFunction: "#7fb4ff",
    syntaxVariable: "#eeeeee",
    syntaxString: "#7fd88f",
    syntaxNumber: "#e9b872",
    syntaxType: "#c3e88d",
    syntaxOperator: "#d4d4d4",
    syntaxPunctuation: "#8b949e",
    markdownHeading: "#f5c26b",
    markdownLink: "#7fb4ff",
    markdownLinkText: "#56b6c2",
    markdownCode: "#7fd88f",
    markdownBlockQuote: "#d3a55b",
    markdownEmph: "#d3a55b",
    markdownStrong: "#f5c26b",
    markdownListItem: "#5c9cf5",
  }),
  defineTheme("tokyonight", "Tokyonight", {
    primary: "#7aa2f7",
    secondary: "#7dcfff",
    accent: "#bb9af7",
    error: "#f7768e",
    warning: "#e0af68",
    success: "#9ece6a",
    info: "#7dcfff",
    text: "#c0caf5",
    textMuted: "#565f89",
    background: "#1a1b26",
    backgroundPanel: "#222436",
    backgroundElement: "#2f334d",
    border: "#3b4261",
    borderActive: "#565f89",
    borderSubtle: "#292e42",
    syntaxComment: "#565f89",
    syntaxKeyword: "#bb9af7",
    syntaxFunction: "#7aa2f7",
    syntaxVariable: "#c0caf5",
    syntaxString: "#9ece6a",
    syntaxNumber: "#ff9e64",
    syntaxType: "#2ac3de",
    syntaxOperator: "#89ddff",
    syntaxPunctuation: "#a9b1d6",
    markdownHeading: "#bb9af7",
    markdownLink: "#7aa2f7",
    markdownLinkText: "#7dcfff",
    markdownCode: "#9ece6a",
    markdownBlockQuote: "#e0af68",
    markdownEmph: "#e0af68",
    markdownStrong: "#ff9e64",
    markdownListItem: "#7aa2f7",
  }),
  defineTheme("catppuccin", "Catppuccin", {
    primary: "#b4befe",
    secondary: "#89b4fa",
    accent: "#f38ba8",
    error: "#f38ba8",
    warning: "#f4b8e4",
    success: "#a6d189",
    info: "#89dceb",
    text: "#cdd6f4",
    textMuted: "#6c7086",
    background: "#1e1e2e",
    backgroundPanel: "#242438",
    backgroundElement: "#313244",
    border: "#45475a",
    borderActive: "#585b70",
    borderSubtle: "#383a4c",
    syntaxComment: "#6c7086",
    syntaxKeyword: "#cba6f7",
    syntaxFunction: "#89b4fa",
    syntaxVariable: "#cdd6f4",
    syntaxString: "#a6d189",
    syntaxNumber: "#fab387",
    syntaxType: "#94e2d5",
    syntaxOperator: "#f5c2e7",
    syntaxPunctuation: "#bac2de",
    markdownHeading: "#cba6f7",
    markdownLink: "#89b4fa",
    markdownLinkText: "#89dceb",
    markdownCode: "#a6d189",
    markdownBlockQuote: "#f4b8e4",
    markdownEmph: "#f4b8e4",
    markdownStrong: "#fab387",
    markdownListItem: "#b4befe",
  }),
  defineTheme("vercel", "Vercel", {
    primary: "#52a8ff",
    secondary: "#0070f3",
    accent: "#bf7af0",
    error: "#e5484d",
    warning: "#ffb224",
    success: "#46a758",
    info: "#52a8ff",
    text: "#ededed",
    textMuted: "#878787",
    background: "#000000",
    backgroundPanel: "#111111",
    backgroundElement: "#1f1f1f",
    border: "#333333",
    borderActive: "#525252",
    borderSubtle: "#262626",
    syntaxComment: "#878787",
    syntaxKeyword: "#f75590",
    syntaxFunction: "#52a8ff",
    syntaxVariable: "#ededed",
    syntaxString: "#63c46d",
    syntaxNumber: "#f2a700",
    syntaxType: "#0ac7ac",
    syntaxOperator: "#f75590",
    syntaxPunctuation: "#ededed",
    markdownHeading: "#bf7af0",
    markdownLink: "#52a8ff",
    markdownLinkText: "#0ac7ac",
    markdownCode: "#63c46d",
    markdownBlockQuote: "#878787",
    markdownEmph: "#f2a700",
    markdownStrong: "#f75590",
    markdownListItem: "#ededed",
  }),
  defineTheme("sober", "Sober", {
    primary: "#e6e6e6",
    secondary: "#b8b8b8",
    accent: "#d0d0d0",
    error: "#d7d7d7",
    warning: "#c4c4c4",
    success: "#dadada",
    info: "#bdbdbd",
    text: "#e8e8e8",
    textMuted: "#8a8a8a",
    background: "#080808",
    backgroundPanel: "#121212",
    backgroundElement: "#1b1b1b",
    border: "#3a3a3a",
    borderActive: "#626262",
    borderSubtle: "#2c2c2c",
    syntaxComment: "#777777",
    syntaxKeyword: "#e6e6e6",
    syntaxFunction: "#d6d6d6",
    syntaxVariable: "#e8e8e8",
    syntaxString: "#cfcfcf",
    syntaxNumber: "#c2c2c2",
    syntaxType: "#dcdcdc",
    syntaxOperator: "#bdbdbd",
    syntaxPunctuation: "#9a9a9a",
    markdownHeading: "#f0f0f0",
    markdownLink: "#d0d0d0",
    markdownLinkText: "#bdbdbd",
    markdownCode: "#cfcfcf",
    markdownBlockQuote: "#a8a8a8",
    markdownEmph: "#a8a8a8",
    markdownStrong: "#f0f0f0",
    markdownListItem: "#b8b8b8",
  }),
  fromOpenCodeDark("matrix", "Matrix", {
    palette: {
      neutral: "#000000",
      ink: "#62ff94",
      primary: "#2eff6a",
      accent: "#c770ff",
      success: "#62ff94",
      warning: "#e6ff57",
      error: "#ff4b4b",
      info: "#30b3ff",
    },
    overrides: {
      "text-weak": "#8ca391",
      "syntax-comment": "#8ca391",
      "syntax-keyword": "#c770ff",
      "syntax-string": "#1cc24b",
      "syntax-primitive": "#30b3ff",
      "syntax-variable": "#62ff94",
      "syntax-property": "#24f6d9",
      "syntax-type": "#e6ff57",
      "syntax-constant": "#ffa83d",
      "syntax-operator": "#24f6d9",
      "syntax-punctuation": "#62ff94",
      "markdown-heading": "#00efff",
      "markdown-link": "#30b3ff",
      "markdown-link-text": "#24f6d9",
      "markdown-code": "#1cc24b",
      "markdown-block-quote": "#8ca391",
      "markdown-emph": "#ffa83d",
      "markdown-strong": "#e6ff57",
      "markdown-list-item": "#30b3ff",
    },
  }),
  fromOpenCodeDark("gruvbox", "Gruvbox", {
    palette: {
      neutral: "#282828",
      ink: "#ebdbb2",
      primary: "#83a598",
      accent: "#fb4934",
      success: "#b8bb26",
      warning: "#fabd2f",
      error: "#fb4934",
      info: "#d3869b",
    },
    overrides: {
      "syntax-comment": "#928374",
      "syntax-keyword": "#fb4934",
      "syntax-primitive": "#83a598",
      "syntax-constant": "#d3869b",
    },
  }),
  fromOpenCodeDark("nord", "Nord", {
    palette: {
      neutral: "#2e3440",
      ink: "#e5e9f0",
      primary: "#88c0d0",
      accent: "#d57780",
      success: "#a3be8c",
      warning: "#d08770",
      error: "#bf616a",
      info: "#81a1c1",
    },
    overrides: {
      "syntax-comment": "#616e88",
      "syntax-keyword": "#81a1c1",
      "syntax-primitive": "#88c0d0",
      "syntax-constant": "#b48ead",
    },
  }),
  fromOpenCodeDark("dracula", "Dracula", {
    palette: {
      neutral: "#1d1e28",
      ink: "#f8f8f2",
      primary: "#bd93f9",
      accent: "#ff79c6",
      success: "#50fa7b",
      warning: "#ffb86c",
      error: "#ff5555",
      info: "#8be9fd",
    },
    overrides: {
      "syntax-comment": "#6272a4",
      "syntax-keyword": "#ff79c6",
      "syntax-string": "#f1fa8c",
      "syntax-primitive": "#50fa7b",
      "syntax-property": "#8be9fd",
      "syntax-constant": "#bd93f9",
    },
  }),
  fromOpenCodeDark("monokai", "Monokai", {
    palette: {
      neutral: "#272822",
      ink: "#f8f8f2",
      primary: "#ae81ff",
      accent: "#f92672",
      success: "#a6e22e",
      warning: "#fd971f",
      error: "#f92672",
      info: "#66d9ef",
    },
    overrides: {
      "syntax-comment": "#75715e",
      "syntax-keyword": "#f92672",
      "syntax-string": "#e6db74",
      "syntax-primitive": "#a6e22e",
      "syntax-property": "#66d9ef",
      "syntax-constant": "#ae81ff",
    },
  }),
]

const BUILTIN_THEME_IDS = new Set(BUILTIN_THEME_DEFINITIONS.map((theme) => theme.id))

export const DEFAULT_THEME_REGISTRY = createThemeRegistry(BUILTIN_THEME_DEFINITIONS)

export function normalizeThemeName(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY): ThemeName {
  return typeof value === "string" && registry.themes.has(value)
    ? value
    : DEFAULT_THEME_ID
}

export function themeLabel(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return themeDefinition(value, registry).name
}

export function themeOptions(registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return registry.orderedIds.map((id) => ({
    id,
    label: registry.themes.get(id)?.name ?? id,
  }))
}

export function themeDefinition(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return registry.themes.get(normalizeThemeName(value, registry)) ?? registry.themes.get(DEFAULT_THEME_ID)!
}

export async function loadThemeRegistry(options: ThemeLoadOptions = {}): Promise<ThemeRegistry> {
  const directories = options.directories ?? themeDirectories(options.workspace)
  const customThemes: ThemeDefinition[] = []
  for (const directory of directories) {
    const files = await themeFiles(directory)
    for (const filePath of files) {
      try {
        const raw = JSON.parse(await readFile(filePath, "utf8")) as unknown
        const theme = parseThemeDefinition(raw, filePath)
        if (BUILTIN_THEME_IDS.has(theme.id)) {
          options.onWarning?.({ path: filePath, message: `custom theme id '${theme.id}' conflicts with a built-in theme` })
          continue
        }
        customThemes.push(theme)
      } catch (error) {
        options.onWarning?.({ path: filePath, message: error instanceof Error ? error.message : String(error) })
      }
    }
  }
  return createThemeRegistry([...BUILTIN_THEME_DEFINITIONS, ...customThemes])
}

export function parseThemeDefinition(raw: unknown, filePath = "<theme>"): ThemeDefinition {
  if (!isRecord(raw)) {
    throw new Error("theme file must contain a JSON object")
  }
  const id = parseThemeId(raw.id, filePath)
  const name = typeof raw.name === "string" && raw.name.trim() ? raw.name.trim() : titleFromThemeId(id)

  if (isRecord(raw.dark) && isRecord(raw.dark.palette)) {
    return {
      ...fromOpenCodeDark(id, name, {
        palette: raw.dark.palette,
        overrides: isRecord(raw.dark.overrides) ? raw.dark.overrides : {},
      }),
      source: "user",
      filePath,
    }
  }

  if (isRecord(raw.theme)) {
    return {
      ...fromOpenCodeTuiTheme(id, name, {
        defs: isRecord(raw.defs) ? raw.defs : {},
        theme: raw.theme,
      }),
      source: "user",
      filePath,
    }
  }

  if (!isRecord(raw.palette)) {
    throw new Error("theme must include a native palette object, an OpenCode TUI theme object, or an OpenCode desktop dark.palette object")
  }

  return {
    id,
    name,
    source: "user",
    filePath,
    colors: mergeThemeColors({
      ...nativePaletteColors(raw.palette),
      ...nativeSyntaxColors(isRecord(raw.syntax) ? raw.syntax : {}),
      ...nativeMarkdownColors(isRecord(raw.markdown) ? raw.markdown : {}),
    }),
  }
}

function defineTheme(id: ThemeName, name: string, colors: ThemeTokenColors): ThemeDefinition {
  return {
    id,
    name,
    source: "builtin",
    colors: validateThemeColors(colors),
  }
}

function createThemeRegistry(themes: ThemeDefinition[]): ThemeRegistry {
  const map = new Map<ThemeName, ThemeDefinition>()
  const orderedIds: ThemeName[] = []
  for (const theme of themes) {
    if (!map.has(theme.id)) {
      orderedIds.push(theme.id)
    }
    map.set(theme.id, theme)
  }
  return { themes: map, orderedIds }
}

function fromOpenCodeDark(
  id: ThemeName,
  name: string,
  variant: {
    palette: Record<string, unknown>
    overrides?: Record<string, unknown>
  },
): ThemeDefinition {
  const palette = variant.palette
  const overrides = variant.overrides ?? {}
  const neutral = hex(palette.neutral, "dark.palette.neutral")
  const ink = hex(palette.ink, "dark.palette.ink")
  const primary = hex(palette.primary, "dark.palette.primary")
  const accent = optionalHex(palette.accent) ?? primary
  const success = optionalHex(palette.success) ?? primary
  const warning = optionalHex(palette.warning) ?? accent
  const error = optionalHex(palette.error) ?? accent
  const info = optionalHex(palette.info) ?? primary
  const textMuted = optionalHex(overrides["text-weak"]) ?? mixHex(neutral, ink, 0.48)
  const panelMix = isDarkHex(neutral) ? 0.08 : 0.04
  const elementMix = isDarkHex(neutral) ? 0.14 : 0.08
  return defineTheme(id, name, {
    primary,
    secondary: info,
    accent,
    error,
    warning,
    success,
    info,
    text: ink,
    textMuted,
    background: neutral,
    backgroundPanel: mixHex(neutral, ink, panelMix),
    backgroundElement: mixHex(neutral, ink, elementMix),
    border: mixHex(neutral, ink, isDarkHex(neutral) ? 0.28 : 0.2),
    borderActive: mixHex(neutral, ink, isDarkHex(neutral) ? 0.38 : 0.3),
    borderSubtle: mixHex(neutral, ink, isDarkHex(neutral) ? 0.2 : 0.14),
    syntaxComment: optionalHex(overrides["syntax-comment"]) ?? textMuted,
    syntaxKeyword: optionalHex(overrides["syntax-keyword"]) ?? accent,
    syntaxFunction: optionalHex(overrides["syntax-function"]) ?? optionalHex(overrides["syntax-primitive"]) ?? primary,
    syntaxVariable: optionalHex(overrides["syntax-variable"]) ?? ink,
    syntaxString: optionalHex(overrides["syntax-string"]) ?? success,
    syntaxNumber: optionalHex(overrides["syntax-number"]) ?? optionalHex(overrides["syntax-constant"]) ?? warning,
    syntaxType: optionalHex(overrides["syntax-type"]) ?? optionalHex(overrides["syntax-property"]) ?? info,
    syntaxOperator: optionalHex(overrides["syntax-operator"]) ?? info,
    syntaxPunctuation: optionalHex(overrides["syntax-punctuation"]) ?? ink,
    markdownHeading: optionalHex(overrides["markdown-heading"]) ?? accent,
    markdownLink: optionalHex(overrides["markdown-link"]) ?? primary,
    markdownLinkText: optionalHex(overrides["markdown-link-text"]) ?? info,
    markdownCode: optionalHex(overrides["markdown-code"]) ?? success,
    markdownBlockQuote: optionalHex(overrides["markdown-block-quote"]) ?? textMuted,
    markdownEmph: optionalHex(overrides["markdown-emph"]) ?? warning,
    markdownStrong: optionalHex(overrides["markdown-strong"]) ?? accent,
    markdownListItem: optionalHex(overrides["markdown-list-item"]) ?? primary,
  })
}

function fromOpenCodeTuiTheme(
  id: ThemeName,
  name: string,
  source: {
    defs: Record<string, unknown>
    theme: Record<string, unknown>
  },
): ThemeDefinition {
  return defineTheme(id, name, mergeThemeColors(Object.fromEntries(
    THEME_TOKEN_KEYS.flatMap((key) => {
      const color = resolveOpenCodeTuiColor(source.theme[key], source, `theme.${key}`)
      return color ? [[key, color]] : []
    }),
  ) as PartialThemeTokenColors))
}

function nativePaletteColors(palette: Record<string, unknown>): PartialThemeTokenColors {
  return pickThemeColors(palette, [
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
  ])
}

function nativeSyntaxColors(syntax: Record<string, unknown>): PartialThemeTokenColors {
  return {
    ...themeColor("syntaxComment", syntax.comment),
    ...themeColor("syntaxKeyword", syntax.keyword),
    ...themeColor("syntaxFunction", syntax.function),
    ...themeColor("syntaxVariable", syntax.variable),
    ...themeColor("syntaxString", syntax.string),
    ...themeColor("syntaxNumber", syntax.number),
    ...themeColor("syntaxType", syntax.type),
    ...themeColor("syntaxOperator", syntax.operator),
    ...themeColor("syntaxPunctuation", syntax.punctuation),
  }
}

function nativeMarkdownColors(markdown: Record<string, unknown>): PartialThemeTokenColors {
  return {
    ...themeColor("markdownHeading", markdown.heading),
    ...themeColor("markdownLink", markdown.link),
    ...themeColor("markdownLinkText", markdown.linkText),
    ...themeColor("markdownCode", markdown.code),
    ...themeColor("markdownBlockQuote", markdown.blockQuote),
    ...themeColor("markdownEmph", markdown.emph),
    ...themeColor("markdownStrong", markdown.strong),
    ...themeColor("markdownListItem", markdown.listItem),
  }
}

function pickThemeColors(source: Record<string, unknown>, keys: ThemeTokenKey[]): PartialThemeTokenColors {
  return Object.fromEntries(
    keys.flatMap((key) => {
      const value = optionalHex(source[key])
      return value ? [[key, value]] : []
    }),
  ) as PartialThemeTokenColors
}

function themeColor(key: ThemeTokenKey, value: unknown): PartialThemeTokenColors {
  const color = optionalHex(value)
  return color ? { [key]: color } : {}
}

function mergeThemeColors(partial: PartialThemeTokenColors): ThemeTokenColors {
  return validateThemeColors({
    ...themeDefinition(DEFAULT_THEME_ID).colors,
    ...partial,
  })
}

function validateThemeColors(colors: Record<ThemeTokenKey, string>): ThemeTokenColors {
  for (const key of THEME_TOKEN_KEYS) {
    colors[key] = hex(colors[key], key)
  }
  return colors
}

function parseThemeId(value: unknown, filePath = "<theme>") {
  if (typeof value !== "string") {
    const fromPath = themeIdFromPath(filePath)
    if (fromPath) {
      return fromPath
    }
    throw new Error("theme id must be a string")
  }
  const id = value.trim()
  if (!/^[A-Za-z0-9._-]+$/.test(id)) {
    throw new Error("theme id may only contain letters, numbers, dots, underscores, and dashes")
  }
  return id
}

function themeIdFromPath(filePath: string) {
  const basename = path.basename(filePath, ".json")
  return basename && basename !== "<theme>" && /^[A-Za-z0-9._-]+$/.test(basename)
    ? basename
    : null
}

function titleFromThemeId(id: string) {
  return id
    .split(/[._-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ") || id
}

function optionalHex(value: unknown) {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value.trim())
    ? value.trim().toLowerCase()
    : null
}

function hex(value: unknown, field: string) {
  const color = optionalHex(value)
  if (!color) {
    throw new Error(`${field} must be a #rrggbb color`)
  }
  return color
}

function mixHex(a: string, b: string, amount: number) {
  const left = parseRgb(a)
  const right = parseRgb(b)
  const mixed = left.map((channel, index) => Math.round(channel + (right[index]! - channel) * amount))
  return `#${mixed.map((channel) => channel.toString(16).padStart(2, "0")).join("")}`
}

function isDarkHex(value: string) {
  const [red, green, blue] = parseRgb(value)
  return (red * 0.299 + green * 0.587 + blue * 0.114) < 128
}

function resolveOpenCodeTuiColor(
  value: unknown,
  source: {
    defs: Record<string, unknown>
    theme: Record<string, unknown>
  },
  field: string,
  seen = new Set<string>(),
): string | null {
  if (value == null) {
    return null
  }
  if (typeof value === "number") {
    return ansiToHex(value)
  }
  if (typeof value === "string") {
    const color = optionalHex(value)
    if (color) {
      return color
    }
    if (value === "transparent" || value === "none") {
      return "#000000"
    }
    if (seen.has(value)) {
      throw new Error(`${field} contains a circular color reference '${value}'`)
    }
    seen.add(value)
    if (source.defs[value] != null) {
      return resolveOpenCodeTuiColor(source.defs[value], source, `${field}.${value}`, seen)
    }
    if (source.theme[value] != null) {
      return resolveOpenCodeTuiColor(source.theme[value], source, `${field}.${value}`, seen)
    }
    throw new Error(`${field} references unknown color '${value}'`)
  }
  if (isRecord(value)) {
    if (value.dark != null) {
      return resolveOpenCodeTuiColor(value.dark, source, `${field}.dark`, seen)
    }
    if (value.light != null) {
      return resolveOpenCodeTuiColor(value.light, source, `${field}.light`, seen)
    }
  }
  throw new Error(`${field} must resolve to a #rrggbb color`)
}

function ansiToHex(code: number) {
  if (!Number.isInteger(code) || code < 0 || code > 255) {
    return "#000000"
  }
  if (code < 16) {
    return [
      "#000000",
      "#800000",
      "#008000",
      "#808000",
      "#000080",
      "#800080",
      "#008080",
      "#c0c0c0",
      "#808080",
      "#ff0000",
      "#00ff00",
      "#ffff00",
      "#0000ff",
      "#ff00ff",
      "#00ffff",
      "#ffffff",
    ][code] ?? "#000000"
  }
  if (code < 232) {
    const index = code - 16
    const blue = index % 6
    const green = Math.floor(index / 6) % 6
    const red = Math.floor(index / 36)
    const value = (channel: number) => channel === 0 ? 0 : channel * 40 + 55
    return rgbToHex(value(red), value(green), value(blue))
  }
  const gray = (code - 232) * 10 + 8
  return rgbToHex(gray, gray, gray)
}

function rgbToHex(red: number, green: number, blue: number) {
  return `#${[red, green, blue].map((channel) => channel.toString(16).padStart(2, "0")).join("")}`
}

function parseRgb(value: string): [number, number, number] {
  const normalized = value.replace("#", "")
  return [
    Number.parseInt(normalized.slice(0, 2), 16),
    Number.parseInt(normalized.slice(2, 4), 16),
    Number.parseInt(normalized.slice(4, 6), 16),
  ]
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value)
}
