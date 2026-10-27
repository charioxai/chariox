import assert from 'node:assert/strict'
import test from 'node:test'
import path from 'node:path'
import {
  assertSameRoomObservers,
  buildObserverCliArgs,
  buildRoomObserverManifest,
  parseArgs,
  validateRemoteRelay,
} from './live-tui-web-parity-visual-session.mjs'

const observerArgs = [
  '--observe-room-session', 'room-1',
  '--kernel-url', 'ws://127.0.0.1:51000',
  '--slice-id', 'slice-1',
  '--workspace', '/tmp/workspace',
  '--worktree', '/tmp/worktree',
  '--web-observation', '/tmp/web-observation.json',
  '--relay-url', 'ws://localhost:52000',
  '--target-daemon-id', 'slice-daemon-1',
]

test('Room observer accepts the same-host relay identity as explicit arguments', () => {
  const options = parseArgs(observerArgs)
  assert.equal(options.relayUrl, 'ws://localhost:52000')
  assert.equal(options.targetDaemonId, 'slice-daemon-1')
  assert.equal(options.roomSessionId, 'room-1')
})

test('Room observer requires relay URL and daemon identity together', () => {
  assert.throws(
    () => parseArgs(observerArgs.slice(0, -2)),
    /--relay-url and --target-daemon-id must be supplied together/,
  )
})

test('remote relay observer fails closed without a nonempty environment token', () => {
  const options = parseArgs(observerArgs)
  assert.throws(() => validateRemoteRelay(options, undefined), /CHARIOX_DRILL_C_RELAY_TOKEN is required/)
  assert.throws(() => validateRemoteRelay(options, '  '), /CHARIOX_DRILL_C_RELAY_TOKEN is required/)
})

test('remote relay validation accepts loopback endpoints and returns token-free metadata', () => {
  const options = parseArgs(observerArgs)
  const relay = validateRemoteRelay(options, 'relay-secret-test-value')
  assert.deepEqual(relay, {
    url: 'ws://localhost:52000',
    targetDaemonId: 'slice-daemon-1',
  })
  assert.equal(Object.hasOwn(relay, 'token'), false)
})

test('remote relay validation rejects non-loopback endpoints and credential-bearing URLs', () => {
  const options = parseArgs(observerArgs)
  assert.throws(() => validateRemoteRelay({ ...options, relayUrl: 'wss://relay.example.test' }, 'relay-token'), /same-host loopback/)
  assert.throws(() => validateRemoteRelay({ ...options, relayUrl: 'ws://user:pass@localhost:52000' }, 'relay-token'), /must not include credentials/)
  assert.throws(() => validateRemoteRelay({ ...options, relayUrl: 'ws://localhost:52000/?token=secret' }, 'relay-token'), /must not include credentials/)
})

test('observer CLI arguments keep direct kernel and relay attachments separate', () => {
  const local = buildObserverCliArgs({
    cliPath: '/repo/apps/cli/dist/index.js',
    kernelUrl: 'ws://127.0.0.1:51000',
    automationSocket: '/tmp/local.sock',
    sessionId: 'room-1',
    workspace: '/tmp/workspace',
    worktree: '/tmp/worktree',
    clientId: 'local-observer',
  })
  assert.ok(local.includes('--kernel-url'))
  assert.ok(!local.includes('--relay-url'))
  assert.ok(local.includes('/tmp/local.sock'))

  const remote = buildObserverCliArgs({
    cliPath: '/repo/apps/cli/dist/index.js',
    relay: { url: 'ws://localhost:52000', targetDaemonId: 'slice-daemon-1' },
    automationSocket: '/tmp/remote.sock',
    sessionId: 'room-1',
    workspace: '/tmp/workspace',
    worktree: '/tmp/worktree',
    clientId: 'remote-observer',
  })
  assert.ok(remote.includes('--relay-url'))
  assert.ok(remote.includes('--relay-token-env'))
  assert.ok(remote.includes('CHARIOX_DRILL_C_RELAY_TOKEN'))
  assert.ok(!remote.includes('relay-secret-test-value'))
  assert.ok(remote.includes('--target-daemon-id'))
  assert.ok(remote.includes('/tmp/remote.sock'))
  assert.ok(!remote.includes('--kernel-url'))
})

test('Room observer manifest records both absolute sockets and token-free remote relay identity', () => {
  const manifest = buildRoomObserverManifest({
    startedAt: '2026-09-24T00:00:00.000Z',
    repoRoot: '/repo',
    rootDir: '/tmp/drill-c',
    evidenceDir: '/tmp/drill-c/evidence',
    kernelUrl: 'ws://127.0.0.1:51000',
    sessionId: 'room-1',
    sliceId: 'slice-1',
    workspace: '/tmp/workspace',
    worktree: '/tmp/worktree',
    webObservationPath: '/tmp/web-observation.json',
    baseline: { sessionId: 'room-1' },
    relay: { url: 'ws://localhost:52000', targetDaemonId: 'slice-daemon-1' },
    automationSocket: '/tmp/local.sock',
    remoteAutomationSocket: '/tmp/remote.sock',
    reportPath: '/tmp/drill-c/evidence/report.json',
    manifestPath: '/tmp/drill-c/manifest.json',
    relayToken: 'relay-secret-test-value',
  })
  assert.equal(manifest.automationSocket, '/tmp/local.sock')
  assert.equal(manifest.remoteAutomationSocket, '/tmp/remote.sock')
  assert.ok(path.isAbsolute(manifest.remoteAutomationSocket))
  assert.deepEqual(manifest.remoteRelay, {
    url: 'ws://localhost:52000',
    targetDaemonId: 'slice-daemon-1',
  })
  assert.doesNotMatch(JSON.stringify(manifest), /relay-secret-test-value|relayToken/i)
})

test('both observer snapshots must identify the requested same Room', () => {
  assert.doesNotThrow(() => assertSameRoomObservers(
    { session: { id: 'room-1' } },
    { session: { id: 'room-1' } },
    'room-1',
  ))
  assert.throws(() => assertSameRoomObservers(
    { session: { id: 'room-1' } },
    { session: { id: 'room-2' } },
    'room-1',
  ), /remote relay TUI did not attach/)
})
