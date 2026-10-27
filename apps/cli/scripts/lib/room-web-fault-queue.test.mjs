// MP-08 / MP-10: pressure must use the negotiated supported contract and settle failures.
import assert from 'node:assert/strict'
import test from 'node:test'
import { startRoomQueuePressure } from './room-web-fault-queue.mjs'
test('queue pressure preserves the protocol gate and accounts for rejected sends', async () => {
  let sent = 0
  const responses = await startRoomQueuePressure({
    sessionId: 'room', protocolVersion: 369, durationMs: 0, width: 3,
    requests: { getRoomEnvironmentStateRequest(sessionId, version) { assert.equal(version, 369); return { GetRoomEnvironmentState: { session_id: sessionId } } } },
    client: { async send(request) { assert.equal(request.GetRoomEnvironmentState.session_id, 'room'); const id = sent++; if (id === 1) throw new Error('no available capacity'); if (id === 2) throw new Error('socket closed') } },
  })
  assert.deepEqual(responses, ['ok', 'backpressure', 'other-error'])
})
test('unsupported request construction fails before starting a background burst', () => {
  assert.throws(() => startRoomQueuePressure({ requests: { getRoomEnvironmentStateRequest() { throw new Error('unsupported protocol') } } }), /unsupported protocol/)
})
