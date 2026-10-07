// MP-11 #868 regressions exercise ESM linking and the default captured-group guard.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'
import { fileURLToPath } from 'node:url'
import { registerCapturedProcessGroup } from "../apps/kernel/slice-linux-docker/owned-process-signals.mjs"
import * as runtime from '../apps/cli/scripts/lib/browser-computer-soak-runtime.mjs'

test('MP-11 migrated memory soak entry links without launching a soak', () => {
  const script = fileURLToPath(new URL('../apps/cli/scripts/live-leased-home-memory-soak.mjs', import.meta.url))
  const result = spawnSync(process.execPath, [script, '--help'], { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /Usage:/)
})

test('MP-11 captured external launch cleanup uses the real default signal guard', { skip: process.platform !== 'linux' }, async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { detached: true, stdio: 'ignore' })
  const exit = once(child, 'exit')
  try {
    const [expected] = await runtime.captureOwnedIdentities([['viewer', child.pid]])
    expected.signalIdentity = registerCapturedProcessGroup(expected)
    const result = await runtime.terminateCapturedProcessGroup('viewer', expected)
    assert.equal(result.ok, true, JSON.stringify(result))
    await exit
  } finally {
    // Direct ChildProcess handle, solely this test's child, never a numeric/group target.
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    await exit
  }
})

test('MP-11 foreground runner accounting accepts its inherited PGID without signal registration', { skip: process.platform !== 'linux' }, async () => {
  const child = spawn(process.execPath, ['-e', 'setInterval(()=>{},1000)'], { stdio: 'ignore' })
  const exit = once(child, 'exit')
  try {
    const observed = await runtime.processIdentity(child.pid)
    assert.notEqual(observed.processGroupId, child.pid)
    const [entry] = await runtime.captureOwnedIdentities([['runner', child.pid]])
    assert.equal(entry.pid, child.pid)
    assert.equal(entry.signalIdentity, undefined)
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    await exit
  }
})
