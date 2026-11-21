// The shrink-only behaviour every counted check shares, driven through the real runner with tracked-state as the example.
import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
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

test('the runner fetches missing origin/main and uses its merge base with the topic', t => {
  const upstream = repo()
  t.after(upstream.dispose)
  upstream.write('run/c.log', 'accepted upstream')
  upstream.write('scripts/checks/tracked-state.allow.json', list([entry('run/a.log'), entry('run/b.log'), entry('run/c.log')]))
  const merged = upstream.commit('main accepts a new entry')
  upstream.git('switch', '-c', 'topic')
  upstream.write('topic.txt', 'topic change')
  upstream.commit('topic change')
  upstream.git('switch', 'main')
  upstream.write('main.txt', 'main advances independently')
  const latest = upstream.commit('advance main')

  const clone = mkdtempSync(join(tmpdir(), 'rule-check-lint-base-'))
  t.after(() => rmSync(clone, { recursive: true, force: true }))
  const git = (...args) => execFileSync('git', args, { cwd: clone, encoding: 'utf8', stdio: 'pipe' }).trim()
  git('clone', '-q', '--single-branch', '--branch', 'topic', upstream.root, '.')
  git('branch', 'main', upstream.base)
  const run = (base = '') => {
    try {
      const text = execFileSync(process.execPath, [new URL('./run.mjs', import.meta.url).pathname, '--root', clone, '--only', 'tracked-state'], {
        env: { ...process.env, CI: 'true', CHARIOX_BOUNDARY_BASE: base },
        encoding: 'utf8',
        stdio: 'pipe',
      })
      return { status: 0, text }
    } catch (error) {
      return { status: error.status, text: error.stdout + error.stderr }
    }
  }

  assert.throws(() => git('rev-parse', '--verify', 'refs/remotes/origin/main'))
  const explicit = run('refs/heads/main')
  assert.equal(explicit.status, 1, explicit.text)
  assert.ok(explicit.text.includes(`lint:rules: baseline ${upstream.base} (CHARIOX_BOUNDARY_BASE)`))
  assert.match(explicit.text, /run\/c\.log is not in the list at the merge base/)
  assert.throws(() => git('rev-parse', '--verify', 'refs/remotes/origin/main'))

  const implicit = run()
  assert.equal(implicit.status, 0, implicit.text)
  assert.equal(git('rev-parse', 'refs/remotes/origin/main'), latest)
  assert.match(implicit.text, new RegExp(`lint:rules: baseline ${merged} \\(merge base with origin/main\\)`))
  assert.equal(git('rev-parse', 'refs/heads/main'), upstream.base)
  assert.match(run().text, new RegExp(`lint:rules: baseline ${merged}`))
})
