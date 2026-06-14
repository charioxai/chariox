export function parseDrillIsoTimestamp(value, source) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${source} is missing timestamp`)
  }
  const milliseconds = Date.parse(value)
  if (!Number.isFinite(milliseconds) || new Date(milliseconds).toISOString() !== value) {
    throw new Error(`${source} must be an ISO timestamp`)
  }
  return milliseconds
}

export function validateDrillTimestampOrder({ startedAt, completedAt }, source) {
  const startedMs = parseDrillIsoTimestamp(startedAt, `${source}.startedAt`)
  const completedMs = parseDrillIsoTimestamp(completedAt, `${source}.completedAt`)
  if (completedMs < startedMs) {
    throw new Error(`${source}.completedAt must not be before startedAt`)
  }
}
