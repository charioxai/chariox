const SENSITIVE_KEY_PATTERN = /token|secret|password|credential|cookie|authorization|api[-_]?key/i

const SECRET_VALUE_PATTERNS = [
  /\bBearer\s+[A-Za-z0-9._~+/=-]{12,}/i,
  /\bsk-[A-Za-z0-9_-]{16,}\b/,
  /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{16,}\b/,
]

export function isSensitiveDrillKey(key) {
  return SENSITIVE_KEY_PATTERN.test(String(key ?? ""))
}

export function looksLikeDrillSecretValue(value) {
  return typeof value === "string" && SECRET_VALUE_PATTERNS.some((pattern) => pattern.test(value))
}

export function sanitizeDrillMetadata(value, key = "") {
  if (isSensitiveDrillKey(key)) return "<redacted>"
  if (typeof value === "string") {
    return looksLikeDrillSecretValue(value) ? "<redacted>" : value
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return value
  if (Array.isArray(value)) return value.map((item) => sanitizeDrillMetadata(item, key))
  if (!value || typeof value !== "object") return null

  const sanitized = {}
  for (const [childKey, childValue] of Object.entries(value)) {
    sanitized[childKey] = sanitizeDrillMetadata(childValue, childKey)
  }
  return sanitized
}
