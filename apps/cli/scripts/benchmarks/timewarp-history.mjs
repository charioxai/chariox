// MP-08/MP-10: audit complete original history, including inline native events.
import { roomProviderToolName } from '../lib/room-provider-tool-record.mjs'
import { assembleTurnEntries } from './round2/kernel.mjs'
import { finalAnswer } from './round2/export.mjs'
export function auditTimeWarpHistory(turn, entries, helpers) {
  const originals = assembleTurnEntries(entries, helpers)
  const tools = originals.filter(item => item.entry.kind === 'provider_tool')
    .map(item => JSON.parse(item.entry.text))
  return { answer: finalAnswer(turn, entries, helpers),
    tools: tools.map(record => ({ ...record, tool: roomProviderToolName(record.tool) })),
    providerError: originals.some(item => item.entry.kind === 'provider_error') }
}
