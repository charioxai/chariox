import { RGBA } from "@opentui/core"

import type { TranscriptEntry } from "./cli-types.js"
import { theme } from "./theme.js"
import {
  transcriptAccentToken,
  transcriptBodySurface,
  transcriptInlineCodeColorToken,
  transcriptSurfacePaletteTokens,
  transcriptTextColorToken,
  type TranscriptSurfaceColorToken,
  type TranscriptSurfaceTone,
  type TranscriptThemeColorToken,
} from "./transcript-render-theme-policy.js"

export {
  resolveTranscriptSurfaceTone,
  transcriptUsesSeparator,
  type TranscriptSurfaceTone,
} from "./transcript-render-theme-policy.js"

export function transcriptSurfacePalette(surfaceTone: TranscriptSurfaceTone) {
  const palette = transcriptSurfacePaletteTokens(surfaceTone)
  return {
    panel: transcriptSurfaceColor(palette.panel),
    element: transcriptSurfaceColor(palette.element),
  }
}

export function transcriptAccent(entry: TranscriptEntry) {
  return transcriptThemeColor(transcriptAccentToken(entry))
}

export function transcriptBodyColor(entry: TranscriptEntry, surfaceTone: TranscriptSurfaceTone = "default") {
  const surface = transcriptBodySurface(entry)
  if (surface === null) {
    return null
  }
  return transcriptSurfacePalette(surfaceTone)[surface]
}

export function transcriptTextColor(entry: TranscriptEntry) {
  return transcriptThemeColor(transcriptTextColorToken(entry))
}

export function transcriptInlineCodeColor(entry: TranscriptEntry) {
  return transcriptThemeColor(transcriptInlineCodeColorToken(entry))
}

function transcriptThemeColor(token: TranscriptThemeColorToken) {
  return theme[token]
}

function transcriptSurfaceColor(token: TranscriptSurfaceColorToken) {
  if (token === "fadedPanel") {
    return RGBA.fromHex("#171717")
  }
  if (token === "fadedElement") {
    return RGBA.fromHex("#202020")
  }
  return theme[token]
}
