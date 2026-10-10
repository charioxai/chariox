import os from "node:os"
import path from "node:path"
import { mkdir, readFile, writeFile, rename, chmod } from "node:fs/promises"
import type { ThemeName } from "./theme-registry.js"

export type CharioxPreferences = {
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
  machineId?: string
  machineAlias?: string
  kernelId?: string
  kernelEnrolled?: boolean
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
  hiddenRemoteKernelIds?: string[]
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
  current: CharioxPreferences,
  next: UiPreferences,
): CharioxPreferences {
  return {
    ...current,
    ui: {
      ...(current.ui ?? {}),
      ...next,
    },
  }
}

export function mergeSessionPromptHistory(
  current: CharioxPreferences,
  sessionId: string,
  entries: readonly string[],
): CharioxPreferences {
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
  current: CharioxPreferences,
  sessionId: string,
  next: {
    promptHistory?: readonly string[]
    promptDraft?: string | null
  },
): CharioxPreferences {
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
  current: CharioxPreferences,
  sessionId: string,
): string[] {
  return normalizePromptHistoryEntries(current.sessions?.[sessionId]?.promptHistory ?? [])
}

export function sessionPromptDraftEntry(
  current: CharioxPreferences,
  sessionId: string,
): string {
  return normalizePromptDraftEntry(current.sessions?.[sessionId]?.promptDraft) ?? ""
}

export async function loadPreferences() {
  try {
    const filePath = preferencesPath()
    const parsed = JSON.parse(await readFile(filePath, "utf8")) as CharioxPreferences
    const safe = sanitizePreferences(parsed)
    if (JSON.stringify(parsed) !== JSON.stringify(safe)) await writePreferences(filePath, safe)
    await chmod(filePath, 0o600)
    return safe
  } catch {
    return {} as CharioxPreferences
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
  current: CharioxPreferences,
  profile: RelayCloudProfile | null,
): CharioxPreferences {
  return {
    ...current,
    relay: {
      ...(current.relay ?? {}),
      cloud: publicRelayCloudProfile(profile),
    },
  }
}

export function relayCloudProfile(
  current: CharioxPreferences,
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
  const explicitHome = process.env.CHARIOX_HOME?.trim()
  if (explicitHome) return path.join(explicitHome, "config.json")
  const xdg = process.env.XDG_CONFIG_HOME?.trim()
  if (xdg) {
    return path.join(xdg, "chariox", "config.json")
  }
  return path.join(os.homedir(), ".chariox", "config.json")
}

async function updatePreferences(
  apply: (current: CharioxPreferences) => CharioxPreferences,
) {
  preferencesSaveQueue = preferencesSaveQueue
    .catch(() => {
      // Preserve the queue after prior save failures.
    })
    .then(async () => {
      const filePath = preferencesPath()
      const current = await loadPreferences()
      const next = apply(current)
      await writePreferences(filePath, sanitizePreferences(next))
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

export function publicRelayCloudProfile(profile: RelayCloudProfile | null | undefined): RelayCloudProfile | null {
  if (!profile) return null
  const keys = ["apiUrl", "email", "accountId", "userId", "accountSlug", "realmId", "relayUrl", "issuerId", "clientId", "clientAlias", "machineId", "machineAlias", "kernelId", "kernelEnrolled", "tokenExpiresAtMs"] as const
  return Object.fromEntries(keys.filter(key => profile[key] !== undefined).map(key => [key, profile[key]])) as RelayCloudProfile
}
function sanitizePreferences(current: CharioxPreferences): CharioxPreferences {
  return current.relay ? { ...current, relay: { ...current.relay, cloud: publicRelayCloudProfile(current.relay.cloud) } } : current
}
async function writePreferences(filePath: string, preferences: CharioxPreferences) {
  await mkdir(path.dirname(filePath), { recursive: true, mode: 0o700 })
  const temporary = `${filePath}.${process.pid}.${Date.now()}.tmp`
  try {
    await writeFile(temporary, JSON.stringify(preferences, null, 2), { mode: 0o600, flag: "wx" })
    await rename(temporary, filePath)
  } finally {
    const { rm } = await import("node:fs/promises")
    await rm(temporary, { force: true })
  }
}
