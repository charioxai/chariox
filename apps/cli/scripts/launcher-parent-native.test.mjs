// Exercise Bun's real parent metadata after an owned launcher exits.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import test from 'node:test'
import { createOwnedSignalGuard } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

test('Bun parent watch settles after its real launcher is killed', { timeout: 10000 }, async () => {
  const module = new URL('../dist/cli-launcher-parent.js', import.meta.url).href
  const entry = `;(async () => {
    const { watchCliLauncherParent } = await import(${JSON.stringify(module)})
    watchCliLauncherParent(process.env.CHARIOX_CLI_LAUNCHER_PID)
    setInterval(() => {}, 1000)
    console.log(JSON.stringify({ pid: process.pid, ppid: process.ppid }))
  })()`
  const parentEntry = `
    const { spawn } = require('node:child_process')
    spawn(${JSON.stringify(process.env.BUN_BIN ?? 'bun')}, ['-e', ${JSON.stringify(entry)}], {
      stdio: ['ignore', 'inherit', 'inherit'],
      env: { ...process.env, CHARIOX_CLI_LAUNCHER_PID: String(process.pid) },
    })
    setInterval(() => {}, 1000)
  `
  const parent = spawn(process.execPath, ['-e', parentEntry], { stdio: ['ignore', 'pipe', 'pipe'] })
  const guard = createOwnedSignalGuard()
  const parentIdentity = guard.record(parent.pid, { freshLaunch: true })
  const parentExited = once(parent, 'exit')
  const parentClosed = once(parent, 'close')
  let childIdentity
  let stderr = ''
  parent.stderr.on('data', chunk => { stderr += chunk })
  try {
    let output = ''
    const ready = new Promise((resolve, reject) => {
      parent.stdout.on('data', chunk => {
        output += chunk
        if (output.includes('\n')) resolve(JSON.parse(output.split('\n')[0]))
      })
      parent.on('error', reject)
    })
    const child = await Promise.race([ready, delay(3000).then(() => { throw new Error(`Bun child did not start: ${stderr}`) })])
    assert.equal(child.ppid, parent.pid)
    childIdentity = guard.record(child.pid, { freshLaunch: true })
    guard.pid(parentIdentity, 'SIGKILL')
    await parentExited
    const deadline = Date.now() + 2000
    while (Date.now() < deadline && guard.refresh().some(row => row.pid === child.pid)) await delay(25)
    assert.equal(guard.refresh().some(row => row.pid === child.pid), false,
      'Bun child survived its launcher by more than 2 seconds')
  } finally {
    for (const identity of guard.processes(parentIdentity)) guard.pid(identity, 'SIGKILL')
    guard.pid(parentIdentity, 'SIGKILL')
    if (childIdentity) guard.pid(childIdentity, 'SIGKILL')
    await parentExited
    await parentClosed
  }
})
