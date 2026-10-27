// MP-10/MP-11: distinguish the measured paired TUI from other real clients.
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'

export function terminalSessionEvents(lines, subject, window) {
  assert(typeof subject === 'string' && subject.length > 0, 'MP-10 paired TUI subject required')
  assert(Number.isFinite(window?.startedAtMs) && window.finishedAtMs >= window.startedAtMs, 'MP-10 completed TUI window required')
  const admissions = [], closures = [], otherClientClosures = []
  for (const line of lines) {
    let event
    try { event = JSON.parse(line) } catch { continue }
    if (!['local browser session admitted', 'local browser session closed'].includes(event.message)) continue
    if (event.timestamp_ms > window.finishedAtMs) continue
    if (event.message === 'local browser session admitted') {
      if (event.subject === subject) admissions.push({ at: event.timestamp_ms })
      continue
    }
    if (event.timestamp_ms < window.startedAtMs) continue
    assert(typeof event.subject === 'string' && event.subject.length > 0, 'MP-10 session closure identity missing')
    const observation = { at: event.timestamp_ms, reason: event.reason }
    if (event.subject === subject) closures.push(observation)
    else otherClientClosures.push({ ...observation, subjectSha256: createHash('sha256').update(event.subject).digest('hex') })
  }
  assert(admissions.length > 0, 'MP-10 no kernel admission for the paired serving TUI')
  return { admissions, closures, otherClientClosures }
}
