import type { TranscriptEntry } from "./cli-types.js"
import { reindexTranscriptEntries, transcriptRetentionSlice } from "@chariox/kernel-client/transcript-entry-state"

export function roomActivityNoticeKey(
  sessionId: string, environmentId: string, source: "state" | "resync" | "events", cursor: number, index: number,
): string {
  return `room-environment:${encodeURIComponent(sessionId)}:${encodeURIComponent(environmentId)}:${source}:${cursor}:${index}`
}

// MP-08/MP-10: Room events and explicit wake inspections project kernel state outside
// provider history. Retain that bounded view while history replaces a pane.
export function retainKernelProjectionNotices(
  refreshed: TranscriptEntry[], current: readonly TranscriptEntry[], sessionId: string,
): TranscriptEntry[] {
  const namespaces = ["room-environment:", "agent-wakes:"]
  const prefixes = namespaces.map(namespace => `${namespace}${encodeURIComponent(sessionId)}:`)
  const isKernelNotice = (entry: TranscriptEntry) => entry.role === "notice"
    && namespaces.some(namespace => entry.mergeKey?.startsWith(namespace))
  const notices = new Map<string, TranscriptEntry>()
  for (const entry of [...refreshed, ...current]) {
    const key = entry.mergeKey
    if (!isKernelNotice(entry) || !key || !prefixes.some(prefix => key.startsWith(prefix)) || entry.text.length > 64 * 1024) continue
    const notice = { ...entry, turnTracking: "none" as const }
    delete notice.turnId
    notices.set(key, notice)
  }
  const retained = transcriptRetentionSlice([...notices.values()], { maxEntries: 128, maxChars: 64 * 1024 }).kept
  return reindexTranscriptEntries([...refreshed.filter((entry) => !isKernelNotice(entry)), ...retained], 0)
}
