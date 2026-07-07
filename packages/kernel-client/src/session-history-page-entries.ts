import type { SessionHistoryEntry, SessionHistoryPageEntry } from "./kernel-types.js"
import { mergeExternalProviderObservedHistoryFields } from "./external-provider-observation.js"
import {
  cloneSessionHistoryPromptAttachments,
  mergeSessionHistoryPromptAttachments,
} from "./session-history-attachments.js"
import { sessionHistoryFragmentsAreAdjacent } from "./session-history-fragments.js"

export function mergeAdjacentSessionHistoryPageEntries<T extends SessionHistoryPageEntry>(
  historyEntries: readonly T[],
): SessionHistoryPageEntry[] {
  const merged: SessionHistoryPageEntry[] = []

  for (const entry of historyEntries) {
    const previous = merged.at(-1)
    if (
      previous
      && sessionHistoryFragmentsAreAdjacent(previous, entry)
      && previous.entry.kind === entry.entry.kind
    ) {
      previous.fragment_end = entry.fragment_end
      previous.entry.text += entry.entry.text
      previous.total_chars = Math.max(previous.total_chars, entry.total_chars)
      if (entry.entry.attachments !== undefined) {
        previous.entry.attachments = mergeSessionHistoryPromptAttachments(previous.entry.attachments, entry.entry.attachments)
      }
      if (previous.entry.prompt_origin === undefined && entry.entry.prompt_origin !== undefined) {
        previous.entry.prompt_origin = entry.entry.prompt_origin
      }
      mergeExternalProviderObservedHistoryFields(previous.entry, entry.entry)
      continue
    }

    merged.push({
      entry_index: entry.entry_index,
      fragment_start: entry.fragment_start,
      fragment_end: entry.fragment_end,
      total_chars: entry.total_chars,
      entry: cloneSessionHistoryEntry(entry.entry),
    })
  }

  return merged
}

export function cloneSessionHistoryEntry(entry: SessionHistoryEntry): SessionHistoryEntry {
  return {
    kind: entry.kind,
    text: entry.text,
    ...(entry.agent_id !== undefined ? { agent_id: entry.agent_id } : {}),
    ...(entry.provider_run_id !== undefined ? { provider_run_id: entry.provider_run_id } : {}),
    ...(entry.prompt_origin !== undefined ? { prompt_origin: entry.prompt_origin } : {}),
    ...(entry.merge_key !== undefined ? { merge_key: entry.merge_key } : {}),
    ...(entry.source !== undefined ? { source: entry.source } : {}),
    ...(entry.external_provider !== undefined ? { external_provider: entry.external_provider } : {}),
    ...(entry.external_provider_session_id !== undefined ? { external_provider_session_id: entry.external_provider_session_id } : {}),
    ...(entry.external_provider_turn_id !== undefined ? { external_provider_turn_id: entry.external_provider_turn_id } : {}),
    ...(entry.observed_at_ms !== undefined ? { observed_at_ms: entry.observed_at_ms } : {}),
    ...(entry.external_observation !== undefined ? { external_observation: entry.external_observation } : {}),
    ...(entry.source_attachment_id !== undefined ? { source_attachment_id: entry.source_attachment_id } : {}),
    ...(entry.attachments !== undefined ? { attachments: cloneSessionHistoryPromptAttachments(entry.attachments) } : {}),
    ...(entry.timestamp_ms !== undefined ? { timestamp_ms: entry.timestamp_ms } : {}),
  }
}
