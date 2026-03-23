export function formatPromptMetaLine(provider: string, model: string, effort: string) {
  return [formatProvider(provider), formatModel(model), formatEffort(effort)].filter(Boolean).join(" • ")
}

function formatProvider(provider: string) {
  return formatKnownLabel(provider) || "OpenCode"
}

function formatModel(model: string) {
  const normalized = model.trim()
  if (!normalized || normalized === "default") {
    return "Default"
  }
  if (!normalized.includes("/")) {
    return formatKnownLabel(normalized)
  }
  const parts = normalized.split("/").filter(Boolean)
  const modelId = parts.at(-1)
  const providerId = parts.length > 1 ? parts.at(-2) : undefined
  if (!modelId) {
    return formatKnownLabel(normalized)
  }
  if (!providerId) {
    return formatKnownLabel(modelId)
  }
  return `${formatKnownLabel(modelId)} ${formatKnownLabel(providerId)}`
}

function formatEffort(effort: string) {
  const normalized = effort.trim()
  if (!normalized || normalized === "default") {
    return ""
  }
  return formatKnownLabel(normalized)
}

function formatKnownLabel(value: string) {
  return value
    .split(/([\/-])/)
    .map((part) => {
      if (part === "/" || part === "-") {
        return part
      }
      if (!part) {
        return ""
      }
      if (/^gpt$/i.test(part)) {
        return "GPT"
      }
      if (/^openai$/i.test(part)) {
        return "OpenAI"
      }
      if (/^github$/i.test(part)) {
        return "GitHub"
      }
      if (/^opencode$/i.test(part)) {
        return "OpenCode"
      }
      return part.charAt(0).toUpperCase() + part.slice(1)
    })
    .join("")
}
