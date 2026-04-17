import { RGBA, SyntaxStyle, type ThemeTokenStyle } from "@opentui/core"
import {
  DEFAULT_THEME_REGISTRY,
  normalizeThemeName,
  themeDefinition,
  type ThemeName,
  type ThemeRegistry,
  type ThemeTokenColors,
} from "./theme-registry.js"

export type ArrobaThemePalette = {
  primary: RGBA
  secondary: RGBA
  accent: RGBA
  error: RGBA
  warning: RGBA
  success: RGBA
  info: RGBA
  text: RGBA
  textMuted: RGBA
  background: RGBA
  backgroundPanel: RGBA
  backgroundElement: RGBA
  border: RGBA
  borderActive: RGBA
  borderSubtle: RGBA
  syntaxComment: RGBA
  syntaxKeyword: RGBA
  syntaxFunction: RGBA
  syntaxVariable: RGBA
  syntaxString: RGBA
  syntaxNumber: RGBA
  syntaxType: RGBA
  syntaxOperator: RGBA
  syntaxPunctuation: RGBA
  markdownHeading: RGBA
  markdownLink: RGBA
  markdownLinkText: RGBA
  markdownCode: RGBA
  markdownBlockQuote: RGBA
  markdownEmph: RGBA
  markdownStrong: RGBA
  markdownListItem: RGBA
}

let selectedThemeName: ThemeName = "opencode"
let currentThemeRegistry: ThemeRegistry = DEFAULT_THEME_REGISTRY

export const theme: ArrobaThemePalette = palette(themeDefinition(selectedThemeName).colors)

export function currentThemeName() {
  return selectedThemeName
}

export function setThemeRegistry(registry: ThemeRegistry) {
  currentThemeRegistry = registry
  return applyTheme(selectedThemeName, registry)
}

export function applyTheme(value: unknown, registry: ThemeRegistry = currentThemeRegistry) {
  const nextThemeName = normalizeThemeName(value, registry)
  selectedThemeName = nextThemeName
  Object.assign(theme, palette(themeDefinition(nextThemeName, registry).colors))
  return nextThemeName
}

function palette(colors: ThemeTokenColors): ArrobaThemePalette {
  return Object.fromEntries(
    Object.entries(colors).map(([key, value]) => [key, RGBA.fromHex(value)]),
  ) as ArrobaThemePalette
}

export const EmptyBorder = {
  topLeft: "",
  bottomLeft: "",
  vertical: "",
  topRight: "",
  bottomRight: "",
  horizontal: " ",
  bottomT: "",
  topT: "",
  cross: "",
  leftT: "",
  rightT: "",
}

export const SplitBorder = {
  border: ["left", "right"] as const,
  customBorderChars: {
    ...EmptyBorder,
    vertical: "┃",
  },
}

export const PaneGridBorderChars = {
  ...EmptyBorder,
  vertical: "│",
  horizontal: "─",
}

export const TranscriptSeparatorBorder = {
  customBorderChars: {
    ...EmptyBorder,
    horizontal: "─",
  },
}

export const PromptBorderChars = {
  ...EmptyBorder,
  vertical: "┃",
  horizontal: "━",
  bottomLeft: "╹",
}

export function createTranscriptSyntaxStyle() {
  const rules: ThemeTokenStyle[] = [
    {
      scope: ["default"],
      style: {
        foreground: theme.text,
      },
    },
    {
      scope: ["comment", "comment.documentation"],
      style: {
        foreground: theme.syntaxComment,
        italic: true,
      },
    },
    {
      scope: ["string", "string.escape", "symbol", "character", "character.special"],
      style: {
        foreground: theme.syntaxString,
      },
    },
    {
      scope: ["number", "float", "boolean", "constant"],
      style: {
        foreground: theme.syntaxNumber,
      },
    },
    {
      scope: ["keyword", "keyword.return", "keyword.conditional", "keyword.repeat", "keyword.import", "keyword.modifier", "keyword.exception"],
      style: {
        foreground: theme.syntaxKeyword,
        italic: true,
      },
    },
    {
      scope: ["keyword.function", "function", "function.call", "function.method", "function.method.call", "constructor"],
      style: {
        foreground: theme.syntaxFunction,
      },
    },
    {
      scope: ["variable", "variable.parameter", "parameter", "property", "field"],
      style: {
        foreground: theme.syntaxVariable,
      },
    },
    {
      scope: ["type", "type.definition", "class", "namespace", "module"],
      style: {
        foreground: theme.syntaxType,
      },
    },
    {
      scope: ["operator", "keyword.operator", "punctuation.delimiter", "punctuation.special"],
      style: {
        foreground: theme.syntaxOperator,
      },
    },
    {
      scope: ["punctuation", "punctuation.bracket", "conceal"],
      style: {
        foreground: theme.syntaxPunctuation,
      },
    },
    {
      scope: ["markup.heading", "markup.heading.1", "markup.heading.2", "markup.heading.3", "markup.heading.4", "markup.heading.5", "markup.heading.6"],
      style: {
        foreground: theme.markdownHeading,
        bold: true,
      },
    },
    {
      scope: ["markup.bold", "markup.strong"],
      style: {
        foreground: theme.markdownStrong,
        bold: true,
      },
    },
    {
      scope: ["markup.italic"],
      style: {
        foreground: theme.markdownEmph,
        italic: true,
      },
    },
    {
      scope: ["markup.list"],
      style: {
        foreground: theme.markdownListItem,
      },
    },
    {
      scope: ["markup.quote"],
      style: {
        foreground: theme.markdownBlockQuote,
        italic: true,
      },
    },
    {
      scope: ["markup.raw", "markup.raw.block", "markup.raw.inline"],
      style: {
        foreground: theme.markdownCode,
      },
    },
    {
      scope: ["markup.link", "markup.link.url", "string.special.url"],
      style: {
        foreground: theme.markdownLink,
        underline: true,
      },
    },
    {
      scope: ["markup.link.label", "label"],
      style: {
        foreground: theme.markdownLinkText,
        underline: true,
      },
    },
  ]

  return SyntaxStyle.fromTheme(rules)
}
