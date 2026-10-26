// MP-11 #868: execute migrated callers, including their surviving descendants.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { once } from 'node:events'
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnOwned, signalOwnedProcessGroup, processSnapshot } from '../apps/kernel/slice-linux-docker/owned-process-signals.mjs'
import { CodexAppServerRun } from '../experiments/mcp-isolation-spike/src/codex-driver.mjs'

const pause = ms => new Promise(resolve => setTimeout(resolve, ms))
async function waitFor(predicate) {
  const deadline = Date.now() + 5_000
  while (Date.now() < deadline) {
    if (await predicate()) return
    await pause(25)
  }
  assert.fail('MP-11 caller fixture did not settle within 5 seconds')
}
const linux = { skip: process.platform !== 'linux', timeout: 15_000 }

test('MP-11 embedded keyboard cancellation kills active typing rather than failing guard admission', linux, async () => {
  const root = await mkdtemp(join(tmpdir(), 'chariox-mp11-keyboard-'))
  let child, closed
  try {
    const source = await readFile(new URL('../apps/cli/scripts/live-computer-keyboard-x11-drill.mjs', import.meta.url), 'utf8')
    const embedded = source.match(/const typing = exec\(\[.*?`(import os,sys,signal,subprocess,time[\s\S]*?sys.exit\(child.returncode\))`\]/)
    assert.ok(embedded, 'MP-11 exercise the actual embedded keyboard cancellation wrapper')
    const signalsDir = fileURLToPath(new URL('../apps/kernel/slice-linux-docker/', import.meta.url))
    const wrapper = embedded[1].replace("sys.path.insert(0, '/tmp/chariox-keyboard-x11')", `sys.path.insert(0, ${JSON.stringify(signalsDir)})`)
      .replaceAll('/tmp/chariox-keyboard-x11', root)
    const pulses = join(root, 'pulses')
    await writeFile(join(root, 'slice-screen.sh'), `#!/usr/bin/env python3
import os,time
from pathlib import Path
root=Path(${JSON.stringify(root)})
(root/'typing.pid').write_text(str(os.getpid()))
while True:
    with (root/'pulses').open('a') as output: output.write('x')
    time.sleep(0.025)
`, { mode: 0o700 })
    child = spawnOwned('python3', ['-B', '-c', wrapper], { detached: true, stdio: ['ignore', 'ignore', 'pipe'] })
    const exit = once(child, 'exit')
    closed = once(child, 'close')
    let stderr = ''
    child.stderr.on('data', chunk => { stderr += chunk })
    await waitFor(async () => (await readFile(pulses, 'utf8').catch(() => '')).length >= 4)
    const typingPid = Number(await readFile(join(root, 'typing.pid'), 'utf8'))
    await writeFile(join(root, 'input-cancel'), '')
    await waitFor(() => child.exitCode !== null || child.signalCode !== null)
    const [code, signal] = await exit
    assert.equal(stderr, '', 'MP-11 cancellation must not throw a numeric-target guard error')
    assert.equal(code, null)
    assert.equal(signal, 'SIGKILL', 'MP-11 the wrapper and typing child must be group-cancelled')
    await waitFor(() => !processSnapshot().some(row => row.pid === typingPid && row.state !== 'Z'))
    const stopped = await readFile(pulses, 'utf8')
    await pause(200)
    assert.equal(await readFile(pulses, 'utf8'), stopped, 'MP-11 typing must stay stopped after cancellation')
  } finally {
    if (child) { signalOwnedProcessGroup(child, 'SIGKILL'); await closed }
    await rm(root, { recursive: true, force: true })
  }
})

for (const leaderExit of ['signal', 'success']) test(`MP-11 Codex experiment stop force-settles a provider descendant after ${leaderExit} leader exit`, linux, async () => {
  const root = await mkdtemp(join(tmpdir(), 'chariox-mp11-codex-stop-'))
  let child, closed
  try {
    const pidPath = join(root, 'provider.pid')
    // The leader exits on TERM; the provider keeps its inherited pipes and group.
    const descendant = `process.on('SIGTERM', () => {}); require('node:fs').writeFileSync(${JSON.stringify(pidPath)}, String(process.pid)); setInterval(() => {}, 1000)`
    const exitHandler = leaderExit === 'success' ? "process.on('SIGTERM', () => process.exit(0));" : ''
    child = spawnOwned(process.execPath, ['-e', `${exitHandler} require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(descendant)}], { stdio: ['ignore', 'pipe', 'pipe'] }); setInterval(() => {}, 1000)`], { detached: true, stdio: 'ignore' })
    closed = once(child, 'close')
    await waitFor(async () => Boolean(await readFile(pidPath, 'utf8').catch(() => '')))
    const providerPid = Number(await readFile(pidPath, 'utf8'))
    const run = new CodexAppServerRun({ child })
    await run.stop()
    await waitFor(() => !processSnapshot().some(row => row.pid === providerPid && row.state !== 'Z'))
    await closed
  } finally {
    if (child) { signalOwnedProcessGroup(child, 'SIGKILL'); await closed }
    await rm(root, { recursive: true, force: true })
  }
})
