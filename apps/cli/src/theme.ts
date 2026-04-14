import { RGBA, SyntaxStyle, type ThemeTokenStyle } from "@opentui/core"

export const theme = {
  primary: RGBA.fromHex("#fab283"),
  secondary: RGBA.fromHex("#5c9cf5"),
  accent: RGBA.fromHex("#9d7cd8"),
  error: RGBA.fromHex("#e06c75"),
  warning: RGBA.fromHex("#f5a742"),
  success: RGBA.fromHex("#7fd88f"),
  info: RGBA.fromHex("#56b6c2"),
  text: RGBA.fromHex("#eeeeee"),
  textMuted: RGBA.fromHex("#808080"),
  background: RGBA.fromHex("#0a0a0a"),
  backgroundPanel: RGBA.fromHex("#141414"),
  backgroundElement: RGBA.fromHex("#1e1e1e"),
  border: RGBA.fromHex("#484848"),
  borderActive: RGBA.fromHex("#606060"),
  borderSubtle: RGBA.fromHex("#3c3c3c"),
  syntaxComment: RGBA.fromHex("#6e7681"),
  syntaxKeyword: RGBA.fromHex("#fab283"),
  syntaxFunction: RGBA.fromHex("#7fb4ff"),
  syntaxVariable: RGBA.fromHex("#eeeeee"),
  syntaxString: RGBA.fromHex("#7fd88f"),
  syntaxNumber: RGBA.fromHex("#e9b872"),
  syntaxType: RGBA.fromHex("#c3e88d"),
  syntaxOperator: RGBA.fromHex("#d4d4d4"),
  syntaxPunctuation: RGBA.fromHex("#8b949e"),
  markdownHeading: RGBA.fromHex("#f5c26b"),
  markdownLink: RGBA.fromHex("#7fb4ff"),
  markdownLinkText: RGBA.fromHex("#56b6c2"),
  markdownCode: RGBA.fromHex("#7fd88f"),
  markdownBlockQuote: RGBA.fromHex("#d3a55b"),
  markdownEmph: RGBA.fromHex("#d3a55b"),
  markdownStrong: RGBA.fromHex("#f5c26b"),
  markdownListItem: RGBA.fromHex("#5c9cf5"),
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

export const TranscriptSeparatorBorder = {
  customBorderChars: {
    ...EmptyBorder,
    horizontal: "─",
  },
}

export const PromptBorderChars = {
  ...EmptyBorder,
  vertical: "┃",
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
