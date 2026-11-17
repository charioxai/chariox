import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './commit-message-rules.mjs'

const repo = () => makeRepo({ 'apps/a.ts': 'one\n' })
const commit = (r, message) => { r.write('apps/a.ts', `${message.length}\n`); return r.commit(message) }
const planted = (r, expected) => {
  const result = r.run(check.name)
  assert.equal(result.status, 1)
  assert.match(result.text, expected)
  assert.ok(result.text.includes(`Root fix: ${check.rootFix}`), 'names the root fix')
  r.dispose()
}

test('a branch whose commits are plain summaries passes', () => {
  const r = repo()
  commit(r, 'Show the inbox count in the header [skip ci]')
  commit(r, 'Keep the Claude provider run alive across a restart\n\nThe kernel restores the run from its state file.')
  assert.equal(r.run(check.name).status, 0)
  r.dispose()
})

test('a Co-Authored-By trailer is red', () => {
  const r = repo()
  commit(r, 'Fix the wake order [skip ci]\n\nCo-Authored-By: Some Agent <noreply@example.test>')
  planted(r, /commit [0-9a-f]{10}:0: message "Fix the wake order \[skip ci\]" carries a Co-Authored-By line/)
})

test('a generated-with footer and an agent noreply address are red', () => {
  for (const [footer, name] of [
    ['Generated with [Claude Code](https://claude.com/claude-code)', 'a "generated with" attribution line'],
    ['\u{1F916} Generated with a tool', 'a robot "generated" footer'],
    ['Signed-off-by: Bot <noreply@anthropic.com>', 'an agent noreply address'],
  ]) {
    const r = repo()
    commit(r, `Fix the wake order\n\n${footer}`)
    planted(r, new RegExp(`carries ${name.replace(/[()"]/g, '\\$&')}`))
  }
})

test('only commits after the merge base count', () => {
  const r = makeRepo({ 'apps/a.ts': 'one\n' })
  r.write('apps/a.ts', 'two\n')
  r.commit('Old change\n\nCo-Authored-By: Someone <a@example.test>')
  assert.equal(r.run(check.name).status, 1)
  r.write('apps/a.ts', 'three\n')
  const head = r.commit('Clean change [skip ci]')
  assert.equal(r.run(check.name, { base: head }).status, 0, 'a commit already at the merge base is history, not part of the change')
  r.dispose()
})
