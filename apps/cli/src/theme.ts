import { RGBA } from "@opentui/core"

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

export const PromptBorderChars = {
  ...EmptyBorder,
  vertical: "┃",
  bottomLeft: "╹",
}
