import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  resolveTranscriptSurfaceTone,
  transcriptAccentToken,
  transcriptBodySurface,
  transcriptInlineCodeColorToken,
  transcriptSurfacePaletteTokens,
  transcriptTextColorToken,
  transcriptUsesSeparator,
} from "./transcript-render-theme-policy.js"

function transcriptEntry(role: TranscriptEntry["role"], overrides: Partial<TranscriptEntry> = {}): TranscriptEntry {
  return {
    id: 1,
    role,
    text: "hello",
    ...overrides,
  } as TranscriptEntry
}

test("resolveTranscriptSurfaceTone follows split focus state", () => {
  assert.equal(resolveTranscriptSurfaceTone(false, false), "default")
  assert.equal(resolveTranscriptSurfaceTone(false, true), "default")
  assert.equal(resolveTranscriptSurfaceTone(true, true), "focused")
  assert.equal(resolveTranscriptSurfaceTone(true, false), "faded")
})

test("transcriptSurfacePalette uses focused/default theme colors and muted faded colors", () => {
  assert.deepEqual(transcriptSurfacePaletteTokens("default"), {
    panel: "backgroundPanel",
    element: "backgroundElement",
  })
  assert.deepEqual(transcriptSurfacePaletteTokens("focused"), {
    panel: "backgroundPanel",
    element: "backgroundElement",
  })
  assert.deepEqual(transcriptSurfacePaletteTokens("faded"), {
    panel: "fadedPanel",
    element: "fadedElement",
  })
})

test("transcript role chrome stays centralized in render theme policy", () => {
  assert.equal(transcriptUsesSeparator(transcriptEntry("user")), true)
  assert.equal(transcriptUsesSeparator(transcriptEntry("assistant")), false)
  assert.equal(transcriptAccentToken(transcriptEntry("user")), "primary")
  assert.equal(transcriptTextColorToken(transcriptEntry("reasoning")), "textMuted")
  assert.equal(transcriptInlineCodeColorToken(transcriptEntry("tool")), "primary")
  assert.equal(transcriptBodySurface(transcriptEntry("status")), null)
  assert.equal(transcriptBodySurface(transcriptEntry("assistant")), "panel")
  assert.equal(transcriptBodySurface(transcriptEntry("user")), "element")
})
