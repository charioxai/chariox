import os from "node:os"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"

export type ArrobaPreferences = {
  providers?: Record<string, ProviderPreferences>
  ui?: UiPreferences
  sessions?: Record<string, SessionPreferences>
}

export type ProviderPreferences = {
  model?: string
  effort?: string
}

export type MultiAgentResponseLayout = "individual" | "split"

export type UiPreferences = {
  multiAgentResponseLayout?: MultiAgentResponseLayout
  maxAgentsPerScreen?: number
}

export type SessionPreferences = {
  promptHistory?: string[]
}

export const DEFAULT_MAX_AGENTS_PER_SCREEN = 6

export function resolveMaxAgentsPerScreen(value?: number | null) {
  if (!Number.isFinite(value)) {
    return DEFAULT_MAX_AGENTS_PER_SCREEN
  }
  return Math.max(1, Math.floor(Number(value)))
}

export function mergeUiPreferences(
  current: ArrobaPreferences,
  next: UiPreferences,
): ArrobaPreferences {
  return {
    ...current,
    ui: {
      ...(current.ui ?? {}),
      ...next,
    },
  }
}

export function mergeSessionPromptHistory(
  current: ArrobaPreferences,
  sessionId: string,
  entries: readonly string[],
): ArrobaPreferences {
  return {
    ...current,
    sessions: {
      ...(current.sessions ?? {}),
      [sessionId]: {
        ...(current.sessions?.[sessionId] ?? {}),
        promptHistory: normalizePromptHistoryEntries(entries),
      },
    },
  }
}

export function sessionPromptHistoryEntries(
  current: ArrobaPreferences,
  sessionId: string,
): string[] {
  return normalizePromptHistoryEntries(current.sessions?.[sessionId]?.promptHistory ?? [])
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
  await savePreferences(mergeUiPreferences(current, next))
}

export async function saveSessionPromptHistory(sessionId: string, entries: readonly string[]) {
  const current = await loadPreferences()
  await savePreferences(mergeSessionPromptHistory(current, sessionId, entries))
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

function normalizePromptHistoryEntries(entries: readonly string[]) {
  return entries
    .filter((entry): entry is string => typeof entry === "string")
    .map((entry) => entry.trimEnd())
    .filter((entry) => entry.length > 0)
}
