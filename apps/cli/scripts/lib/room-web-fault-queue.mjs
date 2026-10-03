// MP-08 / MP-10: bounded read-only pressure through the supported request builder.
import { setTimeout as sleep } from 'node:timers/promises'
export function startRoomQueuePressure({ client, requests, sessionId, protocolVersion, durationMs = 8000, width = 128, onComplete = () => {} }) {
  const request = requests.getRoomEnvironmentStateRequest(sessionId, protocolVersion)
  return (async () => {
    const values = []
    const deadline = Date.now() + durationMs
    do {
      values.push(...await Promise.all(Array.from({ length: width }, () => Promise.resolve().then(() => client.send(request, { timeoutMs: 8000 })).then(() => 'ok', error => /backpressure|queue.*full|no available capacity/i.test(error.message) ? 'backpressure' : 'other-error'))))
      await sleep(200)
    } while (Date.now() < deadline)
    onComplete()
    return values
  })()
}
