export const DRILL_VALIDATION_RESULT_STATUSES = Object.freeze(["passed", "failed"])
export const DRILL_VALIDATION_CHECK_STATUSES = Object.freeze(["passed", "failed", "skipped"])

export function isKnownDrillValidationResultStatus(status) {
  return DRILL_VALIDATION_RESULT_STATUSES.includes(status)
}

export function validateDrillValidationResultStatus(status, source, { message } = {}) {
  if (!isKnownDrillValidationResultStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
  }
}

export function isKnownDrillValidationCheckStatus(status) {
  return DRILL_VALIDATION_CHECK_STATUSES.includes(status)
}

export function validateDrillValidationCheckStatus(status, source, { message } = {}) {
  if (!isKnownDrillValidationCheckStatus(status)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(status) : message)
    }
    throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
  }
}
