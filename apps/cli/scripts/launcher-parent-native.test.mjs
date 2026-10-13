// MP-08/MP-11: exercise the macOS fallback with real Bun processes on any host.
// Linux Rust PR_SET_PDEATHSIG is exercised by the native launcher/live drill.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import test from 'node:test'
import { createOwnedSignalGuard } from '../../kernel/slice-linux-docker/owned-process-signals.mjs'

const delay = ms => new Promise(resolve => setTimeout(resolve, ms))

for (const wrapped of [false, true]) test(`Bun ${wrapped ? 'Linux wrapped child' : 'macOS fallback'} settles after its launcher is killed`, { timeout: 10000 }, async () => {
  const module = new URL('../dist/cli-launcher-parent.js', import.meta.url).href
  const entry = `;(async () => {
    const { watchCliLauncherParent } = await import(${JSON.stringify(module)})
    watchCliLauncherParent(process.env.CHARIOX_CLI_LAUNCHER_PID, { platform: ${JSON.stringify(wrapped ? "linux" : "darwin")} })
    setInterval(() => {}, 1000)
    console.log(JSON.stringify({ pid: process.pid, ppid: process.ppid }))
  })()`
  const bunCommand = `
    const { spawn } = require('node:child_process')
    spawn(${JSON.stringify(process.env.BUN_BIN ?? 'bun')}, ['-e', ${JSON.stringify(entry)}], {
      stdio: ['ignore', 'inherit', 'inherit'], env: process.env,
    })
    setInterval(() => {}, 1000)
  `
  const parentEntry = `
    process.env.CHARIOX_CLI_LAUNCHER_PID = String(process.pid)
    ${wrapped ? `require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(bunCommand)}], {stdio: ['ignore', 'inherit', 'inherit']}); setInterval(() => {}, 1000)` : bunCommand}
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
    if (wrapped) assert.notEqual(child.ppid, parent.pid)
    else assert.equal(child.ppid, parent.pid)
    childIdentity = guard.record(child.pid, { freshLaunch: true })
    guard.pid(parentIdentity, 'SIGKILL')
    await parentExited
    const deadline = Date.now() + 2000
    while (Date.now() < deadline && guard.refresh().some(row => row.pid === child.pid && row.state !== 'Z')) await delay(25)
    assert.equal(guard.refresh().some(row => row.pid === child.pid && row.state !== 'Z'), false,
      'Bun child survived its launcher by more than 2 seconds')
  } finally {
    for (const identity of guard.processes(parentIdentity)) guard.pid(identity, 'SIGKILL')
    guard.pid(parentIdentity, 'SIGKILL')
    if (childIdentity) guard.pid(childIdentity, 'SIGKILL')
    await parentExited
    await parentClosed
  }
})

// MP-08/MP-11: keep the dead launcher unreaped, as a supervisor may do.
test('Bun parent watch rejects an unreaped zombie launcher', { timeout: 10000 }, async () => {
  const module = new URL('../dist/cli-launcher-parent.js', import.meta.url).href
  const entry = `;(async () => {
    const { watchCliLauncherParent } = await import(${JSON.stringify(module)})
    // The fallback ignores cached ppid and inspects real ancestry/state.
    watchCliLauncherParent(process.env.CHARIOX_CLI_LAUNCHER_PID, { platform: "darwin" })
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
  // Popen.wait() runs only after the test sends a newline on stdin.
  const supervisorEntry = `
import subprocess, sys
parent = subprocess.Popen([sys.argv[1], '-e', sys.argv[2]])
sys.stdin.readline()
parent.wait(timeout=3)
`
  const supervisor = spawn('python3', ['-c', supervisorEntry, process.execPath, parentEntry],
    { stdio: ['pipe', 'pipe', 'pipe'] })
  const guard = createOwnedSignalGuard()
  const supervisorIdentity = guard.record(supervisor.pid, { freshLaunch: true })
  const closed = once(supervisor, 'close')
  let parentIdentity, childIdentity
  let output = '', stderr = ''
  supervisor.stderr.on('data', chunk => { stderr += chunk })
  try {
    const ready = new Promise((resolve, reject) => {
      supervisor.stdout.on('data', chunk => {
        output += chunk
        if (output.includes('\n')) resolve(JSON.parse(output.split('\n')[0]))
      })
      supervisor.on('error', reject)
    })
    const child = await Promise.race([ready, delay(3000).then(() => { throw new Error(`Bun did not start: ${stderr}`) })])
    parentIdentity = guard.record(child.ppid, { freshLaunch: true })
    childIdentity = guard.record(child.pid, { freshLaunch: true })
    guard.pid(parentIdentity, 'SIGKILL')
    await delay(250)
    const { execFileSync } = await import('node:child_process')
    const state = execFileSync('ps', ['-o', 'stat=', '-p', String(child.ppid)], { encoding: 'utf8' }).trim()
    assert.match(state, /^Z/, 'supervisor reaped the launcher before the probe')
    const deadline = Date.now() + 2000
    while (Date.now() < deadline && guard.refresh().some(row => row.pid === child.pid && row.state !== 'Z')) await delay(25)
    assert.equal(guard.refresh().some(row => row.pid === child.pid && row.state !== 'Z'), false,
      'Bun child survived its zombie launcher by more than 2 seconds')
  } finally {
    for (const identity of guard.processes(supervisorIdentity)) {
      if (identity.pid !== supervisor.pid) guard.pid(identity, 'SIGKILL')
    }
    if (parentIdentity) guard.pid(parentIdentity, 'SIGKILL')
    if (childIdentity) guard.pid(childIdentity, 'SIGKILL')
    supervisor.stdin.end('\n')
    await Promise.race([closed, delay(3000)])
    guard.pid(supervisorIdentity, 'SIGKILL')
    await closed
  }
})
