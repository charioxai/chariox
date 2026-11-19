import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './no-polling-where-subscription.mjs'

const FILE = 'packages/kernel-client/src/room-view.ts'
const clean = 'export function watch(client) {\n  client.onEvent(event => { if (event.SessionSnapshot) void client.send({ GetSessionState: { session_id: id } }) })\n  const heartbeat = setInterval(() => client.ping(), 15_000)\n}\n'
const red = (text, expected) => {
  const r = makeRepo({ [FILE]: clean })
  assert.equal(r.run(check.name).status, 0, 'the clean tree passes')
  r.write(FILE, text)
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`))
  r.dispose()
}

test('a timer that re-reads a pushed read is red', () => {
  red('export function watch(client) {\n  setInterval(() => {\n    void client.send({ GetSessionState: { session_id: id } })\n  }, 1000)\n}\n', /room-view\.ts:3: setInterval re-reads GetSessionState, which the kernel pushes as an event/)
})

test('a loop that sleeps between reads is red', () => {
  red('export async function watch(client) {\n  while (!stopped) {\n    await client.listSessions()\n    await delay(2000)\n  }\n}\n', /room-view\.ts:3: a loop that sleeps re-reads listSessions/)
})

test('a timer that reads through a function of the same file is red', () => {
  red('async function refresh(client) {\n  outline = await client.send({ GetSessionHistoryOutline: { session_id: id } })\n}\nexport function watch(client) {\n  const tick = setInterval(() => { void refresh(client) }, 1000)\n}\n', /room-view\.ts:5: setInterval re-reads GetSessionHistoryOutline \(through refresh\)/)
})

test('a timer given a reading function of the same file by name is red', () => {
  red('async function refresh() {\n  state = await client.send({ GetSessionState: { session_id: id } })\n}\nexport function watch() {\n  setInterval(refresh, 1000)\n}\n', /room-view\.ts:5: setInterval re-reads GetSessionState \(through refresh\)/)
})

test('a timer or helper that only handles a reading function, without running it, passes', () => {
  const r = makeRepo({
    [FILE]: 'function refresh() {\n  return client.getSessionState(id)\n}\nfunction forget() {\n  pending.delete(refresh)\n}\nexport function watch() {\n  setInterval(() => pending.delete(refresh), 1000)\n  setInterval(() => forget(), 1000)\n}\n',
  })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a read in a comment or string, and a test file, pass', () => {
  const r = makeRepo({
    [FILE]: 'export function watch(client) {\n  setInterval(() => {\n    // not GetSessionState\n    log("ListSessions")\n  }, 1000)\n}\n',
    'packages/kernel-client/src/room-view.test.ts': 'setInterval(() => client.send({ ListAgents: {} }), 10)\n',
  })
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})
