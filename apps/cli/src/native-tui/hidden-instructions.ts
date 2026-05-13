export const hiddenInstructionsStart = "<<<ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>"
export const hiddenInstructionsEnd = "<<<END_ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS>>>"

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")
}

const hiddenInstructionBlockPattern = new RegExp(
  `${escapeRegExp(hiddenInstructionsStart)}[\\s\\S]*?${escapeRegExp(hiddenInstructionsEnd)}(?:\\r?\\n\\r?\\n|\\\\n\\\\n)?`,
  "g",
)

export function redactHiddenInstructions(value: string): string {
  return value.replace(hiddenInstructionBlockPattern, "")
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
