type ExternalProviderObservationSpec = {
  readonly provider: string
  readonly passiveStatusPrefixes: readonly string[]
}

const EXTERNAL_PROVIDER_OBSERVATION_SPECS: readonly ExternalProviderObservationSpec[] = [{
  provider: "codex",
  passiveStatusPrefixes: ["codex token_count"],
}, {
  provider: "claude",
  passiveStatusPrefixes: ["claude last-prompt", "claude ai-title"],
}, {
  provider: "opencode",
  passiveStatusPrefixes: [],
}]

export function externalProviderStatusIsPassiveTelemetry(
  provider: string | null | undefined,
  text: string,
): boolean {
  const spec = externalProviderObservationSpec(provider)
  return spec?.passiveStatusPrefixes.some((prefix) => text.startsWith(prefix)) ?? false
}

function externalProviderObservationSpec(
  provider: string | null | undefined,
): ExternalProviderObservationSpec | null {
  const normalized = provider?.trim().toLowerCase()
  if (!normalized) {
    return null
  }
  return EXTERNAL_PROVIDER_OBSERVATION_SPECS.find((spec) => spec.provider === normalized) ?? null
}
