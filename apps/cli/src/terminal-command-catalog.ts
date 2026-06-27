import type {
  TerminalCommandCatalog,
  TerminalCommandCatalogNode,
} from "@arroba/kernel-client/kernel-types"
import type { CommandNode } from "./command-center-tree-projection.js"

export function commandTreeFromTerminalCommandCatalog(
  catalog: TerminalCommandCatalog | null | undefined,
): CommandNode[] {
  return (catalog?.nodes ?? []).map(commandNodeFromCatalogNode)
}

function commandNodeFromCatalogNode(node: TerminalCommandCatalogNode): CommandNode {
  return {
    id: node.id,
    label: node.label,
    description: node.description,
    value: node.value,
    ...(node.search_aliases?.length ? { searchAliases: node.search_aliases } : {}),
    ...(node.children?.length ? { children: node.children.map(commandNodeFromCatalogNode) } : {}),
  }
}
