import { looksLikeDrillSecretValue } from "./drill-secrets.mjs"

export function countDrillAggregateNextAction(counts, { owner, classification, nextAction, count = 1 }) {
  validateDrillAggregateNextAction({
    owner,
    classification,
    nextAction,
    count,
  }, "aggregate next action")
  const key = JSON.stringify([owner, classification, nextAction])
  const previous = counts.get(key)
  const increment = nextActionIncrement(count)
  counts.set(key, previous
    ? { ...previous, count: previous.count + increment }
    : { owner, classification, nextAction, count: increment })
}

export function formatDrillAggregateNextActionCounts(counts) {
  return [...counts.values()].sort((left, right) => (
    right.count - left.count
    || left.owner.localeCompare(right.owner)
    || left.classification.localeCompare(right.classification)
    || left.nextAction.localeCompare(right.nextAction)
  ))
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
