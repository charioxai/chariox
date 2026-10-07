import { providerSupportsNamespaceCommands } from "./provider-command-catalog.js"
import { providerNamespaceRootItem } from "./command-center-dynamic-items.js"
import { mapNodeToItem, mapRootGroup } from "./command-center-tree-projection.js"
import type { CommandCenterProviderNamespaceContext } from "./command-center-context.js"

export function buildCommandCenterRootItems(
  context: CommandCenterProviderNamespaceContext,
) {
  const rootNodes = context.commandTree
    .filter((node) => node.id !== "misc")
    .map((node) => mapRootGroup(node))
  if (context.focusedProvider && providerSupportsNamespaceCommands(context.focusedProvider)) {
    rootNodes.push(providerNamespaceRootItem(context.focusedProvider, context.providerCommandCatalogs))
  }
  const miscNodes = context.commandTree.find((node) => node.id === "misc")?.children?.map(mapNodeToItem) ?? []
  return [...rootNodes, ...miscNodes, {
    id: "computer", label: "/computer", description: "Start or take over the owned desktop",
    kind: "command" as const, value: "/computer",
  }, {
    id: "access", label: "/access", description: "List or revoke user-domain access",
    kind: "command" as const, value: "/access",
  }, {
    id: "approvals", label: "/approvals", description: "Open pending approvals (Ctrl+G or F8)",
    kind: "command" as const, value: "/approvals",
  }]
}
