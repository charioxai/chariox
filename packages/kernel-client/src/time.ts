const ABSOLUTE_RFC3339_PATTERN = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/

export function parseAbsoluteInstantMs(value: string): number {
  if (!ABSOLUTE_RFC3339_PATTERN.test(value)) {
    throw new Error("timestamp must be RFC 3339 with an explicit timezone")
  }
  const milliseconds = Date.parse(value)
  if (!Number.isFinite(milliseconds)) {
    throw new Error("timestamp must be a valid RFC 3339 instant")
  }
  return milliseconds
}

export function parseAbsoluteInstantMsOrNull(value: string): number | null {
  try {
    return parseAbsoluteInstantMs(value)
  } catch {
    return null
  }
}

export function canonicalUtcInstant(value: string): string {
  return new Date(parseAbsoluteInstantMs(value)).toISOString()
}
