import os from "node:os"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"

export type ArrobaPreferences = {
  providers?: Record<string, ProviderPreferences>
}

export type ProviderPreferences = {
  model?: string
  effort?: string
}

export async function loadPreferences() {
  try {
    return JSON.parse(await readFile(preferencesPath(), "utf8")) as ArrobaPreferences
  } catch {
    return {} as ArrobaPreferences
  }
}

export async function saveProviderPreferences(provider: string, next: ProviderPreferences) {
  const filePath = preferencesPath()
  const current = await loadPreferences()
  await mkdir(path.dirname(filePath), { recursive: true })
  await writeFile(
    filePath,
    JSON.stringify(
      {
        providers: {
          ...(current.providers ?? {}),
          [provider]: {
            ...(current.providers?.[provider] ?? {}),
            ...next,
          },
        },
      } satisfies ArrobaPreferences,
      null,
      2,
    ),
  )
}

export function preferencesPath() {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) {
    return path.join(xdg, "arroba", "config.json")
  }
  return path.join(os.homedir(), ".arroba", "config.json")
}
