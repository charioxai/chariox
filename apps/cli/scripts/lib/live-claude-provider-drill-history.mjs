export function claudeProviderOutputContainsMarker(entries, expected) {
  const providerOutput = (Array.isArray(entries) ? entries : [])
    .filter((entry) => entry?.kind === 'provider_output' && typeof entry.text === 'string')
    .map((entry) => entry.text)
    .join('\n')
  const compactOutput = providerOutput.replace(/\s+/g, '')
  const compactExpected = String(expected ?? '').replace(/\s+/g, '')
  return providerOutput.includes(expected) || (
    compactExpected.length > 0 && compactOutput.includes(compactExpected)
  )
}
