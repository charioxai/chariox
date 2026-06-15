export const DRILL_MATRIX_REPORT_STATUSES = Object.freeze(["passed", "failed", "dry-run"])
export const DRILL_MATRIX_SCENARIO_STATUSES = Object.freeze(["passed", "failed", "skipped", "dry-run"])

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
  return DRILL_MATRIX_SCENARIO_STATUSES.includes(status)
}

export function validateDrillMatrixScenarioStatus(status, source, { message } = {}) {
  if (!isKnownDrillMatrixScenarioStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
  }
}
