import type { ProviderNamespaceSubmitCommand } from "./provider-namespace-submit-policy.js"

export const PROVIDER_NAMESPACE_COMMAND_IDS = ["opencode", "codex"] as const

export type ProviderNamespaceCommandProviderId = typeof PROVIDER_NAMESPACE_COMMAND_IDS[number]

export type ParsedProviderNamespaceCommand = ProviderNamespaceSubmitCommand & {
  provider: ProviderNamespaceCommandProviderId
}

export function providerNamespace(provider: string) {
  return `/${provider}`
}

export function providerSupportsNamespaceCommands(
  provider: string | null | undefined,
): provider is ProviderNamespaceCommandProviderId {
  return !!provider
    && (PROVIDER_NAMESPACE_COMMAND_IDS as readonly string[]).includes(provider)
}

export function parseProviderNamespaceCommand(input: string): ParsedProviderNamespaceCommand | null {
  const trimmed = input.trim()
  const matchedProvider = PROVIDER_NAMESPACE_COMMAND_IDS.find((provider) => {
    const namespace = providerNamespace(provider)
    return trimmed === namespace || trimmed.startsWith(`${namespace} `)
  })
  if (!matchedProvider) {
    return null
  }
  const namespace = providerNamespace(matchedProvider)
  const forwardedBody = trimmed.slice(namespace.length).trim()
  if (!forwardedBody) {
    return {
      raw: trimmed,
      provider: matchedProvider,
      forwardedCommand: "",
    }
  }
  return {
    raw: trimmed,
    provider: matchedProvider,
    forwardedCommand: forwardedBody.startsWith("/") ? forwardedBody : `/${forwardedBody}`,
  }
}
