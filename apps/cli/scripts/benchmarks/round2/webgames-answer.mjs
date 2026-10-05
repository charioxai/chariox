// MP-08 / MP-10 / MP-11: WebGames transcript export, independent of scoring.
export function webGamesFinalOutput(turn, outputEntries, helpers) {
  const inlineOutput = [...(turn?.entries ?? []), ...(turn?.summary ? [turn.summary] : [])]
    .filter(item => item.entry?.kind === 'provider_output')
  if (turn?.lifecycle === 'completed') {
    return helpers.assembleSessionHistoryFinalMessage(turn, [...outputEntries, ...inlineOutput])
  }
  return [...outputEntries, ...inlineOutput].sort((a, b) => (a.sequence ?? 0) - (b.sequence ?? 0))
    .at(-1)?.entry.text ?? ''
}
