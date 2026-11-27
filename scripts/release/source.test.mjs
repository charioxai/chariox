import assert from 'node:assert/strict'
import test from 'node:test'
import { stagingRef, validateSources, requireMainRelease } from './source.mjs'

const commit = 'a'.repeat(40)
const sources = ref => Object.fromEntries(['cloud', 'oss', 'relay'].map(name => [name, { ref, commit }]))
test('MP-08 / MP-09 / MP-10 / MP-11: only main and staging union refs are admitted', () => {
  for (const ref of ['main', 'stagenew/web', 'staging/release-1', 'refs/heads/stagenew/kernel']) assert.ok(stagingRef(ref).startsWith('refs/heads/'))
  for (const ref of ['feature/test', 'main~1', commit, 'refs/tags/main', 'stagenew/', 'stagenew/../main', 'staging/a.lock', 'staging/a@{b', 'staging/a\nmain']) assert.throws(() => stagingRef(ref))
})
test('MP-08 / MP-09 / MP-10 / MP-11: branch staging can never become a production release', () => {
  const main = sources('refs/heads/main')
  validateSources(main, { cloud: { commit }, oss: { commit }, relay: { commit } })
  requireMainRelease({ source: main })
  for (const name of ['cloud', 'oss', 'relay']) {
    const branch = structuredClone(main); branch[name].ref = 'refs/heads/stagenew/union'
    assert.throws(() => requireMainRelease({ source: branch }), /main release/)
  }
  assert.throws(() => requireMainRelease({}), /main release/)
  assert.throws(() => validateSources({ ...main, cloud: { ref: 'refs/heads/main', commit: 'b'.repeat(40) } }, { cloud: { commit }, oss: { commit }, relay: { commit } }), /identity/)
})
