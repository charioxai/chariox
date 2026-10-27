import path from "node:path"
import {
  ansiToHex,
  hex,
  isDarkHex,
  mixHex,
  optionalHex,
} from "./theme-color-utils.js"
import {
  THEME_TOKEN_KEYS,
  type ThemeDefinition,
  type ThemeName,
  type ThemeTokenColors,
  type ThemeTokenKey,
} from "./theme-contracts.js"

type PartialThemeTokenColors = Partial<Record<ThemeTokenKey, string>>

export function parseThemeDefinition(
  raw: unknown,
  fallbackColors: ThemeTokenColors,
  filePath = "<theme>",
): ThemeDefinition {
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
      }, fallbackColors),
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
    }, fallbackColors),
  }
}

export function defineTheme(id: ThemeName, name: string, colors: ThemeTokenColors): ThemeDefinition {
  return {
    id,
    name,
    source: "builtin",
    colors: validateThemeColors(colors),
  }
}

export function fromOpenCodeDark(
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
  fallbackColors: ThemeTokenColors,
): ThemeDefinition {
  return defineTheme(id, name, mergeThemeColors(Object.fromEntries(
    THEME_TOKEN_KEYS.flatMap((key) => {
      const color = resolveOpenCodeTuiColor(source.theme[key], source, `theme.${key}`)
      return color ? [[key, color]] : []
    }),
  ) as PartialThemeTokenColors, fallbackColors))
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

function mergeThemeColors(
  partial: PartialThemeTokenColors,
  fallbackColors: ThemeTokenColors,
): ThemeTokenColors {
  return validateThemeColors({
    ...fallbackColors,
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value)
}
