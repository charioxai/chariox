import { formatAccessibilityNodes } from "./accessibility-outline.js"
import type { RoomEnvironmentAccessibilityNode, UserAppView } from "@chariox/kernel-client/kernel-types"

/** App text is data, including descriptions/roles; never terminal control bytes. */
export function appViewText(value: unknown, limit = 1024): string {
  return typeof value === "string" ? value.slice(0, limit).replace(/[\u0000-\u001f\u007f-\u009f]/g,
    char => "\\u" + char.charCodeAt(0).toString(16).padStart(4, "0")) : ""
}

export function formatUserAppViewOutline(view: UserAppView, snapshot: unknown): string {
  const raw = snapshot as { accessibility_nodes?: unknown; truncated?: boolean } | null
  if (!raw || !Array.isArray(raw.accessibility_nodes)) throw new Error("The kernel did not return an App accessibility outline")
  const nodes: RoomEnvironmentAccessibilityNode[] = raw.accessibility_nodes.slice(0, 500).flatMap(item => {
    if (!item || typeof item !== "object" || item.ignored === true) return []
    if (typeof item.node_ref !== "string" || typeof item.role !== "string") return []
    return [{ element_ref: appViewText(item.node_ref),
      ...(typeof item.parent_ref === "string" ? {parent_ref: appViewText(item.parent_ref)} : {}),
      role: appViewText(item.role), name: appViewText(item.name), value: appViewText(item.value),
      description: appViewText(item.description), focused: item.focused === true, disabled: item.disabled === true,
      states: Array.isArray(item.states) ? item.states.slice(0,16).map((state: unknown) => appViewText(state,64)) : [],
    }]
  })
  return [`App ${appViewText(view.installation_id)} · user domain`,
    `View ${appViewText(view.view_id)}`,
    ...formatAccessibilityNodes(nodes, raw.truncated === true || raw.accessibility_nodes.length > 500, 16),
  ].join("\n")
}
