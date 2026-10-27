import assert from "node:assert/strict"
import test from "node:test"

import { parseThemeDefinition } from "./theme-definition-parser.js"
import { themeDefinition } from "./theme-registry.js"

const fallbackColors = themeDefinition("opencode").colors

test("theme definition parser derives id and name from file path", () => {
  const parsed = parseThemeDefinition({
    palette: {
      primary: "#112233",
      text: "#eeeeee",
      background: "#000000",
    },
  }, fallbackColors, "/themes/review-theme.json")

  assert.equal(parsed.id, "review-theme")
  assert.equal(parsed.name, "Review Theme")
  assert.equal(parsed.source, "user")
  assert.equal(parsed.colors.primary, "#112233")
  assert.equal(parsed.colors.secondary, fallbackColors.secondary)
})

test("theme definition parser resolves OpenCode TUI color references", () => {
  const parsed = parseThemeDefinition({
    id: "tui-theme",
    defs: { accent: "#abcdef" },
    theme: {
      primary: "accent",
      markdownHeading: { dark: "accent" },
    },
  }, fallbackColors)

  assert.equal(parsed.colors.primary, "#abcdef")
  assert.equal(parsed.colors.markdownHeading, "#abcdef")
  assert.equal(parsed.colors.text, fallbackColors.text)
})
