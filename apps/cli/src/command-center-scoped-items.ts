import { providerSupportsNamespaceCommands } from "./provider-command-catalog.js"
import { providerNamespaceScopeNode } from "./command-center-dynamic-items.js"
import { filterCommandCenterItems } from "./command-center-search.js"
import {
  collectCommandNodes,
  findDeepestScope,
  mapNodeToItem,
} from "./command-center-tree-projection.js"
import type { CommandCenterProviderNamespaceContext } from "./command-center-context.js"

export function buildScopedCommandCenterItems(
  input: string,
  context: CommandCenterProviderNamespaceContext,
) {
  const scopeNodes = context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)
    ? [
      ...context.commandTree,
      providerNamespaceScopeNode(context.focusedProvider, context.providerCommandCatalogs),
    ]
    : context.commandTree
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
    const scopedNodes = query ? collectCommandNodes(scope.node.children) : scope.node.children
    const items = [mapNodeToItem(scope.node), ...scopedNodes.map(mapNodeToItem)]
    return query ? filterCommandCenterItems(items, query) : items
  }
  return null
}
