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

test('classifies provider authentication failures', () => {
  const text = 'Token refresh failed: 401'

  assert.equal(classifyDrillChildFailure(text), 'provider-auth')
  assert.match(
    formatDrillChildFailure('remote workspace live sync drill', 1, null, '', text),
    /Provider authentication blocked validation/,
  )
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
