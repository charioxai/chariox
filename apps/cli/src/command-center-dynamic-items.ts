import { backendProviderLabel, catalogModelOptions, type BackendProviderId, type ProviderCatalog } from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
  providerNamespace,
  providerNamespaceDescription,
} from "./provider-command-catalog.js"
import { filterCommandCenterItems } from "./command-center-search.js"
import { mapNodeToItem, type CommandNode } from "./command-center-tree-projection.js"
import type { CommandCenterItem } from "./command-center-types.js"

export type CommandCenterDynamicContext = {
  providerCatalog: ProviderCatalog
  currentProvider: BackendProviderId
  currentModel: string
  currentVariant: string
}

export function providerNamespaceRootItem(
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
): CommandCenterItem {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  return {
    id: `provider-namespace-${provider}`,
    label: providerNamespace(provider),
    description: providerNamespaceDescription(provider, catalog.commands.length),
    kind: "group",
    value: `${providerNamespace(provider)} `,
    ...(catalog.commands.length > 0
      ? {
        searchAliases: catalog.commands.flatMap((command) => [command.name, command.description, command.value]),
      }
      : {}),
  }
}

export function providerNamespaceScopeNode(
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
): CommandNode {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  return {
    id: `provider-namespace-${provider}`,
    label: providerNamespace(provider),
    description: providerNamespaceDescription(provider, catalog.commands.length),
    value: `${providerNamespace(provider)} `,
  }
}

export function buildProviderNamespaceItems(
  input: string,
  provider: BackendProviderId,
  catalogs: ProviderCommandCatalogs,
) {
  const catalog = catalogs[provider] ?? emptyProviderCommandCatalog(provider)
  const namespace = providerNamespace(provider)
  const rootItem: CommandCenterItem = {
    id: `provider-namespace-${provider}`,
    label: namespace,
    description: catalog.commands.length > 0
      ? providerNamespaceDescription(provider, catalog.commands.length)
      : `${providerNamespaceDescription(provider, 0)}; no registered completions for this provider version yet`,
    kind: "group",
    value: `${namespace} `,
  }
  if (input === namespace || input === `${namespace} `) {
    return catalog.commands.length > 0
      ? catalog.commands.map((command) => ({
        id: `${provider}-${command.id}`,
        label: command.name,
        description: command.description,
        kind: "command" as const,
        value: `${namespace} ${command.value}`,
      }))
      : [rootItem]
  }
  const query = input.slice(`${namespace} `.length).trim().toLowerCase()
  if (catalog.commands.length === 0) {
    return filterCommandCenterItems([rootItem], query)
  }
  return filterCommandCenterItems(
    catalog.commands.map((command) => ({
      id: `${provider}-${command.id}`,
      label: command.name,
      description: command.description,
      kind: "command" as const,
      value: `${namespace} ${command.value}`,
    })),
    query,
  )
}

export function buildProviderItems(input: string, providerNode: CommandNode) {
  const query = input.slice("/provider ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    mapNodeToItem(providerNode),
    {
      id: "provider-opencode",
      label: "OpenCode",
      description: "Use the OpenCode backend",
      kind: "provider",
      value: "opencode",
    },
    {
      id: "provider-codex",
      label: "Codex",
      description: "Use the Codex backend",
      kind: "provider",
      value: "codex",
    },
    {
      id: "provider-claude",
      label: backendProviderLabel("claude"),
      description: "Use the Claude Code backend",
      kind: "provider",
      value: "claude",
    },
    {
      id: "provider-status",
      label: "status",
      description: "Show auth status for the current or named provider",
      kind: "command",
      value: "/provider status ",
    },
    {
      id: "provider-login",
      label: "login",
      description: "Start login for the current or named provider",
      kind: "command",
      value: "/provider login ",
    },
    {
      id: "provider-logout",
      label: "logout",
      description: "Log out the current or named provider",
      kind: "command",
      value: "/provider logout ",
    },
    {
      id: "provider-reauth",
      label: "reauth",
      description: "Log out and start a fresh login for the current or named provider",
      kind: "command",
      value: "/provider reauth ",
    },
  ], query)
}

export function buildModelItems(input: string, context: CommandCenterDynamicContext) {
  const query = input.slice("/model ".length).trim().toLowerCase()
  return filterCommandCenterItems(
    catalogModelOptions(context.providerCatalog, context.currentProvider).map((option) => ({
      id: `model-${option.id}`,
      label: `${option.providerName} ${option.label}`,
      description: option.id === context.currentModel ? "current model" : option.id,
      kind: "model" as const,
      value: option.id,
    })),
    query,
  )
}

export function buildVariantItems(input: string, context: CommandCenterDynamicContext) {
  const query = input.slice("/variant ".length).trim().toLowerCase()
  const current = catalogModelOptions(context.providerCatalog, context.currentProvider).find((option) => option.id === context.currentModel)
  const variants = current?.variants ?? []
  return filterCommandCenterItems(
    variants.map((variant) => ({
      id: `variant-${variant}`,
      label: variant,
      description: `${current?.label ?? context.currentModel}${variant === context.currentVariant ? " • current" : ""}`,
      kind: "variant" as const,
      value: variant,
    })),
    query,
  )
}

export function buildViewItems(input: string) {
  const query = input.slice("/view ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    {
      id: "view-individual",
      label: "individual",
      description: "Show one focused agent transcript at a time",
      kind: "command",
      value: "/view individual",
    },
    {
      id: "view-split",
      label: "split",
      description: "Split the response area across active session agents",
      kind: "command",
      value: "/view split",
    },
  ], query)
}

function emptyProviderCommandCatalog(provider: BackendProviderId) {
  return {
    provider,
    source: "shipped" as const,
    discovery: "none" as const,
    commands: [],
  }
}
