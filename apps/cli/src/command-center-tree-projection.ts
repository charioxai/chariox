import type { CommandCenterItem } from "./command-center-types.js"

export type CommandNode = {
  id: string
  label: string
  description: string
  value: string
  searchAliases?: string[]
  children?: CommandNode[]
}

export function findDeepestScope(input: string, nodes: CommandNode[]): { node: CommandNode } | null {
  let match: CommandNode | null = null
  for (const node of nodes) {
    if (input === node.value.trim() || input === node.value || input.startsWith(node.value)) {
      if (!match || node.value.length > match.value.length) {
        match = node
      }
    }
    if (node.children?.length) {
      const childMatch = findDeepestScope(input, node.children)
      if (childMatch && (!match || childMatch.node.value.length > match.value.length)) {
        match = childMatch.node
      }
    }
  }
  return match ? { node: match } : null
}

export function collectCommandNodes(nodes: CommandNode[]): CommandNode[] {
  return nodes.flatMap((node) => [node, ...collectCommandNodes(node.children ?? [])])
}

export function mapNodeToItem(node: CommandNode): CommandCenterItem {
  return {
    id: node.id,
    label: node.label,
    description: node.children?.length ? `${node.description} (${node.children.length})` : node.description,
    kind: node.children?.length ? "group" : "command",
    value: node.value,
    ...commandNodeSearchAliases(node),
  }
}

export function mapRootGroup(node: CommandNode): CommandCenterItem {
  return {
    id: node.id,
    label: node.label,
    description: node.children?.length ? `${node.description} (${node.children.length})` : node.description,
    kind: "group",
    value: node.value,
    ...commandNodeSearchAliases(node),
  }
}

function commandNodeSearchAliases(node: CommandNode): { searchAliases: string[] } | Record<string, never> {
  const aliases = [
    ...(node.searchAliases ?? []),
    ...(node.children?.length ? collectDescendantSearchAliases(node) : []),
  ]
  return aliases.length > 0 ? { searchAliases: aliases } : {}
}

function collectDescendantSearchAliases(node: CommandNode): string[] {
  const aliases = new Set<string>()
  for (const alias of node.searchAliases ?? []) {
    aliases.add(alias)
  }
  for (const child of node.children ?? []) {
    aliases.add(child.label)
    aliases.add(child.value.trim())
    aliases.add(child.description)
    for (const alias of child.searchAliases ?? []) {
      aliases.add(alias)
    }
    for (const alias of collectDescendantSearchAliases(child)) {
      aliases.add(alias)
    }
  }
  return [...aliases]
}
