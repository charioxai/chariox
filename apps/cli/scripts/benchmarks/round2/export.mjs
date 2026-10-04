// MP-08 / MP-10 / MP-11 H2/H7. Evaluators receive only admitted complete answers.
import { mkdir, writeFile, rename } from 'node:fs/promises'
import path from 'node:path'
import { sanitizeDrillMetadata } from '../../lib/drill-secrets.mjs'
import { packageRound2Answer } from './policy.mjs'

export function finalAnswer(turn, entries, helpers) {
  // summary is excluded by loadTurnHistory; tolerate its preview repeated
  // by callers without treating a combined preview as an original entry.
  const summary = turn.summary
  const originals = entries.filter(item => !summary || !(item.entry_index === summary.entry_index
    && item.fragment_start === summary.fragment_start && item.fragment_end === summary.fragment_end
    && item.total_chars === summary.total_chars && item.entry.kind === summary.entry.kind
    && item.entry.provider_run_id === summary.entry.provider_run_id && item.entry.merge_key === summary.entry.merge_key
    && item.entry.text === summary.entry.text))
  const answer = helpers.assembleSessionHistoryFinalMessage(turn, originals)
  if (!answer.length) throw new Error('MP-08 / MP-10 / MP-11 empty final message')
  return answer
}

export function settlementRecord({ turn, entries = [], elapsedMs, sessionId, agentId, cancellation = null, helpers }) {
  const errors = new Map()
  for (const item of entries.filter(item => item.entry.kind === 'provider_error')) {
    const group = errors.get(item.entry_index) ?? []
    group.push(item); errors.set(item.entry_index, group)
  }
  const providerErrors = [...errors.entries()].map(([entryIndex, group]) => {
    let text
    try {
      if (helpers) text = helpers.assembleSessionHistoryEntry(group)
      else {
        const item = group[0]
        if (group.length !== 1 || item.fragment_start !== 0 || item.fragment_end !== item.total_chars
          || Array.from(item.entry.text).length !== item.total_chars) throw new Error('incomplete provider error')
        text = item.entry.text
      }
    } catch { return { entryIndex, complete: false, error: null } }
    let error
    try { error = JSON.parse(text) } catch { error = { message: text } }
    return { entryIndex, complete: true, error: sanitizeDrillMetadata(error) }
  })
  const lifecycle = turn?.lifecycle ?? 'open'
  const accepted = lifecycle === 'completed' && !providerErrors.length && !cancellation
  return {
    mpItems: ['MP-08', 'MP-10', 'MP-11'], denominatorIncluded: true,
    sessionId: sessionId ?? null, agentId: agentId ?? null,
    promptId: turn?.prompt_id ?? null, turnId: turn?.turn_id ?? null,
    lifecycle, elapsedMs, providerErrors,
    cancellation: sanitizeDrillMetadata(cancellation), status: accepted ? 'settled' : 'RED',
    firstFailingSeam: accepted ? null : 'provider_settlement',
    failureCause: accepted ? null : cancellation?.cause ?? (providerErrors.some(error => error.complete) ? 'provider_error' : 'unknown'),
  }
}

export async function writeEvidence(directory, name, value) {
  if (!path.isAbsolute(directory) || !/^[a-z0-9][a-z0-9._-]*$/i.test(name)) throw new Error('MP-10 invalid evidence path')
  await mkdir(directory, { recursive: true, mode: 0o700 })
  const target = path.join(directory, name)
  await writeFile(`${target}.next`, JSON.stringify(sanitizeDrillMetadata(value), null, 2) + '\n', { mode: 0o600 })
  await rename(`${target}.next`, target)
}

export async function exportFinalAnswer({ directory, turn, entries, helpers, settlement, answerContract }) {
  if (settlement.status !== 'settled') throw new Error('MP-08 / MP-10 / MP-11 provider_settlement RED')
  const answer = finalAnswer(turn, entries, helpers)
  // Sanitization must never silently turn an altered answer into a scored one.
  if (sanitizeDrillMetadata(answer) !== answer) throw new Error('MP-10 secret-like final answer rejected')
  const packaged = packageRound2Answer(answer, answerContract)
  if (JSON.stringify(sanitizeDrillMetadata(packaged)) !== JSON.stringify(packaged)) throw new Error('MP-10 secret-like packaged answer rejected')
  await mkdir(directory, { recursive: true, mode: 0o700 })
  await writeFile(path.join(directory, 'answer.txt'), answer, { mode: 0o600 })
  await writeEvidence(directory, 'answer-package.json', { mpItems: ['MP-08', 'MP-10', 'MP-11'], contract: answerContract ?? { kind: 'text' }, answer: packaged })
  return packaged
}
