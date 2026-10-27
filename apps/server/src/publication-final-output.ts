export type NormalizedFinalOutput = {
  message: unknown
  text: string
  artifacts: unknown[]
}

export function normalizeFinalOutput(output: unknown): NormalizedFinalOutput {
  if (!output || typeof output !== "object" || Array.isArray(output)) {
    const message = output ?? ""
    return { message, text: finalOutputText(message), artifacts: [] }
  }
  const record = output as Record<string, unknown>
  const message = Object.hasOwn(record, "message") ? record.message : output
  const artifacts = Array.isArray(record.artifacts) ? record.artifacts : []
  return { message, text: finalOutputText(message), artifacts }
}

export function finalOutputText(value: unknown): string {
  if (typeof value === "string") return value
  if (value == null) return ""
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}
