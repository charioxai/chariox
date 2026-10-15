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
  return [...rootNodes, ...miscNodes, ...(!rootNodes.some(n=>n.value.trim()==="/skills")?[{id:"skills-v2",label:"/skills",description:"Manage Chariox and Project skills",kind:"command" as const,value:"/skills"}]:[]), {
    id: "approvals", label: "/approvals", description: "Open pending approvals (Ctrl+G or F8)",
    kind: "command" as const, value: "/approvals",
  }]
}
