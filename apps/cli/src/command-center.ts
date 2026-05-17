import { backendProviderLabel, catalogModelOptions, type BackendProviderId, type ProviderCatalog } from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
  providerNamespace,
  providerNamespaceDescription,
  providerSupportsNamespaceCommands,
} from "./provider-command-catalog.js"
import { filterCommandCenterItems } from "./command-center-search.js"
import { COMMAND_TREE } from "./command-center-tree.js"
import {
  collectCommandNodes,
  findDeepestScope,
  mapNodeToItem,
  mapRootGroup,
} from "./command-center-tree-projection.js"
import type { CommandCenterItem } from "./command-center-types.js"

export type { CommandCenterItem } from "./command-center-types.js"

type CommandContext = {
  providerCatalog: ProviderCatalog
  providerCommandCatalogs: ProviderCommandCatalogs
  currentProvider: BackendProviderId
  focusedProvider: BackendProviderId | null
  currentModel: string
  currentVariant: string
}

export function buildCommandCenterItems(input: string, context: CommandContext): CommandCenterItem[] {
  if (!input.startsWith("/")) {
    return []
  }

  const normalized = input.trimStart()
  if (!normalized.startsWith("/")) {
    return []
  }

  if (normalized === "/") {
    return rootItems(context)
  }

  if (normalized.startsWith("/provider ")) {
    return buildProviderItems(normalized, context)
  }

  if (normalized.startsWith("/model ")) {
    return buildModelItems(normalized, context)
  }

  if (normalized.startsWith("/variant ")) {
    return buildVariantItems(normalized, context)
  }

  if (normalized.startsWith("/view ")) {
    return buildViewItems(normalized)
  }

  if (context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)) {
    const namespace = `${providerNamespace(context.focusedProvider)} `
    if (normalized === providerNamespace(context.focusedProvider) || normalized.startsWith(namespace)) {
      return buildProviderNamespaceItems(normalized, context.focusedProvider, context.providerCommandCatalogs)
    }
  }

  const scoped = buildScopedCommandItems(normalized, context)
  if (scoped) {
    return scoped
  }

  return filterCommandCenterItems(rootItems(context), normalized.slice(1).toLowerCase())
}

export function shouldSubmitExactCommandCenterMatch(item: CommandCenterItem, currentPrompt: string) {
  if (item.kind !== "command") {
    return false
  }
  if (!item.value.endsWith(" ")) {
    return currentPrompt.trim() === item.value
  }
  return currentPrompt.startsWith(item.value) || currentPrompt === item.value.trim()
}

export function nextCommandCenterIndex(
  currentIndex: number,
  items: CommandCenterItem[],
  input: string,
  previousInput?: string,
) {
  if (items.length === 0) {
    return 0
  }

  if (previousInput !== input) {
    const normalized = input.trim()
    const exactGroupIndex = items.findIndex((item) => (
      item.kind === "group"
      && (normalized === item.value.trim() || input === item.value)
    ))
    if (exactGroupIndex >= 0) {
      return exactGroupIndex
    }
  }

  return Math.max(0, Math.min(currentIndex, items.length - 1))
}

function rootItems(context: CommandContext) {
  const rootNodes = COMMAND_TREE
    .filter((node) => node.id !== "misc")
    .map((node) => mapRootGroup(node))
  if (context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)) {
    const catalog = context.providerCommandCatalogs[context.focusedProvider] ?? emptyProviderCommandCatalog(context.focusedProvider)
    rootNodes.push({
      id: `provider-namespace-${context.focusedProvider}`,
      label: providerNamespace(context.focusedProvider),
      description: providerNamespaceDescription(context.focusedProvider, catalog.commands.length),
      kind: "group",
      value: `${providerNamespace(context.focusedProvider)} `,
      ...(catalog.commands.length > 0
        ? {
          searchAliases: catalog.commands.flatMap((command) => [command.name, command.description, command.value]),
        }
        : {}),
    })
  }
  const miscNodes = COMMAND_TREE.find((node) => node.id === "misc")?.children?.map(mapNodeToItem) ?? []
  return [...rootNodes, ...miscNodes]
}

function buildProviderNamespaceItems(
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

function buildProviderItems(input: string, context: CommandContext) {
  const query = input.slice("/provider ".length).trim().toLowerCase()
  return filterCommandCenterItems([
    mapNodeToItem(COMMAND_TREE.find((node) => node.id === "provider")!),
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

function buildModelItems(input: string, context: CommandContext) {
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

function buildVariantItems(input: string, context: CommandContext) {
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

function buildViewItems(input: string) {
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

function buildScopedCommandItems(input: string, context: CommandContext) {
  const scopeNodes = context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)
    ? [
      ...COMMAND_TREE,
      {
        id: `provider-namespace-${context.focusedProvider}`,
        label: providerNamespace(context.focusedProvider),
        description: providerNamespaceDescription(
          context.focusedProvider,
          (context.providerCommandCatalogs[context.focusedProvider] ?? emptyProviderCommandCatalog(context.focusedProvider)).commands.length,
        ),
        value: `${providerNamespace(context.focusedProvider)} `,
      },
    ]
    : COMMAND_TREE
  const nodes = collectCommandNodes(scopeNodes)
  const exactCommand = nodes.find((node) => !node.children?.length && input === node.value)
  if (exactCommand && input.endsWith(" ")) {
    return []
  }

  const scope = findDeepestScope(input, scopeNodes.filter((node) => node.value !== "/"))
  if (!scope) {
    return null
  }

  const query = input.slice(scope.node.value.length).trim().toLowerCase()
  if (scope.node.children?.length) {
    const items = [mapNodeToItem(scope.node), ...scope.node.children.map(mapNodeToItem)]
    return query ? filterCommandCenterItems(items, query) : items
  }
  return null
}

function emptyProviderCommandCatalog(provider: BackendProviderId) {
  return {
    provider,
    source: "shipped" as const,
    discovery: "none" as const,
    commands: [],
  }
}
