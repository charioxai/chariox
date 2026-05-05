export type PromptMetaTone = "primary" | "secondary" | "accent" | "warning" | "success" | "info" | "text"

export type PromptMetaPart = {
  kind: "provider" | "model" | "variant"
  text: string
  tone: PromptMetaTone
}

export type PromptUsageMeta = {
  tokensLabel: string
  usagePercent: number | null
  usageLabel: string
  barFilled: string
  barEmpty: string
}

const promptUsageNumber = new Intl.NumberFormat("en-US")

export function formatPromptMetaLine(provider: string, model: string, effort: string) {
  return formatPromptMetaParts(provider, model, effort).map((part) => part.text).join(" • ")
}

export function formatPromptMetaParts(provider: string, model: string, effort: string): PromptMetaPart[] {
  const parts: PromptMetaPart[] = [
    {
      kind: "provider",
      text: formatProvider(provider),
      tone: toneForProvider(provider),
    },
    {
      kind: "model",
      text: formatModel(model),
      tone: toneForModel(model),
    },
  ]

  const effortLabel = formatEffort(effort)
  if (effortLabel) {
    parts.push({
      kind: "variant",
      text: effortLabel,
      tone: toneForVariant(effort),
    })
  }

  return parts
}

export function formatPromptUsageMeta(
  totalTokens: number | null | undefined,
  contextTokens: number | null | undefined,
  contextLimit: number | null | undefined,
  barWidth = 10,
): PromptUsageMeta | null {
  if (totalTokens == null || totalTokens < 0) {
    return null
  }

  const usagePercent = contextTokens != null && contextTokens >= 0 && contextLimit && contextLimit > 0 && contextTokens <= contextLimit
    ? Math.round((contextTokens / contextLimit) * 100)
    : null
  const clampedPercent = usagePercent == null
    ? 0
    : Math.max(0, Math.min(100, usagePercent))
  const filledCount = Math.round((clampedPercent / 100) * barWidth)

  return {
    tokensLabel: `${promptUsageNumber.format(totalTokens)} tok`,
    usagePercent,
    usageLabel: usagePercent == null ? "" : `${usagePercent}%`,
    barFilled: "=".repeat(filledCount),
    barEmpty: "-".repeat(Math.max(0, barWidth - filledCount)),
  }
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
  if (/^codex$/i.test(providerId)) {
    return formatKnownLabel(modelId)
  }
  // Show provider first, then model name (e.g., "OpenAI GPT-5.4", "Zen Kimi 2.5")
  return `${formatKnownLabel(providerId)} ${formatKnownLabel(modelId)}`
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
      if (/^codex$/i.test(part)) {
        return "Codex"
      }
      return part.charAt(0).toUpperCase() + part.slice(1)
    })
    .join("")
}

function toneForProvider(provider: string): PromptMetaTone {
  const normalized = provider.trim().toLowerCase()
  if (!normalized) {
    return "text"
  }
  if (normalized.includes("opencode")) {
    return "primary"
  }
  if (normalized.includes("openai") || normalized.includes("github")) {
    return "info"
  }
  if (normalized.includes("anthropic") || normalized.includes("claude")) {
    return "warning"
  }
  if (normalized.includes("google") || normalized.includes("gemini") || normalized.includes("deepseek")) {
    return "success"
  }
  if (normalized.includes("xai") || normalized.includes("grok")) {
    return "accent"
  }
  return fallbackTone(normalized)
}

function toneForModel(model: string): PromptMetaTone {
  const normalized = model.trim().toLowerCase()
  if (!normalized || normalized === "default") {
    return "text"
  }
  if (/(^|\/)gpt|\bo[134]\b/.test(normalized)) {
    return "secondary"
  }
  if (normalized.includes("claude") || normalized.includes("anthropic")) {
    return "warning"
  }
  if (normalized.includes("gemini") || normalized.includes("deepseek")) {
    return "success"
  }
  if (normalized.includes("grok") || normalized.includes("xai")) {
    return "accent"
  }
  if (normalized.includes("llama") || normalized.includes("mistral")) {
    return "primary"
  }
  if (normalized.includes("qwen") || normalized.includes("github")) {
    return "info"
  }
  return fallbackTone(normalized)
}

function toneForVariant(effort: string): PromptMetaTone {
  const normalized = effort.trim().toLowerCase()
  if (!normalized || normalized === "default") {
    return "text"
  }
  if (normalized === "low" || normalized === "fast") {
    return "success"
  }
  if (normalized === "medium" || normalized === "balanced" || normalized === "normal") {
    return "warning"
  }
  if (normalized === "high" || normalized === "quality") {
    return "primary"
  }
  if (normalized === "max") {
    return "accent"
  }
  return fallbackTone(normalized)
}

function fallbackTone(value: string): PromptMetaTone {
  const tones: PromptMetaTone[] = ["primary", "secondary", "accent", "warning", "success", "info"]
  let hash = 0
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0
  }
  return tones[hash % tones.length] ?? "text"
}
