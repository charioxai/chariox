import { RGBA, SyntaxStyle, type ThemeTokenStyle } from "@opentui/core"
import { normalizeThemeName, type ThemeName } from "./theme-registry.js"

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

const THEME_PALETTES: Record<ThemeName, ArrobaThemePalette> = {
  opencode: palette({
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
  tokyonight: palette({
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
  catppuccin: palette({
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
  vercel: palette({
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
  sober: palette({
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
}

let selectedThemeName: ThemeName = "opencode"

export const theme: ArrobaThemePalette = { ...THEME_PALETTES[selectedThemeName] }

export function currentThemeName() {
  return selectedThemeName
}

export function applyTheme(value: unknown) {
  const nextThemeName = normalizeThemeName(value)
  selectedThemeName = nextThemeName
  Object.assign(theme, THEME_PALETTES[nextThemeName])
  return nextThemeName
}

function palette(colors: Record<keyof ArrobaThemePalette, string>): ArrobaThemePalette {
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
