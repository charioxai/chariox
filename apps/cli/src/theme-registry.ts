import { readFile } from "node:fs/promises"
import {
  DEFAULT_THEME_ID,
  type ThemeDefinition,
  type ThemeName,
  type ThemeRegistry,
} from "./theme-contracts.js"
import { parseThemeDefinition as parseRawThemeDefinition } from "./theme-definition-parser.js"
import { BUILTIN_THEME_DEFINITIONS } from "./theme-builtins.js"
import {
  themeDirectories,
  themeFiles,
  type ThemeLoadOptions,
  type ThemeLoadWarning,
} from "./theme-file-source.js"

export {
  DEFAULT_THEME_ID,
  THEME_NAMES,
  THEME_TOKEN_KEYS,
  type ThemeDefinition,
  type ThemeName,
  type ThemeRegistry,
  type ThemeTokenColors,
  type ThemeTokenKey,
} from "./theme-contracts.js"

export {
  themeDirectories,
  type ThemeLoadWarning,
} from "./theme-file-source.js"

const BUILTIN_THEME_IDS = new Set(BUILTIN_THEME_DEFINITIONS.map((theme) => theme.id))

export const DEFAULT_THEME_REGISTRY = createThemeRegistry(BUILTIN_THEME_DEFINITIONS)

export function normalizeThemeName(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY): ThemeName {
  return typeof value === "string" && registry.themes.has(value)
    ? value
    : DEFAULT_THEME_ID
}

export function themeLabel(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return themeDefinition(value, registry).name
}

export function themeOptions(registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return registry.orderedIds.map((id) => ({
    id,
    label: registry.themes.get(id)?.name ?? id,
  }))
}

export function themeDefinition(value: unknown, registry: ThemeRegistry = DEFAULT_THEME_REGISTRY) {
  return registry.themes.get(normalizeThemeName(value, registry)) ?? registry.themes.get(DEFAULT_THEME_ID)!
}

export async function loadThemeRegistry(options: ThemeLoadOptions = {}): Promise<ThemeRegistry> {
  const directories = options.directories ?? themeDirectories(options.workspace)
  const customThemes: ThemeDefinition[] = []
  for (const directory of directories) {
    const files = await themeFiles(directory)
    for (const filePath of files) {
      try {
        const raw = JSON.parse(await readFile(filePath, "utf8")) as unknown
        const theme = parseThemeDefinition(raw, filePath)
        if (BUILTIN_THEME_IDS.has(theme.id)) {
          options.onWarning?.({ path: filePath, message: `custom theme id '${theme.id}' conflicts with a built-in theme` })
          continue
        }
        customThemes.push(theme)
      } catch (error) {
        options.onWarning?.({ path: filePath, message: error instanceof Error ? error.message : String(error) })
      }
    }
  }
  return createThemeRegistry([...BUILTIN_THEME_DEFINITIONS, ...customThemes])
}

export function parseThemeDefinition(raw: unknown, filePath = "<theme>"): ThemeDefinition {
  return parseRawThemeDefinition(raw, themeDefinition(DEFAULT_THEME_ID).colors, filePath)
}

function createThemeRegistry(themes: ThemeDefinition[]): ThemeRegistry {
  const map = new Map<ThemeName, ThemeDefinition>()
  const orderedIds: ThemeName[] = []
  for (const theme of themes) {
    if (!map.has(theme.id)) {
      orderedIds.push(theme.id)
    }
    map.set(theme.id, theme)
  }
  return { themes: map, orderedIds }
}
