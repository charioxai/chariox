export const DRILL_OUTCOME_STATUSES = Object.freeze(["passed", "failed", "skipped", "dry-run"])

export function isKnownDrillOutcomeStatus(status) {
  return DRILL_OUTCOME_STATUSES.includes(status)
}

export function validateDrillOutcomeStatus(status, source, { message } = {}) {
  if (!isKnownDrillOutcomeStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
  }
}
