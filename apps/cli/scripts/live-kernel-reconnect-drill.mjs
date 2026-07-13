import assert from 'node:assert/strict'
import { once } from 'node:events'
import { mkdir } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { WebSocketServer } from 'ws'
import { finalizeDrillArtifacts, prepareDrillArtifacts } from './lib/drill-artifacts.mjs'

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, '..')

const distIpcUrl = pathToFileURL(path.join(cliRoot, 'dist', 'ipc.js')).href
const { LocalIpcClient } = await import(distIpcUrl)

function parseArgs(argv) {
  const options = { keepArtifactsOnFailure: false }
  for (const arg of argv) {
    if (arg === '--') continue
    if (arg === '--keep-artifacts-on-failure') options.keepArtifactsOnFailure = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node apps/cli/scripts/live-kernel-reconnect-drill.mjs [--keep-artifacts-on-failure]')
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

async function main() {
  const preserveOnFailure = process.argv.slice(2).includes('--keep-artifacts-on-failure')
  const rootDir = path.join(os.tmpdir(), `arroba-kernel-reconnect-${process.pid}-${Date.now()}`)
  const artifactsDir = path.join(rootDir, 'artifacts')
  let options = { keepArtifactsOnFailure: preserveOnFailure }
  let server = null
  let client = null
  let dispose = null
  let endpoint = null
  let succeeded = false
  let failure = null
  const subscribeFrames = []
  const requestFrames = []
  const events = []
  let nextEventId = 1

  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(artifactsDir, { recursive: true })
    options = parseArgs(process.argv.slice(2))

    server = new WebSocketServer({ port: 0 })
    await once(server, 'listening')

    const address = server.address()
    endpoint = `ws://127.0.0.1:${address.port}`

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

    client = new LocalIpcClient(endpoint)
    dispose = client.onKernelEvent((event) => {
      events.push(event)
    })

    await client.subscribeToKernelEvents('session-live-reconnect', 'attachment-live-reconnect')
    const controlResponse = await client.send({ GetSessionState: { session_id: 'session-live-reconnect' } })

    const deadline = Date.now() + 2_000
    while (
      Date.now() < deadline
      && !events.some((event) => event.event === 'transport_resumed')
    ) {
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
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    dispose?.()
    await client?.close?.().catch(() => {})
    if (server) {
      await new Promise((resolve) => {
        server.close(() => resolve())
      })
    }
    await finalizeDrillArtifacts({
      rootDir,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
      metadata: {
        drill: 'kernel-reconnect',
        endpoint,
        subscribeFrames,
        requestFrames,
        observedEvents: events.map((event) => event.event),
        nextEventId,
      },
      log: (name, details) => console.log(`[kernel-reconnect-drill] ${name}`, JSON.stringify(details)),
    })
    if (!succeeded && options.keepArtifactsOnFailure) {
      console.error(`[kernel-reconnect-drill] artifacts retained at ${rootDir}`)
    }
  }
}

await main()
