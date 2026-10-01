// MP-08/MP-10: normalize official provider transcript envelopes for evidence.
export function roomProviderToolName(value) {
  if (typeof value !== "string") return ""
  return value.replace(/^mcp__chariox__/, "").replace(/^(?:chariox\.|chariox_)/, "")
}

export function roomProviderToolOutput(value) {
  try {
    const output = typeof value === "string" ? JSON.parse(value) : value
    if (!Array.isArray(output?.content)) return output
    const texts = output.content.filter(item => item?.type === "text" && typeof item.text === "string")
    // Ambiguous, non-JSON and image-only results cannot establish business data.
    return texts.length === 1 ? JSON.parse(texts[0].text) : null
  } catch {
    return null
  }
}
