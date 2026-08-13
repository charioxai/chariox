import assert from 'node:assert/strict'
import test from 'node:test'

import { publicationStatusWatchdogCount, publicationStatusWatchdogs, readSseUntilEvent, secureGatewayPublicationEnvs, withPublicationDrillProviderInventory } from './live-workflow-publication-drill-runtime.mjs'

test('publication drill reads canonical watchdog status with schedule fallback', () => {
  const canonical = { watchdog_count: 1, watchdogs: [{ id: 'watchdog-1' }] }
  const legacy = { schedule_count: 1, schedules: [{ id: 'schedule-1' }] }

  assert.equal(publicationStatusWatchdogCount(canonical), 1)
  assert.deepEqual(publicationStatusWatchdogs(canonical), canonical.watchdogs)
  assert.equal(publicationStatusWatchdogCount(legacy), 1)
  assert.deepEqual(publicationStatusWatchdogs(legacy), legacy.schedules)
})

test('publication drill exposes its internal provider during package replay', () => {
  assert.deepEqual(withPublicationDrillProviderInventory({ EXISTING: 'value' }), {
    EXISTING: 'value',
    CHARIOX_PROVIDER_DEV_STUB: '1',
  })
})

test('secure publication gateways bind HTTPS and WSS to their matching transports', () => {
  const secureEnvs = secureGatewayPublicationEnvs(
    { EXISTING: 'value' },
    {
      host: '127.0.0.1',
      port: 43119,
      kernelUrl: 'ws://127.0.0.1:43118',
      tls: { keyFile: '/tmp/gateway.key', certFile: '/tmp/gateway.crt' },
      humanHttp: { sessionId: 'human-session', publicationId: 'human-publication' },
      websocket: { sessionId: 'websocket-session', publicationId: 'websocket-publication' },
    },
  )

  assert.equal(secureEnvs.https.CHARIOX_PUBLICATION_SESSION_ID, 'human-session')
  assert.equal(secureEnvs.https.CHARIOX_PUBLICATION_ID, 'human-publication')
  assert.equal(secureEnvs.wss.CHARIOX_PUBLICATION_SESSION_ID, 'websocket-session')
  assert.equal(secureEnvs.wss.CHARIOX_PUBLICATION_ID, 'websocket-publication')
  assert.equal(secureEnvs.wss.CHARIOX_PUBLICATION_TLS_CERT_FILE, '/tmp/gateway.crt')
  assert.equal(secureEnvs.wss.EXISTING, 'value')
})

test('queued-only SSE checks cancel the stream after the expected frame', async () => {
  let canceled = false
  const encoder = new TextEncoder()
  const response = new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode('event: que'))
      controller.enqueue(encoder.encode('ued\ndata: {"invocation_id":"request-1"}\n\n'))
    },
    cancel() {
      canceled = true
    },
  }))

  const body = await readSseUntilEvent(response, 'queued', { timeoutMs: 100 })

  assert.match(body, /event: queued/)
  assert.equal(canceled, true)
})
