export function nonEmpty(value?: string | null) {
  const normalized = value?.trim()
  return normalized ? normalized : null
}

export function trimTrailingNewlines(value: string) {
  return value.replace(/[\r\n]+$/, "")
}

export function renderDetail(value: unknown) {
  if (value == null) {
    return ""
  }
  if (typeof value === "string") {
    return value
  }
  return JSON.stringify(value, null, 2)
}

export function readString(value: unknown) {
  return typeof value === "string" && value.trim() ? value : null
}

export function isObjectValue(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value)
}

export function normalizeJsonLike(value: unknown): unknown {
  if (typeof value !== "string") {
    return value
  }
  const trimmed = value.trim()
  if (!trimmed || (!trimmed.startsWith("[") && !trimmed.startsWith("{"))) {
    return value
  }
  try {
    return JSON.parse(trimmed)
  } catch {
    return value
  }
}

export function normalizeToolOutputPayload(value: unknown): unknown {
  const normalized = normalizeJsonLike(value)
  if (!isObjectValue(normalized)) {
    return normalized
  }

  if ("structuredContent" in normalized) {
    return normalizeToolOutputPayload(normalized.structuredContent)
  }

  const content = normalized.content
  if (Array.isArray(content)) {
    const text = content
      .map((entry) => isObjectValue(entry) && typeof entry.text === "string" ? entry.text : null)
      .find((entry): entry is string => Boolean(entry?.trim()))
    if (text) {
      return normalizeToolOutputPayload(text)
    }
  }

  return normalized
}
