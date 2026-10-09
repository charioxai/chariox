// MP-08/MP-10/MP-11: real compiled TUI, real kernel, real hosted relay.
// The caller owns/enrolls the kernel. This drill creates a normal terminal
// pairing and drives the real TUI's detached attach action, never a mock client.
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { mkdir, mkdtemp, writeFile, rm, readFile, readdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { LocalIpcClient } from '../dist/ipc.js'
import { createCliRelayIdentityStore } from '../dist/cli-relay-identity-store.js'
import { createTerminalPairingLink, joinKernelTerminalPairingLink } from '../dist/relay-api.js'
import { parseArgs } from '../dist/cli-options.js'

const env = process.env
const evidence = path.resolve(env.CHARIOX_LOCAL_DIRECT_EVIDENCE ?? '')
const kernelHome = env.CHARIOX_LOCAL_DIRECT_KERNEL_HOME
const binary = env.CHARIOX_LOCAL_DIRECT_TUI_BINARY
assert(kernelHome && binary && env.CHARIOX_LOCAL_DIRECT_KERNEL_URL && evidence !== process.cwd(), 'MP-10 explicit owned runtime, binary and evidence required')
await mkdir(evidence, { recursive: true })
const root = await mkdtemp(path.join(env.TMPDIR ?? '/root/.chariox/dev/localdirect-b3/tmp', 'terminal-live-'))
const previous = { HOME: env.HOME, CHARIOX_HOME: env.CHARIOX_HOME, XDG_CONFIG_HOME: env.XDG_CONFIG_HOME, XDG_STATE_HOME: env.XDG_STATE_HOME }
const home = root + '/home'; await mkdir(home, { recursive: true, mode: 0o700 })
Object.assign(env, { HOME: home, CHARIOX_HOME: home + '/chariox', XDG_CONFIG_HOME: home + '/config', XDG_STATE_HOME: home + '/state' })
const local = new LocalIpcClient(env.CHARIOX_LOCAL_DIRECT_KERNEL_URL, { localAuthEnvironment: { HOME: kernelHome, CHARIOX_HOME: kernelHome + '/chariox' } })
let bootstrap
const result = { items: ['MP-08', 'MP-10', 'MP-11'], status: 'FAIL', root, realTui: true }
try {
  const status = (await local.send({ RelayStatus: null })).RelayStatus.status
  assert(status.connected && status.relay_url.startsWith('wss://'), 'MP-10 real hosted WSS required')
  const pairing = await createTerminalPairingLink(local)
  const options = parseArgs(['--terminal-pairing-link', pairing.pairing_link])
  const identity = createCliRelayIdentityStore().getOrCreate()
  bootstrap = new LocalIpcClient(options.relayUrl, { relayAuthToken: options.relayToken, targetDaemonId: options.targetDaemonId, relayIdentity: identity })
  const joined = await joinKernelTerminalPairingLink(bootstrap, pairing.pairing_link, pairing.terminal_id, identity.publicKeyThumbprint)
  await bootstrap.close(); bootstrap = null
  const helper = path.join(path.dirname(fileURLToPath(import.meta.url)), 'lib/terminal-local-direct-live.py')
  const child = spawn('python3', [helper], { stdio: ['pipe', 'pipe', 'pipe'] })
  assert(Number.isSafeInteger(child.pid) && child.pid > 1)
  let output = ''; child.stdout.on('data', data => { output += data })
  // Helper emits only fixed diagnostics. It never serializes argv or env.
  child.stderr.on('data', () => {})
  child.stdin.end(JSON.stringify({ binary, evidence, root, relayUrl: pairing.relay_url, kernelId: status.daemon_id,
    token: joined.relay_token, registry: kernelHome + '/chariox/kernels/active', env: { ...env },
    expectLocal: env.CHARIOX_LOCAL_DIRECT_EXPECT_LOCAL !== '0', durationMs: Number(env.CHARIOX_LOCAL_DIRECT_DURATION_MS ?? 35_000) }))
  const code = await new Promise(resolve => child.once('exit', resolve))
  result.driverExit = code
  result.summary = output.trim().slice(0, 200)
  assert.equal(code, 0, 'MP-08 built detached TUI carrier drill failed; inspect retained evidence')
  // MP-10: a control lane can reconnect between UI snapshots. The owned
  // kernel's session records must also show no close during the live interval.
  const driver = JSON.parse(await readFile(evidence + '/terminal-driver.json', 'utf8'))
  result.transportWindow = driver.transportWindow
  const logDir = env.CHARIOX_LOCAL_DIRECT_KERNEL_LOG_DIR ?? path.join(kernelHome, 'chariox/logs')
  const logs = (await readdir(logDir)).filter(name => /-daemon-\d+\.ndjson$/.test(name))
  assert(logs.length > 0, 'MP-10 owned kernel logs required for strict serving-session checks')
  result.kernelSessionClosures = []
  for (const name of logs) {
    for (const line of (await readFile(path.join(logDir, name), 'utf8')).split('\n')) {
      let event
      try { event = JSON.parse(line) } catch { continue }
      if (event.message === 'local browser session closed'
        && event.timestamp_ms >= driver.transportWindow.startedAtMs
        && event.timestamp_ms <= driver.transportWindow.finishedAtMs) {
        result.kernelSessionClosures.push({ at: event.timestamp_ms, reason: event.reason })
      }
    }
  }
  assert.equal(result.kernelSessionClosures.length, 0, 'MP-10 unexpected serving-session closure despite a connected TUI snapshot')
  result.status = 'PASS'
} finally {
  await bootstrap?.close(); await local.close()
  for (const [key, value] of Object.entries(previous)) { if (value === undefined) delete env[key]; else env[key] = value }
  await rm(root, { recursive: true, force: true }); result.runtimeRootRemoved = true
  await writeFile(evidence + '/terminal-drill.json', JSON.stringify(result, null, 2) + '\n')
}
