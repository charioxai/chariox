// The shrink-only behaviour every counted check shares, driven through the real runner with tracked-state as the example.
import { execFileSync } from 'node:child_process'
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'

const list = entries => `${JSON.stringify(entries)}\n`
const entry = (path, count = 1) => ({ path, count, why: 'before the check' })

function repo(extra = {}) {
  return makeRepo({ 'run/a.log': 'x', 'run/b.log': 'y', 'scripts/checks/tracked-state.allow.json': list([entry('run/a.log'), entry('run/b.log')]), ...extra })
}

test('a list that matches today passes', () => {
  const r = repo()
  assert.equal(r.run('tracked-state').status, 0)
  r.dispose()
})

test('a new violation outside the list is red and names the root fix', () => {
  const r = repo()
  r.write('run/c.log', 'z')
  const result = r.run('tracked-state')
  assert.equal(result.status, 1)
  assert.match(result.text, /run\/c\.log:0: tracked log, recording or database file\. Root fix: delete the file from the repository/)
  r.dispose()
})

test('an entry added to the list in this change is red', () => {
  const r = repo()
  r.write('run/c.log', 'z')
  r.write('scripts/checks/tracked-state.allow.json', list([entry('run/a.log'), entry('run/b.log'), entry('run/c.log')]))
  const result = r.run('tracked-state')
  assert.equal(result.status, 1)
  assert.match(result.text, /run\/c\.log is not in the list at the merge base/)
  r.dispose()
})

test('a raised count is red', () => {
  const r = repo()
  r.write('scripts/checks/tracked-state.allow.json', list([entry('run/a.log', 2), entry('run/b.log')]))
  const result = r.run('tracked-state')
  assert.equal(result.status, 1)
  assert.match(result.text, /run\/a\.log allows 2 but the merge base allows 1/)
  r.dispose()
})

test('a fixed file leaves a stale entry that --shrink deletes', () => {
  const r = repo()
  execFileSync('git', ['rm', '-q', 'run/a.log'], { cwd: r.root })
  const stale = r.run('tracked-state')
  assert.equal(stale.status, 1)
  assert.match(stale.text, /run\/a\.log:0: stale entry, the file no longer matches: delete it from scripts\/checks\/tracked-state\.allow\.json/)
  assert.equal(r.run('tracked-state', { args: ['--shrink'] }).status, 0)
  assert.equal(r.run('tracked-state').status, 0)
  r.dispose()
})

test('CI without a baseline fails instead of letting every entry pass', () => {
  const r = repo()
  assert.throws(() => execFileSync('node', [new URL('./run.mjs', import.meta.url).pathname, '--root', r.root], { env: { ...process.env, CI: 'true', CHARIOX_BOUNDARY_BASE: '' }, stdio: 'pipe' }), error => /CHARIOX_BOUNDARY_BASE/.test(String(error.stderr)))
  r.dispose()
})
