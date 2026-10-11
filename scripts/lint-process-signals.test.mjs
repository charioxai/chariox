// MP-11 enforced signal class regressions; strings are never executed.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { signalViolations, lintProcessSignals, repositorySignalLint } from './lint-process-signals.mjs'

for (const [kind, source] of [
  ['node-group', 'process.kill(-child.pid, "SIGTERM")'],
  ['node-group', 'process . kill ( -(pid), "SIGKILL")'],
  ['node-group', 'process["kill"](-pgid, "SIGTERM")'],
  ['node-group', 'process.kill(processGroupId, "SIGTERM")'],
  ['node-group', 'process.kill(platform === "win32" ? child.pid : -child.pid, "SIGKILL")'],
  ['node-alias', 'const signal = process.kill'],
  ['node-owned-numeric', 'signalOwnedProcessGroup(child.pid, "SIGTERM")'],
  ['node-owned-numeric', 'signalOwnedProcessGroup(this.child.pid, "SIGTERM")'],
  ['node-owned-numeric', 'signalOwnedProcessGroup(pgid, "SIGTERM")'],
  ['node-owned-numeric', 'signalOwnedProcess(pid, "SIGTERM")'],
  ['python-owned-numeric', 'owned_signals.group(child.pid, signal.SIGTERM)'],
  ['python-owned-numeric', 'owned_signals.group(os.getpgrp(), signal.SIGTERM)'],
  ['python-owned-numeric', 'owned.group(os.getpid(), signal.SIGKILL)'],
  ['python-group', 'os.killpg(pgid, signal.SIGTERM)'],
  ['shell-group', 'kill -TERM -- "-$pgid"'],
  ['shell-group', 'kill -9 -123'],
  ['shell-group', 'kill -s SIGKILL -- "-$pgid"'],
  ['name-signal', 'pkill -f provider'],
  ['name-signal', 'killall node'],
]) test(`MP-11 rejects ${kind}: ${source}`, () => {
  assert.ok(signalViolations(source).some(site => site.kind === kind))
})

test('MP-11 shared helper calls and positive existence probes pass', () => {
  assert.deepEqual(signalViolations('signalOwnedProcessGroup(this.child, "SIGTERM"); owned.group(launch, signal.SIGKILL); process.kill(pid, 0)'), [])
})
test('MP-11 explicit allowlist requires exact site, reason and occurrence count', () => {
  const root = mkdtempSync(join(tmpdir(), 'chariox-signal-lint-'))
  const text = 'os.killpg(pgid, signal.SIGTERM)'
  writeFileSync(join(root, 'fixture.py'), text)
  try {
    const entry = { path: 'fixture.py', kind: 'python-group', text, count: 1, reason: 'MP-11 inert scanner regression fixture' }
    const run = sites => lintProcessSignals({ root, files: ['fixture.py'], allowlist: { guards: [], sites } })
    assert.equal(run([]).violationCount, 1)
    assert.equal(run([entry]).violationCount, 0)
    assert.ok(run([{ ...entry, reason: '' }]).violationCount > 0)
    assert.ok(run([{ ...entry, count: 2 }]).violationCount > 0)
    writeFileSync(join(root, 'fixture.py'), text + '\n' + text)
    assert.ok(run([entry]).violationCount > 0)
    writeFileSync(join(root, 'fixture.py'), 'os.killpg(other, signal.SIGTERM)')
    assert.ok(run([entry]).violationCount > 0)
  } finally { rmSync(root, { recursive: true }) }
})
test('MP-11 repository signal lint is enforced in the CI Node test entry', () => {
  const result = repositorySignalLint()
  assert.deepEqual(result.failures, [])
})
