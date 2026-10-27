import {
  DRILL_OUTCOME_STATUSES,
  isKnownDrillOutcomeStatus,
  validateDrillOutcomeStatus,
} from "./drill-outcome-statuses.mjs"

export const DRILL_MATRIX_REPORT_STATUSES = Object.freeze(["passed", "failed", "dry-run"])
export const DRILL_MATRIX_SCENARIO_STATUSES = DRILL_OUTCOME_STATUSES

export function isKnownDrillMatrixReportStatus(status) {
  return DRILL_MATRIX_REPORT_STATUSES.includes(status)
}

export function validateDrillMatrixReportStatus(status, source, { message } = {}) {
  if (!isKnownDrillMatrixReportStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
  }
}

export function isKnownDrillMatrixScenarioStatus(status) {
  return isKnownDrillOutcomeStatus(status)
}

export function validateDrillMatrixScenarioStatus(status, source, { message } = {}) {
  validateDrillOutcomeStatus(status, source, { message })
}
