import assert from 'node:assert/strict'
import test from 'node:test'

import {
  classifyDrillChildFailure,
  formatDrillChildFailure,
} from './drill-child-process.mjs'

test('classifies provider account and billing failures', () => {
  const text = '**OpenCode error**\n\nInsufficient balance. Manage your billing here: https://opencode.ai/workspace/example/billing'

  assert.equal(classifyDrillChildFailure(text), 'provider-account')
  assert.match(
    formatDrillChildFailure('remote workspace live sync drill', 1, null, '', text),
    /Provider account\/billing blocked validation/,
  )
})

test('classifies provider account model limitations before provider marker timeouts', () => {
  const text = [
    'Error: timed out waiting for marker THREAD_TRANSFER_WORKER_CODEX_28090_1783606899611; ordered_match=false',
    `{"type":"error","status":400,"error":{"message":"The 'gpt-5.2-codex' model is not supported when using Codex with a ChatGPT account."}}`,
  ].join('\n')

  assert.equal(classifyDrillChildFailure(text), 'provider-account')
})

test('classifies provider authentication failures', () => {
  const text = 'Token refresh failed: 401'

  assert.equal(classifyDrillChildFailure(text), 'provider-auth')
  assert.match(
    formatDrillChildFailure('remote workspace live sync drill', 1, null, '', text),
    /Provider authentication blocked validation/,
  )
})

test('redacts token-shaped values from formatted child output tails', () => {
  const text = 'Token refresh failed: 401\nAuthorization: Bearer abcdefghijklmnopqrstuvwxyz'
  const formatted = formatDrillChildFailure('remote workspace live sync drill', 1, null, '', text)

  assert.match(formatted, /Authorization: <redacted>/)
  assert.doesNotMatch(formatted, /abcdefghijklmnopqrstuvwxyz/)
})

test('keeps generic child process failures distinct from provider failures', () => {
  assert.equal(classifyDrillChildFailure('assertion failed after relay setup'), 'child-process')
})

test('classifies local test harness prerequisite failures', () => {
  const text = 'Error: relay failed to start: spawn cargo ENOENT'

  assert.equal(classifyDrillChildFailure(text), 'test-harness')
  assert.match(
    formatDrillChildFailure('matrix drill', 1, null, '', text),
    /Local drill prerequisites or build tooling blocked validation/,
  )
})

test('classifies Docker slice image build failures as test harness failures', () => {
  const text = [
    'docker build -f apps/kernel/slice-linux-docker/docker/Dockerfile -t arroba-slice-linux:0.1.0 . exited with code 101',
    'stdout tail:',
    "error: couldn't read `src/../../../examples/workflow-code/prompt-chaining.js`: No such file or directory",
  ].join('\n')

  assert.equal(classifyDrillChildFailure(text), 'test-harness')
})

test('classifies cloud deployment runtime failures', () => {
  const text = 'deployment did not become ready: {"status":"FAILED","lastError":"Scalingo 503 service unavailable"}'

  assert.equal(classifyDrillChildFailure(text), 'cloud-runtime')
  assert.match(
    formatDrillChildFailure('cloud publication drill', 1, null, '', text),
    /Cloud control-plane or deployment infrastructure blocked validation/,
  )
})

test('classifies relay runtime failures', () => {
  const text = 'target daemon disconnected from relay while waiting for response'

  assert.equal(classifyDrillChildFailure(text), 'relay-runtime')
  assert.match(
    formatDrillChildFailure('remote drill', 1, null, '', text),
    /Relay transport blocked validation/,
  )
})

test('classifies relay target heartbeat freshness failures', () => {
  const text = 'Selected relay kernel target is stale (last heartbeat 91s ago); relaunch or wait for a fresh heartbeat'

  assert.equal(classifyDrillChildFailure(text), 'relay-target-freshness')
  assert.match(
    formatDrillChildFailure('browser relay bootstrap drill', 1, null, '', text),
    /Relay target heartbeat freshness blocked validation/,
  )
})

test('classifies remote worker protocol version skew', () => {
  const text = 'remote worker `worker-1` uses relay peer protocol 2, but this home kernel requires 3. Upgrade and restart the worker kernel, then retry the remote agent.'

  assert.equal(classifyDrillChildFailure(text), 'remote-worker-version')
  assert.match(
    formatDrillChildFailure('hetzner remote agent drill', 1, null, '', text),
    /Remote worker kernel version\/protocol skew blocked validation/,
  )
})

test('classifies remote worker checkout version skew', () => {
  const text = 'remote worker checkout `/tmp/arroba-native-remote-validate` is at commit 1111111, but home checkout expects 2222222. Upgrade/rebuild the remote worker checkout and restart the worker kernel, then rerun the drill.'

  assert.equal(classifyDrillChildFailure(text), 'remote-worker-version')
})

test('classifies remote host disk capacity failures', () => {
  const text = 'fatal: cannot create directory at apps/kernel/target: No space left on device'

  assert.equal(classifyDrillChildFailure(text), 'remote-host-capacity')
  assert.match(
    formatDrillChildFailure('hetzner remote agent drill', 1, null, '', text),
    /Remote host disk or filesystem capacity blocked validation/,
  )
})

test('classifies runtime state timeouts', () => {
  const text = 'timed out waiting for agents to become idle: agent-1\nlast_observation=[{"agentState":"Working"}]'

  assert.equal(classifyDrillChildFailure(text), 'runtime-timeout')
  assert.match(
    formatDrillChildFailure('runtime drill', 1, null, '', text),
    /Runtime state did not converge/,
  )
})

test('classifies provider thread marker timeouts as provider runtime failures', () => {
  const text = [
    'Error: timed out waiting for marker THREAD_TRANSFER_WORKER_CODEX_83612_1783605777388; ordered_match=false',
    'compact:',
    'fallback_compact:',
    'raw_compact:',
    'at waitForHistoryOutputMarker (apps/cli/scripts/lib/live-provider-thread-transfer-runtime.mjs:806:9)',
  ].join('\n')

  assert.equal(classifyDrillChildFailure(text), 'provider-error')
  assert.match(
    formatDrillChildFailure('provider thread transfer drill', 1, null, '', text),
    /Provider runtime did not complete the drill/,
  )
})

test('classifies kernel authority drift failures', () => {
  const text = "kernel rejected request: agent `agent-1` does not belong to session `session-2`"

  assert.equal(classifyDrillChildFailure(text), 'kernel-authority')
  assert.match(
    formatDrillChildFailure('authority drill', 1, null, '', text),
    /Kernel authority state rejected the request/,
  )
})

test('classifies duplicate provider run binding health failures as kernel authority', () => {
  assert.equal(classifyDrillChildFailure('daemon health duplicate_arroba_agent_bindings: agent-1 has run-a and run-b'), 'kernel-authority')
  assert.equal(classifyDrillChildFailure('daemon health multi_interface_agent_bindings: agent-1 has Arroba and native TUI runs'), 'kernel-authority')
  assert.equal(classifyDrillChildFailure('multiple provider runs are bound to agent agent-1'), 'kernel-authority')
})

test('classifies remote extension manifest sync failures', () => {
  const text = 'remote extension manifest sync failed; home validation remains authoritative'

  assert.equal(classifyDrillChildFailure(text), 'remote-extension-sync')
  assert.match(
    formatDrillChildFailure('remote extension drill', 1, null, '', text),
    /Remote extension manifest state did not reconcile/,
  )
})

test('classifies runtime projection health failures separately from client rendering failures', () => {
  const text = 'daemon health projection invariant failed: session prompt read-model stale for session session-1'

  assert.equal(classifyDrillChildFailure(text), 'runtime-projection-health')
  assert.match(
    formatDrillChildFailure('projection health drill', 1, null, '', text),
    /Runtime projection health failed/,
  )
})

test('keeps generic stale projection failures in the projection-staleness bucket', () => {
  const text = 'session projection stale after projection reconciliation failed'

  assert.equal(classifyDrillChildFailure(text), 'projection-staleness')
  assert.match(
    formatDrillChildFailure('projection health drill', 1, null, '', text),
    /Kernel projection state is stale/,
  )
})

test('classifies remote worker execution failures', () => {
  const text = 'remote worker execution failed: leased agent agent-1 failed to launch provider run'

  assert.equal(classifyDrillChildFailure(text), 'worker-execution')
  assert.match(
    formatDrillChildFailure('remote worker drill', 1, null, '', text),
    /Remote worker execution failed/,
  )
})

test('classifies UI client projection failures', () => {
  const text = 'web terminal projection failed: terminal event was not rendered in transcript'

  assert.equal(classifyDrillChildFailure(text), 'ui-client-projection')
  assert.match(
    formatDrillChildFailure('web terminal drill', 1, null, '', text),
    /UI\/client projection failed/,
  )
})

test('classifies workspace live sync conflicts', () => {
  const text = 'Workspace Live Sync result skipped_conflict for src/app.ts after target changed outside workspace live sync'

  assert.equal(classifyDrillChildFailure(text), 'workspace-live-sync-conflict')
  assert.match(
    formatDrillChildFailure('workspace live sync drill', 1, null, '', text),
    /Workspace Live Sync detected a conflict/,
  )
})

test('classifies slice provider auth failures separately', () => {
  const text = 'slice_lifecycle.provider_auth_issues: provider=codex status=not_configured'

  assert.equal(classifyDrillChildFailure(text), 'slice-auth')
  assert.match(
    formatDrillChildFailure('slice drill', 1, null, '', text),
    /Slice provider authentication is missing/,
  )
})

test('classifies slice runtime failures separately from auth and Docker', () => {
  const text = 'slice_lifecycle.launch_timeout: slice slice-1 did not become ready after container start'

  assert.equal(classifyDrillChildFailure(text), 'slice-runtime')
  assert.match(
    formatDrillChildFailure('slice drill', 1, null, '', text),
    /Slice runtime failed/,
  )
})

test('keeps relay timeouts classified as relay runtime', () => {
  assert.equal(classifyDrillChildFailure('timed out waiting for relay target worker'), 'relay-runtime')
})

test('classifies Docker runtime failures', () => {
  const text = 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock'

  assert.equal(classifyDrillChildFailure(text), 'docker-runtime')
  assert.match(
    formatDrillChildFailure('container drill', 1, null, '', text),
    /Docker or Colima blocked validation/,
  )
})

test('does not classify stack frame line numbers as provider auth', () => {
  const text = [
    'LocalIpcError: kernel transport `connect kernel websocket` failed: connect ECONNREFUSED 127.0.0.1:9',
    '    at WebSocket.handleConnectError (file:///repo/packages/kernel-client/dist/ipc.js:401:51)',
  ].join('\n')

  assert.equal(classifyDrillChildFailure(text), 'child-process')
})
