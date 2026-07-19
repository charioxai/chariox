import assert from 'node:assert/strict'
import test from 'node:test'

import { publicationStatusWatchdogCount, publicationStatusWatchdogs, readSseUntilEvent, withPublicationDrillProviderInventory } from './live-workflow-publication-drill-runtime.mjs'

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
    ARROBA_PROVIDER_DEV_STUB: '1',
  })
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
