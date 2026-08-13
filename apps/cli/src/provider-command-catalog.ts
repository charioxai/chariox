import {
  backendProviderLabel,
  type BackendProviderId,
} from "./provider-catalog.js"
import {
  parseProviderNamespaceCommand as parseSharedProviderNamespaceCommand,
  providerNamespace,
  providerSupportsNamespaceCommands,
  type ParsedProviderNamespaceCommand as SharedParsedProviderNamespaceCommand,
} from "@chariox/kernel-client/provider-namespace-command"

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
  catalog_source?: "daemon" | "local_fallback"
  unavailable_reason?: string
  commands: ProviderCommandDescriptor[]
}

export type ProviderCommandCatalogs = Record<BackendProviderId, ProviderCommandCatalog>
type ProviderCommandCatalogFallbackOptions = {
  catalogSource?: "local_fallback"
  unavailableReason?: string
}

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
  "claude-headless": {
    provider: "claude-headless",
    source: "shipped",
    discovery: "none",
    // Claude Code CLI commands are not exposed through Chariox forwarding yet.
    commands: [],
  },
  "claude-p": {
    provider: "claude-p",
    source: "shipped",
    discovery: "none",
    // Claude Code CLI commands are not exposed through Chariox forwarding yet.
    commands: [],
  },
}

export {
  providerNamespace,
  providerSupportsNamespaceCommands,
}

export function fallbackProviderCommandCatalogs(
  options: ProviderCommandCatalogFallbackOptions = {},
): ProviderCommandCatalogs {
  const metadata = options.catalogSource
    ? {
      catalog_source: options.catalogSource,
      ...(options.unavailableReason ? { unavailable_reason: options.unavailableReason } : {}),
    }
    : {}
  return {
    opencode: {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS.opencode,
      ...metadata,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS.opencode.commands],
    },
    codex: {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS.codex,
      ...metadata,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS.codex.commands],
    },
    "claude-headless": {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS["claude-headless"],
      ...metadata,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS["claude-headless"].commands],
    },
    "claude-p": {
      ...EMPTY_PROVIDER_COMMAND_CATALOGS["claude-p"],
      ...metadata,
      commands: [...EMPTY_PROVIDER_COMMAND_CATALOGS["claude-p"].commands],
    },
  }
}

export function providerCommandCatalogIsLocalFallback(catalog: ProviderCommandCatalog) {
  return catalog.catalog_source === "local_fallback"
}

export function providerNamespaceDescription(
  provider: BackendProviderId,
  commandCount: number,
  options: { localFallback?: boolean } = {},
) {
  const providerLabel = backendProviderLabel(provider)
  const suffix = options.localFallback ? "; local command list" : ""
  if (commandCount > 0) {
    return `Forward ${providerLabel} native commands to the focused agent (${commandCount})${suffix}`
  }
  return `Forward ${providerLabel} native commands to the focused agent${suffix}`
}

export type ParsedProviderNamespaceCommand = SharedParsedProviderNamespaceCommand & {
  provider: BackendProviderId
}

export function parseProviderNamespaceCommand(
  input: string,
  _focusedProvider: BackendProviderId | null | undefined,
): ParsedProviderNamespaceCommand | null {
  return parseSharedProviderNamespaceCommand(input)
}
