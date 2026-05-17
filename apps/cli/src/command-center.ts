import type { BackendProviderId, ProviderCatalog } from "./provider-catalog.js"
import {
  type ProviderCommandCatalogs,
  providerNamespace,
  providerSupportsNamespaceCommands,
} from "./provider-command-catalog.js"
import {
  buildModelItems,
  buildProviderItems,
  buildProviderNamespaceItems,
  buildVariantItems,
  buildViewItems,
  providerNamespaceRootItem,
  providerNamespaceScopeNode,
} from "./command-center-dynamic-items.js"
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
    return buildProviderItems(normalized, COMMAND_TREE.find((node) => node.id === "provider")!)
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
    rootNodes.push(providerNamespaceRootItem(context.focusedProvider, context.providerCommandCatalogs))
  }
  const miscNodes = COMMAND_TREE.find((node) => node.id === "misc")?.children?.map(mapNodeToItem) ?? []
  return [...rootNodes, ...miscNodes]
}

function buildScopedCommandItems(input: string, context: CommandContext) {
  const scopeNodes = context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)
    ? [
      ...COMMAND_TREE,
      providerNamespaceScopeNode(context.focusedProvider, context.providerCommandCatalogs),
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
