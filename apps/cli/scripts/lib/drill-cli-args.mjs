export function parseDrillMaxDepth(value) {
  const parsed = Number(value)
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error("--max-depth must be a non-negative integer")
  }
  return parsed
}

export function parseDrillNonNegativeInteger(value, flag) {
  const parsed = Number(value)
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${flag} must be a non-negative integer`)
  }
  return parsed
}
