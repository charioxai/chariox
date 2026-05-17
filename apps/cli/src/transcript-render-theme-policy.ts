import type { TranscriptEntry } from "./cli-types.js"

export type TranscriptSurfaceTone = "default" | "focused" | "faded"

export type TranscriptRenderThemeEntry = Pick<TranscriptEntry, "role" | "emphasis">

export type TranscriptThemeColorToken =
  | "primary"
  | "secondary"
  | "accent"
  | "error"
  | "warning"
  | "info"
  | "text"
  | "textMuted"
  | "borderSubtle"

export type TranscriptSurfaceColorToken =
  | "backgroundPanel"
  | "backgroundElement"
  | "fadedPanel"
  | "fadedElement"

export type TranscriptSurfacePaletteTokens = {
  panel: TranscriptSurfaceColorToken
  element: TranscriptSurfaceColorToken
}

export function resolveTranscriptSurfaceTone(splitActive: boolean, focused: boolean): TranscriptSurfaceTone {
  if (!splitActive) {
    return "default"
  }
  return focused ? "focused" : "faded"
}

export function transcriptSurfacePaletteTokens(surfaceTone: TranscriptSurfaceTone): TranscriptSurfacePaletteTokens {
  if (surfaceTone === "faded") {
    return {
      panel: "fadedPanel",
      element: "fadedElement",
    }
  }
  return {
    panel: "backgroundPanel",
    element: "backgroundElement",
  }
}

export function transcriptAccentToken(entry: TranscriptRenderThemeEntry): TranscriptThemeColorToken {
  if (entry.role === "user") {
    return "primary"
  }
  if (entry.role === "reasoning") {
    return "accent"
  }
  if (entry.role === "tool") {
    return "secondary"
  }
  if (entry.role === "error") {
    return "error"
  }
  if (entry.role === "status") {
    return "info"
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? "error"
      : entry.emphasis === "warning"
        ? "warning"
        : "textMuted"
  }
  if (entry.role === "turn_toggle") {
    return "info"
  }
  return "borderSubtle"
}

export function transcriptUsesSeparator(entry: TranscriptRenderThemeEntry) {
  return entry.role === "user"
}

export function transcriptBodySurface(entry: TranscriptRenderThemeEntry): keyof TranscriptSurfacePaletteTokens | null {
  if (entry.role === "status") {
    return null
  }
  if (entry.role === "error" || entry.role === "assistant" || entry.role === "reasoning") {
    return "panel"
  }
  return "element"
}

export function transcriptTextColorToken(entry: TranscriptRenderThemeEntry): TranscriptThemeColorToken {
  if (entry.role === "user") {
    return "text"
  }
  if (entry.role === "reasoning") {
    return "textMuted"
  }
  if (entry.role === "tool") {
    return "secondary"
  }
  if (entry.role === "error") {
    return "error"
  }
  if (entry.role === "status") {
    return "info"
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error"
      ? "error"
      : entry.emphasis === "warning"
        ? "warning"
        : "textMuted"
  }
  if (entry.role === "turn_toggle") {
    return "info"
  }
  return "text"
}

export function transcriptInlineCodeColorToken(entry: TranscriptRenderThemeEntry): TranscriptThemeColorToken {
  if (entry.role === "tool" || entry.role === "status" || entry.role === "error" || entry.role === "turn_toggle") {
    return "primary"
  }
  if (entry.role === "user") {
    return "text"
  }
  if (entry.role === "notice") {
    return entry.emphasis === "error" ? "warning" : "info"
  }
  return "info"
}
