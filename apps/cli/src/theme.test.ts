import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import test from "node:test"

import {
  loadThemeRegistry,
  normalizeThemeName,
  parseThemeDefinition,
  themeLabel,
  themeOptions,
} from "./theme-registry.js"

test("theme registry normalizes and labels waiting room options", () => {
  assert.equal(normalizeThemeName("sober"), "sober")
  assert.equal(normalizeThemeName("missing"), "opencode")
  assert.equal(themeLabel("tokyonight"), "Tokyonight")
  assert.equal(themeOptions().some((option) => option.id === "sober"), true)
  assert.equal(themeOptions().some((option) => option.id === "matrix"), true)
})

test("theme registry parses native Arroba theme json", () => {
  const theme = parseThemeDefinition({
    id: "custom-gray",
    name: "Custom Gray",
    palette: {
      primary: "#eeeeee",
      background: "#101010",
      text: "#f0f0f0",
    },
    syntax: {
      keyword: "#dddddd",
    },
    markdown: {
      heading: "#ffffff",
    },
  })

  assert.equal(theme.id, "custom-gray")
  assert.equal(theme.name, "Custom Gray")
  assert.equal(theme.colors.primary, "#eeeeee")
  assert.equal(theme.colors.background, "#101010")
  assert.equal(theme.colors.syntaxKeyword, "#dddddd")
  assert.equal(theme.colors.markdownHeading, "#ffffff")
  assert.equal(theme.colors.secondary, "#5c9cf5")
})

test("theme registry parses OpenCode desktop theme json", () => {
  const theme = parseThemeDefinition({
    id: "custom-matrix",
    name: "Custom Matrix",
    dark: {
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
        "syntax-keyword": "#c770ff",
      },
    },
  })

  assert.equal(theme.id, "custom-matrix")
  assert.equal(theme.colors.background, "#000000")
  assert.equal(theme.colors.text, "#62ff94")
  assert.equal(theme.colors.primary, "#2eff6a")
  assert.equal(theme.colors.syntaxKeyword, "#c770ff")
})

test("theme registry parses OpenCode TUI theme json", () => {
  const theme = parseThemeDefinition({
    $schema: "https://opencode.ai/theme.json",
    defs: {
      bg: "#000000",
      ink: "#62ff94",
      green: "#2eff6a",
      dim: "#8ca391",
    },
    theme: {
      primary: { dark: "green", light: "green" },
      text: { dark: "ink", light: "bg" },
      textMuted: { dark: "dim", light: "dim" },
      background: { dark: "bg", light: "#ffffff" },
      syntaxKeyword: { dark: "#c770ff", light: "#c770ff" },
      syntaxFunction: 14,
      markdownHeading: "primary",
    },
  }, "/tmp/neon-matrix.json")

  assert.equal(theme.id, "neon-matrix")
  assert.equal(theme.name, "Neon Matrix")
  assert.equal(theme.colors.background, "#000000")
  assert.equal(theme.colors.text, "#62ff94")
  assert.equal(theme.colors.primary, "#2eff6a")
  assert.equal(theme.colors.syntaxKeyword, "#c770ff")
  assert.equal(theme.colors.syntaxFunction, "#00ffff")
  assert.equal(theme.colors.markdownHeading, "#2eff6a")
})

test("loadThemeRegistry loads global and workspace custom themes", async () => {
  const temp = await mkdtemp(path.join(os.tmpdir(), "arroba-themes-"))
  const globalThemes = path.join(temp, "global")
  const workspaceThemes = path.join(temp, "workspace")
  await mkdir(globalThemes, { recursive: true })
  await mkdir(workspaceThemes, { recursive: true })
  await writeFile(path.join(globalThemes, "global.json"), JSON.stringify({
    id: "global-theme",
    name: "Global Theme",
    palette: { primary: "#abcdef" },
  }))
  await writeFile(path.join(workspaceThemes, "workspace.json"), JSON.stringify({
    id: "workspace-theme",
    name: "Workspace Theme",
    palette: { primary: "#fedcba" },
  }))

  try {
    const registry = await loadThemeRegistry({ directories: [globalThemes, workspaceThemes] })
    assert.equal(normalizeThemeName("global-theme", registry), "global-theme")
    assert.equal(themeLabel("workspace-theme", registry), "Workspace Theme")
    assert.equal(themeOptions(registry).at(-2)?.id, "global-theme")
    assert.equal(themeOptions(registry).at(-1)?.id, "workspace-theme")
  } finally {
    await rm(temp, { recursive: true, force: true })
  }
})
