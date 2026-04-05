import type { BackendProviderId } from "./provider-catalog.js"

export type ProviderCommandDescriptor = {
  id: string
  name: string
  description: string
  value: string
}

export type ProviderCommandCatalog = {
  provider: BackendProviderId
  source: "shipped" | "discovered" | "merged"
  discovery: "none" | "provider_api" | "custom_files" | "driver"
  commands: ProviderCommandDescriptor[]
}

export type ProviderCommandCatalogs = Record<BackendProviderId, ProviderCommandCatalog>

const EMPTY_PROVIDER_COMMAND_CATALOGS: ProviderCommandCatalogs = {
  opencode: {
    provider: "opencode",
    source: "shipped",
    discovery: "none",
    // Current repository-backed adapter code exposes OpenCode config/agent/provider/session APIs,
    // but no machine-readable command catalog endpoint yet.
    commands: [],
  },
  codex: {
    provider: "codex",
    source: "shipped",
    discovery: "none",
    // Current repository-backed Codex adapter code exposes model/auth/turn APIs,
    // but no machine-readable provider-command list yet.
    commands: [],
  },
}

export function providerNamespace(provider: BackendProviderId) {
  return `/${provider}`
}

export function fallbackProviderCommandCatalogs(): ProviderCommandCatalogs {
  return {
    opencode: {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS.opencode,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS.opencode.commands],
    },
    codex: {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS.codex,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS.codex.commands],
    },
  }
}

export function providerNamespaceDescription(provider: BackendProviderId, commandCount: number) {
  const providerLabel = provider === "codex" ? "Codex" : "OpenCode"
  if (commandCount > 0) {
    return `Forward ${providerLabel} native commands to the focused agent (${commandCount})`
  }
  return `Forward ${providerLabel} native commands to the focused agent`
}

export type ParsedProviderNamespaceCommand = {
  raw: string
  provider: BackendProviderId
  forwardedCommand: string
}

export function parseProviderNamespaceCommand(
  input: string,
  focusedProvider: BackendProviderId | null | undefined,
): ParsedProviderNamespaceCommand | null {
  const trimmed = input.trim()
  const matchedProvider = (["opencode", "codex"] as const).find((provider) => {
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
