import {
  backendProviderLabel,
  type BackendProviderId,
} from "./provider-catalog.js"

const PROVIDER_NAMESPACE_COMMAND_IDS = ["opencode", "codex"] as const

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
  claude: {
    provider: "claude",
    source: "shipped",
    discovery: "none",
    // Claude Code CLI commands are not exposed through Arroba forwarding yet.
    commands: [],
  },
}

export function providerNamespace(provider: BackendProviderId) {
  return `/${provider}`
}

export function providerSupportsNamespaceCommands(provider: BackendProviderId) {
  return (PROVIDER_NAMESPACE_COMMAND_IDS as readonly BackendProviderId[]).includes(provider)
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
    claude: {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS.claude,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS.claude.commands],
    },
  }
}

export function providerNamespaceDescription(provider: BackendProviderId, commandCount: number) {
  const providerLabel = backendProviderLabel(provider)
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
