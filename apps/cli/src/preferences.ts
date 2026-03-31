import os from "node:os"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"

export type ArrobaPreferences = {
  providers?: Record<string, ProviderPreferences>
  ui?: UiPreferences
}

export type ProviderPreferences = {
  model?: string
  effort?: string
}

export type MultiAgentResponseLayout = "individual" | "split"

export type UiPreferences = {
  multiAgentResponseLayout?: MultiAgentResponseLayout
}

export async function loadPreferences() {
  try {
    return JSON.parse(await readFile(preferencesPath(), "utf8")) as ArrobaPreferences
  } catch {
    return {} as ArrobaPreferences
  }
}

export async function saveProviderPreferences(provider: string, next: ProviderPreferences) {
  const current = await loadPreferences()
  await savePreferences({
    providers: {
      ...(current.providers ?? {}),
      [provider]: {
        ...(current.providers?.[provider] ?? {}),
        ...next,
      },
    },
  })
}

export async function saveUiPreferences(next: UiPreferences) {
  const current = await loadPreferences()
  await savePreferences({
    ui: {
      ...(current.ui ?? {}),
      ...next,
    },
  })
}

export function preferencesPath() {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) {
    return path.join(xdg, "arroba", "config.json")
  }
  return path.join(os.homedir(), ".arroba", "config.json")
}

async function savePreferences(next: ArrobaPreferences) {
  const filePath = preferencesPath()
  const current = await loadPreferences()
  await mkdir(path.dirname(filePath), { recursive: true })
  await writeFile(
    filePath,
    JSON.stringify(
      {
        ...current,
        ...next,
      } satisfies ArrobaPreferences,
      null,
      2,
    ),
  )
}
