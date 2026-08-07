import type {
  TerminalCommandCatalog,
  TerminalCommandCatalogExecutionTarget,
  TerminalCommandCatalogNode,
  TerminalCommandCatalogSurface,
} from "@arroba/kernel-client/kernel-types"
import type { CommandNode } from "./command-center-tree-projection.js"

export type TerminalCommandCatalogProjectionOptions = {
  surface?: TerminalCommandCatalogSurface
  executionTargets?: readonly TerminalCommandCatalogExecutionTarget[]
  includeNode?: (node: TerminalCommandCatalogNode) => boolean
}

export function commandTreeFromTerminalCommandCatalog(
  catalog: TerminalCommandCatalog | null | undefined,
  options: TerminalCommandCatalogProjectionOptions = {},
): CommandNode[] {
  return commandNodesFromCatalogNodes(catalog?.nodes ?? [], options)
}

function commandNodesFromCatalogNodes(
  nodes: readonly TerminalCommandCatalogNode[],
  options: TerminalCommandCatalogProjectionOptions,
): CommandNode[] {
  return nodes.flatMap((node) => {
    if (options.includeNode && !options.includeNode(node)) {
      return []
    }
    const children = commandNodesFromCatalogNodes(node.children ?? [], options)
    if (node.kind === "group" && (node.children?.length ?? 0) > 0 && children.length === 0) {
      return []
    }
    const surfaceMatches = !options.surface || node.surfaces.includes(options.surface)
    const targetMatches = !options.executionTargets || options.executionTargets.includes(node.execution_target)
    if ((!surfaceMatches || !targetMatches) && children.length === 0) {
      return []
    }
    return [commandNodeFromCatalogNode(node, children)]
  })
}

function commandNodeFromCatalogNode(
  node: TerminalCommandCatalogNode,
  children: readonly CommandNode[],
): CommandNode {
  return {
    id: node.id,
    label: node.label,
    description: node.description,
    value: node.value,
    kind: node.kind,
    executionTarget: node.execution_target,
    surfaces: node.surfaces,
    ...(node.intents?.length ? { intents: node.intents } : {}),
    ...(node.examples?.length ? { examples: node.examples } : {}),
    ...(node.dynamic_source != null ? { dynamicSource: node.dynamic_source } : {}),
    ...(node.search_aliases?.length ? { searchAliases: node.search_aliases } : {}),
    ...(children.length ? { children } : {}),
  }
}
