import type { RoomEnvironmentTabAccessibility, RoomEnvironmentAccessibilityNode } from "@chariox/kernel-client/kernel-types"

// The Tab's page as indented text: one line per node a reader announces.
export function formatRoomTabOutline(title: string, accessibility: RoomEnvironmentTabAccessibility): string {
  return [
    `Page outline of ${title} (tab ${accessibility.tab_id}, revision ${accessibility.document_revision}):`,
    ...formatAccessibilityNodes(accessibility.nodes, accessibility.truncated),
  ].join("\n")
}

export function formatAccessibilityNodes(nodes: readonly RoomEnvironmentAccessibilityNode[], truncated: boolean, maxDepth = Number.MAX_SAFE_INTEGER): string[] {
  const depth = new Map<string, number>()
  const lines = nodes.map((node) => {
    const level = node.parent_ref === undefined ? 0 : (depth.get(node.parent_ref) ?? -1) + 1
    depth.set(node.element_ref, level)
    const states = [...(node.states ?? []), node.focused ? "focused" : "", node.disabled ? "disabled" : ""].filter(Boolean)
    return [
      `${"  ".repeat(Math.min(maxDepth, level))}${node.role}`,
      node.name ? ` ${JSON.stringify(node.name)}` : "",
      node.value ? ` = ${JSON.stringify(node.value)}` : "",
      node.description ? ` (${node.description})` : "",
      states.length > 0 ? ` [${states.join(", ")}]` : "",
    ].join("")
  })
  return [
    ...(lines.length > 0 ? lines : ["(no readable content)"]),
    ...(truncated ? ["… the page is longer; this outline is shortened."] : []),
  ]
}

