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

test('does not classify stack frame line numbers as provider auth', () => {
  const text = [
    'LocalIpcError: kernel transport `connect kernel websocket` failed: connect ECONNREFUSED 127.0.0.1:9',
    '    at WebSocket.handleConnectError (file:///repo/packages/kernel-client/dist/ipc.js:401:51)',
  ].join('\n')

  assert.equal(classifyDrillChildFailure(text), 'child-process')
})
