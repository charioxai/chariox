// MP-08 / MP-10 / MP-11 H8: first failure identity survives salvage failures.
import assert from 'node:assert/strict'
import test from 'node:test'
import { observeKernelRpcErrors, rpcErrorRecord } from './rpc-errors.mjs'

test('retain original RPC operation, code and message before salvage', async () => {
  const original = Object.assign(new Error('read peer response exceeded 5000ms'), {
    name: 'LocalIpcError', operation: 'handle kernel response', code: 'transport_error', retryable: false,
  })
  const retained = [], client = { async send() { throw original } }
  observeKernelRpcErrors(client, async failure => retained.push(failure))
  await assert.rejects(client.send({ SpawnAgent: {} }), error => error === original)
  assert.equal(retained[0].requestKind, 'SpawnAgent')
  assert.deepEqual(retained[0].error, rpcErrorRecord(original))
  assert.equal(retained[0].error.message, original.message)
  assert.equal(retained[0].error.code, 'transport_error')
  assert.ok(retained[0].elapsedMs >= 0)
})

test('retention failure preserves error and successful replies remain unchanged', async () => {
  const original = new Error('original RPC failure'), response = { SessionState: {} }
  const client = { async send(request) { if (request.Fail) throw original; return response } }
  observeKernelRpcErrors(client, async () => { throw Error('evidence storage failed') })
  await assert.rejects(client.send({ Fail: {} }), error => error === original)
  assert.equal(await client.send({ Read: {} }), response)
})

test('bounded RPC diagnostics redact recognized secret values', () => {
  const record = rpcErrorRecord(new Error('Bearer abcdefghijklmnopqrstuvwxyz'))
  assert.equal(record.message, '<redacted>')
  assert.equal(rpcErrorRecord(new Error('x'.repeat(9000))).message.length, 8192)
})
