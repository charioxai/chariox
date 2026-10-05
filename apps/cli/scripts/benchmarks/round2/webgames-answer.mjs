// MP-08 / MP-10 / MP-11: WebGames transcript export, independent of scoring.
import { finalAnswer } from './export.mjs'

export function webGamesFinalOutput(turn, outputEntries, helpers) {
  const inlineOutput = [...(turn?.entries ?? [])]
    .filter(item => item.entry?.kind === 'provider_output')
  if (turn?.lifecycle === 'completed') {
    return finalAnswer(turn, [...outputEntries, ...inlineOutput], helpers)
  }
  return [...outputEntries, ...inlineOutput, ...(turn?.summary ? [turn.summary] : [])]
    .sort((a, b) => (a.sequence ?? 0) - (b.sequence ?? 0))
    .at(-1)?.entry.text ?? ''
}
