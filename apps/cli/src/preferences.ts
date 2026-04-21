import os from "node:os"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import type { ThemeName } from "./theme-registry.js"

export type ArrobaPreferences = {
  providers?: Record<string, ProviderPreferences>
  relay?: RelayPreferences
  ui?: UiPreferences
  sessions?: Record<string, SessionPreferences>
}

export type RelayPreferences = {
  cloud?: RelayCloudProfile | null
}

export type RelayCloudProfile = {
  apiUrl: string
  email: string
  accountId: string
  userId: string
  accountSlug: string
  realmId: string
  relayUrl: string
  issuerId: string
  clientId?: string
  clientAlias?: string
  tokenExpiresAtMs?: number
}

export type ProviderPreferences = {
  model?: string
  effort?: string
}

export type MultiAgentResponseLayout = "individual" | "split"

export type UiPreferences = {
  multiAgentResponseLayout?: MultiAgentResponseLayout
  maxAgentsPerScreen?: number
  theme?: ThemeName
}

export type SessionPreferences = {
  promptHistory?: string[]
  promptDraft?: string
}

export const DEFAULT_MAX_AGENTS_PER_SCREEN = 6
let preferencesSaveQueue: Promise<void> = Promise.resolve()

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

export function mergeSessionPromptState(
  current: ArrobaPreferences,
  sessionId: string,
  next: {
    promptHistory?: readonly string[]
    promptDraft?: string | null
  },
): ArrobaPreferences {
  const promptDraft = normalizePromptDraftEntry(next.promptDraft)
  return {
    ...current,
    sessions: {
      ...(current.sessions ?? {}),
      [sessionId]: {
        ...(current.sessions?.[sessionId] ?? {}),
        ...(next.promptHistory ? { promptHistory: normalizePromptHistoryEntries(next.promptHistory) } : {}),
        ...(next.promptDraft !== undefined ? { promptDraft: promptDraft ?? "" } : {}),
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

export function sessionPromptDraftEntry(
  current: ArrobaPreferences,
  sessionId: string,
): string {
  return normalizePromptDraftEntry(current.sessions?.[sessionId]?.promptDraft) ?? ""
}

export async function loadPreferences() {
  try {
    return JSON.parse(await readFile(preferencesPath(), "utf8")) as ArrobaPreferences
  } catch {
    return {} as ArrobaPreferences
  }
}

export async function saveProviderPreferences(provider: string, next: ProviderPreferences) {
  await updatePreferences((current) => ({
    ...current,
    providers: {
      ...(current.providers ?? {}),
      [provider]: {
        ...(current.providers?.[provider] ?? {}),
        ...next,
      },
    },
  }))
}

export async function saveUiPreferences(next: UiPreferences) {
  await updatePreferences((current) => mergeUiPreferences(current, next))
}

export function mergeRelayCloudProfile(
  current: ArrobaPreferences,
  profile: RelayCloudProfile | null,
): ArrobaPreferences {
  return {
    ...current,
    relay: {
      ...(current.relay ?? {}),
      cloud: profile,
    },
  }
}

export function relayCloudProfile(
  current: ArrobaPreferences,
): RelayCloudProfile | null {
  return current.relay?.cloud ?? null
}

export async function saveRelayCloudProfile(profile: RelayCloudProfile | null) {
  await updatePreferences((current) => mergeRelayCloudProfile(current, profile))
}

export async function saveSessionPromptHistory(sessionId: string, entries: readonly string[]) {
  await updatePreferences((current) => mergeSessionPromptHistory(current, sessionId, entries))
}

export async function saveSessionPromptState(
  sessionId: string,
  next: {
    promptHistory?: readonly string[]
    promptDraft?: string | null
  },
) {
  await updatePreferences((current) => mergeSessionPromptState(current, sessionId, next))
}

export function preferencesPath() {
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) {
    return path.join(xdg, "arroba", "config.json")
  }
  return path.join(os.homedir(), ".arroba", "config.json")
}

async function updatePreferences(
  apply: (current: ArrobaPreferences) => ArrobaPreferences,
) {
  preferencesSaveQueue = preferencesSaveQueue
    .catch(() => {
      // Preserve the queue after prior save failures.
    })
    .then(async () => {
      const filePath = preferencesPath()
      const current = await loadPreferences()
      const next = apply(current)
      await mkdir(path.dirname(filePath), { recursive: true })
      await writeFile(filePath, JSON.stringify(next, null, 2))
    })
  await preferencesSaveQueue
}

function normalizePromptHistoryEntries(entries: readonly string[]) {
  return entries
    .filter((entry): entry is string => typeof entry === "string")
    .map((entry) => entry.trimEnd())
    .filter((entry) => entry.length > 0)
}

function normalizePromptDraftEntry(entry: unknown) {
  if (typeof entry !== "string") {
    return null
  }
  return entry
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
}
