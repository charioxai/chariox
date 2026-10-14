export const hiddenInstructionsStart = "<<<CHARIOX_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>"
export const hiddenInstructionsEnd = "<<<END_CHARIOX_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>"

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

const hiddenInstructionBlockPattern = new RegExp(
  `${escapeRegExp(hiddenInstructionsStart)}[\\s\\S]*?${escapeRegExp(hiddenInstructionsEnd)}(?:\\r?\\n\\r?\\n|\\\\n\\\\n)?`,
  "g",
)

const workflowInstructionStartPattern = /<workflow-level-prompt>|<node-level-prompt>|<workflow-runtime-instructions>|<system-node-level-prompt>|Workflow-level prompt:\n/g

export function redactHiddenInstructions(value: string): string {
  const visible = value.replace(hiddenInstructionBlockPattern, "")
  for (const match of visible.matchAll(workflowInstructionStartPattern)) {
    const index = match.index
    // A code span containing only a marker is a public literal, not context.
    // Spans that also contain a body remain subject to the normal redaction.
    const before = visible.slice(0, index).match(/`+$/)?.[0]
    const after = visible.slice(index + match[0].length).match(/^`+/)?.[0]
    if (match[0].startsWith("<") && before && before === after) continue
    const start = visible.slice(0, index).endsWith("\r\n") ? index - 2
      : visible[index - 1] === "\n" ? index - 1 : index
    return visible.slice(0, start)
  }
  return visible
}

export function redactHiddenInstructionsFromJson(value: unknown): unknown {
  if (typeof value === "string") return redactHiddenInstructions(value)
  if (Array.isArray(value)) return value.map((entry) => redactHiddenInstructionsFromJson(entry))
  if (!value || typeof value !== "object") return value
  const result: Record<string, unknown> = {}
  for (const [key, entry] of Object.entries(value)) {
    result[key] = redactHiddenInstructionsFromJson(entry)
  }
  return result
}
