// MP-08/MP-10: normalize official provider transcript envelopes for evidence.
export function roomProviderToolName(value) {
  if (typeof value !== "string") return ""
  return value.replace(/^(?:mcp__chariox__|chariox_)/, "").replace(/^(?:chariox\.|chariox_)/, "")
}

export function roomProviderToolOutput(value) {
  try {
    const decode = value => {
      for (let depth = 0; depth < 3 && typeof value === "string"; depth++) value = JSON.parse(value)
      return value && typeof value === "object" ? value : null
    }
    const output = decode(value)
    if (output?.structuredContent != null) return decode(output.structuredContent)
    if (!Array.isArray(output?.content)) return output
    const texts = output.content.filter(item => item?.type === "text" && typeof item.text === "string")
    // Ambiguous, non-JSON and image-only results cannot establish business data.
    return texts.length === 1 ? decode(texts[0].text) : null
  } catch {
    return null
  }
}
