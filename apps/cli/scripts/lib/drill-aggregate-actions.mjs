import { looksLikeDrillSecretValue } from "./drill-secrets.mjs"

export function countDrillAggregateNextAction(counts, { owner, classification, nextAction, count = 1, sourceDetails = [] }) {
  const hasSourceDetails = sourceDetails !== undefined && sourceDetails !== null
  const details = hasSourceDetails ? sourceDetails : []
  validateDrillAggregateNextAction({
    owner,
    classification,
    nextAction,
    count,
    ...(hasSourceDetails ? { sourceDetails: details } : {}),
  }, "aggregate next action")
  const key = JSON.stringify([owner, classification, nextAction])
  const previous = counts.get(key)
  const increment = nextActionIncrement(count)
  if (previous) {
    counts.set(key, {
      ...previous,
      count: previous.count + increment,
      ...mergedSourceDetails(previous.sourceDetails, Array.isArray(details) ? details : []),
    })
  } else {
    counts.set(key, {
      owner,
      classification,
      nextAction,
      count: increment,
      ...mergedSourceDetails([], Array.isArray(details) ? details : []),
    })
  }
}

export function formatDrillAggregateNextActionCounts(counts) {
  return [...counts.values()].sort((left, right) => (
    right.count - left.count
    || left.owner.localeCompare(right.owner)
    || left.classification.localeCompare(right.classification)
    || left.nextAction.localeCompare(right.nextAction)
  ))
}

export function formatDrillAggregateNextActionSourceDetails(sourceDetails) {
  if (!Array.isArray(sourceDetails) || sourceDetails.length === 0) return ""
  return sourceDetails
    .map((detail) => {
      const source = detail.source ?? [detail.matrix, detail.scenarioId].filter(Boolean).join("/")
      const report = detail.reportPath ? ` report=${detail.reportPath}` : ""
      return `${source}${report}`
    })
    .join(", ")
}

export function validateDrillAggregateNextAction(action, source) {
  if (!action || typeof action !== "object" || Array.isArray(action)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["owner", "classification", "nextAction"]) {
    if (!nonEmptyString(action[key])) {
      throw new Error(`${source} is missing ${key}`)
    }
    if (looksLikeDrillSecretValue(action[key])) {
      throw new Error(`${source} includes secret-looking ${key}`)
    }
  }
  if (!Number.isSafeInteger(action.count) || action.count < 1) {
    throw new Error(`${source} has invalid count`)
  }
  if (action.sourceDetails !== undefined) {
    if (!Array.isArray(action.sourceDetails)) {
      throw new Error(`${source} has invalid sourceDetails`)
    }
    for (const [index, detail] of action.sourceDetails.entries()) {
      validateDrillAggregateNextActionSourceDetail(detail, `${source}.sourceDetails[${index}]`)
    }
  }
}

export function countDrillAggregateEntriesBy(entries, keyForEntry) {
  const counts = new Map()
  for (const entry of entries) {
    const key = keyForEntry(entry)
    counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function nextActionIncrement(count) {
  if (!Number.isSafeInteger(count) || count < 1) {
    throw new Error("aggregate next action has invalid count")
  }
  return count
}

function mergedSourceDetails(previous = [], next = []) {
  const details = new Map()
  for (const detail of [...previous, ...next]) {
    if (!detail || typeof detail !== "object" || Array.isArray(detail)) continue
    const normalized = normalizeSourceDetail(detail)
    details.set(JSON.stringify(normalized), normalized)
  }
  const sourceDetails = [...details.values()].sort(compareSourceDetails)
  return sourceDetails.length > 0 ? { sourceDetails } : {}
}

function validateDrillAggregateNextActionSourceDetail(detail, source) {
  if (!detail || typeof detail !== "object" || Array.isArray(detail)) {
    throw new Error(`${source} is not an object`)
  }
  const keys = ["source", "matrix", "scenarioId", "reportPath"]
  if (!keys.some((key) => nonEmptyString(detail[key]))) {
    throw new Error(`${source} is missing source identity`)
  }
  for (const key of keys) {
    if (detail[key] === undefined || detail[key] === null) continue
    if (!nonEmptyString(detail[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
    if (looksLikeDrillSecretValue(detail[key])) {
      throw new Error(`${source}.${key} includes secret-looking diagnostic text`)
    }
  }
}

function normalizeSourceDetail(detail) {
  return {
    ...(nonEmptyString(detail.source) ? { source: detail.source } : {}),
    ...(nonEmptyString(detail.matrix) ? { matrix: detail.matrix } : {}),
    ...(nonEmptyString(detail.scenarioId) ? { scenarioId: detail.scenarioId } : {}),
    ...(nonEmptyString(detail.reportPath) ? { reportPath: detail.reportPath } : {}),
  }
}

function compareSourceDetails(left, right) {
  return JSON.stringify(left).localeCompare(JSON.stringify(right))
}
