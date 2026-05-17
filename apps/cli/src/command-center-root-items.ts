import { providerSupportsNamespaceCommands } from "./provider-command-catalog.js"
import { providerNamespaceRootItem } from "./command-center-dynamic-items.js"
import { COMMAND_TREE } from "./command-center-tree.js"
import { mapNodeToItem, mapRootGroup } from "./command-center-tree-projection.js"
import type { CommandCenterProviderNamespaceContext } from "./command-center-context.js"

export function buildCommandCenterRootItems(
  context: CommandCenterProviderNamespaceContext,
) {
  const rootNodes = COMMAND_TREE
    .filter((node) => node.id !== "misc")
    .map((node) => mapRootGroup(node))
  if (context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)) {
    rootNodes.push(providerNamespaceRootItem(context.focusedProvider, context.providerCommandCatalogs))
  }
  const miscNodes = COMMAND_TREE.find((node) => node.id === "misc")?.children?.map(mapNodeToItem) ?? []
  return [...rootNodes, ...miscNodes]
}
