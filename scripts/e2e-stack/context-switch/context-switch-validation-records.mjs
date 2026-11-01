// MP-08 / MP-10 / MP-11: score only the response to the current real-client probe.
// An idle badge and end marker can precede the client's final history patch.
export class ContextValidationTurnCompletion {
  #answer
  #idleSince
  #sawWorking = false
  get sawWorking() { return this.#sawWorking }
  reset() { this.#idleSince = undefined }
  observe({ busy, answer, now }) {
    if (busy) this.#sawWorking = true
    if (busy || answer !== this.#answer) this.reset()
    this.#answer = answer
    if (!busy) this.#idleSince ??= now
    return !busy && now - this.#idleSince >= 5000
  }
}

export function contextValidationTurnEntries(entries, marker) {
  const users = entries.flatMap((entry, index) =>
    entry.role === 'user' && entry.text?.includes(marker) ? [index] : [],
  )
  if (users.length !== 1) return { userCount: users.length, entries: [] }
  const start = users[0] + 1
  const end = entries.findIndex((entry, index) => index >= start && entry.role === 'user')
  return { userCount: 1, entries: entries.slice(start, end < 0 ? entries.length : end) }
}

export function scoreContextValidationRecall(entries, marker, expected) {
  const turn = contextValidationTurnEntries(entries, marker)
  const answer = turn.entries.filter(entry => entry.role === 'assistant')
    .map(entry => entry.text ?? '').join('\n')
  let observed = {}
  for (const match of answer.matchAll(/\{[^{}]*\}/gs)) {
    try {
      const value = JSON.parse(match[0])
      if (Object.keys(expected).every(key => Object.hasOwn(value, key))) observed = value
    } catch { /* A partial streamed object is not a recall answer. */ }
  }
  const checks = Object.fromEntries(Object.entries(expected).map(([key, value]) =>
    [key, typeof observed[key] === 'string' && observed[key].normalize('NFC') === value.normalize('NFC')],
  ))
  return {
    userCount: turn.userCount,
    toolCount: turn.entries.filter(entry => entry.role === 'tool').length,
    observed,
    checks,
    score: Object.values(checks).filter(Boolean).length,
    total: Object.keys(expected).length,
  }
}
