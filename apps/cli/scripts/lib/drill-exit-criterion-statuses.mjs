export const DRILL_EXIT_CRITERION_STATUSES = Object.freeze(["satisfied", "failed", "skipped", "dry-run"])

export function isKnownDrillExitCriterionStatus(status) {
  return DRILL_EXIT_CRITERION_STATUSES.includes(status)
}

export function validateDrillExitCriterionStatus(status, source, { message } = {}) {
  if (!isKnownDrillExitCriterionStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has unknown exit criterion status ${JSON.stringify(status)}`)
  }
}
