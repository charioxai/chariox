import assert from 'node:assert/strict'
import { once } from 'node:events'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { WebSocketServer } from 'ws'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')

const distIpcUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc.js')).href
const { LocalIpcClient } = await import(distIpcUrl)

const server = new WebSocketServer({ port: 0 })
await once(server, 'listening')

const address = server.address()
const endpoint = `ws://127.0.0.1:${address.port}`
const subscribeFrames = []
const requestFrames = []
let nextEventId = 1

server.on('connection', (socket) => {
  socket.on('message', (payload) => {
    const frame = JSON.parse(String(payload))
    if (frame.type === 'subscribe') {
      subscribeFrames.push(frame)
      socket.send(JSON.stringify({
        type: 'response',
        request_id: frame.request_id,
        response: { ok: true, resumed_from_event_id: frame.resume_from_event_id ?? null },
        error: null,
      }))
      socket.send(JSON.stringify({
        type: 'event',
        event_id: nextEventId,
        event: {
          event: 'heartbeat',
          session_id: frame.session_id,
        },
      }))
      nextEventId += 1
      if (subscribeFrames.length === 1) {
        setTimeout(() => socket.close(1008, 'kernel transport overloaded; reconnecting'), 25)
      }
      return
    }

    if (frame.type === 'request') {
      requestFrames.push(frame)
      setTimeout(() => {
        socket.send(JSON.stringify({
          type: 'response',
          request_id: frame.request_id,
          response: { ok: true, echoed: frame.request },
          error: null,
        }))
      }, 75)
    }
  })
})

const client = new LocalIpcClient(endpoint)
const events = []
const dispose = client.onKernelEvent((event) => {
  events.push(event)
})

try {
  await client.subscribeToKernelEvents('session-live-reconnect', 'attachment-live-reconnect')
  const controlResponse = await client.send({ GetSessionState: { session_id: 'session-live-reconnect' } })

  const deadline = Date.now() + 2_000
  while (Date.now() < deadline && subscribeFrames.length < 2) {
    await new Promise((resolve) => setTimeout(resolve, 25))
  }

  assert.equal(controlResponse.ok, true)
  assert.deepEqual(controlResponse.echoed, { GetSessionState: { session_id: 'session-live-reconnect' } })
  assert.equal(requestFrames.length, 1)
  assert.equal(subscribeFrames.length >= 2, true)
  assert.equal(subscribeFrames[0].resume_from_event_id, null)
  assert.equal(subscribeFrames[1].resume_from_event_id, 1)
  assert.equal(events.some((event) => event.event === 'transport_closed'), true)
  assert.equal(events.some((event) => event.event === 'transport_resumed'), true)

  console.log(JSON.stringify({
    ok: true,
    endpoint,
    control_requests: requestFrames.length,
    subscribe_attempts: subscribeFrames.length,
    second_resume_from_event_id: subscribeFrames[1].resume_from_event_id,
    observed_events: events.map((event) => event.event),
  }, null, 2))
} finally {
  dispose()
  await client.close()
  await new Promise((resolve) => {
    server.close(() => resolve())
  })
}
