import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { makeRepo } from '../lib/check-fixture.mjs'
import check from './commit-message-rules.mjs'

const runner = fileURLToPath(new URL('./run.mjs', import.meta.url))
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

// The shape of the pull-request CI checkout: HEAD is a synthetic merge commit, the clone has depth 1 and the base tip is
// fetched at depth 1. The merge is excluded by --no-merges and its parents are missing, so nothing would be inspected.
const git = (cwd, ...args) => execFileSync('git', ['-c', 'user.name=t', '-c', 'user.email=t@example.test', ...args], { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
const shallowPullRequest = message => {
  const r = repo()
  const base = r.base
  git(r.root, 'checkout', '-q', '-b', 'topic')
  commit(r, message)
  git(r.root, 'checkout', '-q', 'main')
  r.write('apps/b.ts', 'main moved\n')
  const tip = r.commit('Main moves on')
  git(r.root, 'merge', '-q', '--no-ff', '-m', 'Merge topic', 'topic')
  const clone = mkdtempSync(join(tmpdir(), 'rule-check-shallow-'))
  git(tmpdir(), 'clone', '-q', '--depth=1', `file://${r.root}`, clone)
  git(clone, 'fetch', '-q', '--no-tags', '--depth=1', 'origin', tip)
  const run = () => {
    try {
      return { status: 0, text: execFileSync('node', [runner, '--root', clone, '--only', check.name], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], env: { ...process.env, CHARIOX_BOUNDARY_BASE: tip, CI: '' } }) }
    } catch (error) { return { status: error.status, text: `${error.stdout}${error.stderr}` } }
  }
  return { r, clone, base, run, dispose: () => { r.dispose(); rmSync(clone, { recursive: true, force: true }) } }
}

test('a shallow checkout that cannot reach the branch commits is red with the fix, never a silent pass', () => {
  const s = shallowPullRequest('Fix the wake order\n\nCo-Authored-By: Some Agent <noreply@example.test>')
  const result = s.run()
  assert.equal(result.status, 1)
  assert.match(result.text, /commit-message-rules: .*shallow/)
  assert.match(result.text, /fetch-depth: 0|git fetch --unshallow/)
  s.dispose()
})

test('the same pull-request checkout with its history fetched inspects the branch commits', () => {
  const s = shallowPullRequest('Fix the wake order\n\nCo-Authored-By: Some Agent <noreply@example.test>')
  git(s.clone, 'fetch', '-q', '--no-tags', '--unshallow', 'origin')
  const result = s.run()
  assert.equal(result.status, 1)
  assert.match(result.text, /carries a Co-Authored-By line/)
  s.dispose()
})

test('a base that shares no history with HEAD is red, not an empty range', () => {
  const r = repo()
  const other = makeRepo({ 'apps/z.ts': 'z\n' })
  git(r.root, 'fetch', '-q', other.root, 'main')
  const foreign = git(r.root, 'rev-parse', 'FETCH_HEAD')
  const result = r.run(check.name, { base: foreign })
  assert.equal(result.status, 1)
  assert.match(result.text, /no common history/)
  r.dispose()
  other.dispose()
})
