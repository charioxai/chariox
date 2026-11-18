import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check, { READ_PATHS } from './no-full-snapshot-on-read-path.mjs'

const [HISTORY, READS] = READ_PATHS
const clean = {
  [HISTORY]: 'async fn outline(state: &State, id: &str) {\n    let session = state.session_with_agents(id)?;\n}\n',
  [READS]: 'fn state(state: &State) {}\n',
  'apps/kernel/src/runtime/state/prompt.rs': 'async fn submit(state: &State, id: &str) {\n    let session = state.session_snapshot(id).await?;\n}\n',
}

test('a read path that builds a full Room snapshot is red; a write path, a comment and a test are not', () => {
  const r = makeRepo({ ...clean, [READS]: 'fn state() {}\n// state.session_snapshot(id)\n#[cfg(test)]\nmod tests {\n    fn t(state: &State) { state.session_snapshot("id"); }\n}\n' })
  assert.equal(r.run(check.name).status, 0, 'the clean tree passes')
  r.write(HISTORY, 'async fn outline(state: &State, id: &str) {\n    let session = state\n        .session_snapshot(id)\n        .await?;\n}\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /history_executor\.rs:3: builds a full Room snapshot on a read path: session_snapshot\(\)/)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`))
  r.dispose()
})

test('every snapshot builder counts: a list of snapshots on a read path is red', () => {
  const r = makeRepo(clean)
  r.write(READS, 'fn list(state: &State) {\n    let all = state.list_session_snapshots();\n}\n')
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /session_read_control\.rs:2: builds a full Room snapshot on a read path: list_session_snapshots\(\)/)
  r.dispose()
})

test('a read-path file that moved is red until the table names it', () => {
  const r = makeRepo({ [READS]: clean[READS] })
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, /history_executor\.rs:1: read-path file named in no-full-snapshot-on-read-path\.mjs no longer exists/)
  r.dispose()
})
