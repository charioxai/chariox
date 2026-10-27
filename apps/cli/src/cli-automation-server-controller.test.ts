import assert from "node:assert/strict"
import test from "node:test"

import {
  createCliAutomationServerController,
  type CliAutomationServerControllerDeps,
} from "./cli-automation-server-controller.js"
import type { CliAutomationServer } from "./cli-automation.js"

test("cli automation server controller stays idle without a socket path", async () => {
  const harness = createHarness({ socketPath: undefined })

  harness.controller.start()
  await Promise.resolve()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), [])
})

test("cli automation server controller starts and stops the automation socket", async () => {
  const server = fakeServer("server-1")
  const harness = createHarness({ server })

  harness.controller.start()
  await harness.flushStart()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), [
    "start:/tmp/chariox.sock",
    "info:cli automation socket listening:/tmp/chariox.sock",
    "stop:server-1:/tmp/chariox.sock",
  ])
})

test("cli automation server controller reports start failures", async () => {
  const harness = createHarness({
    startError: new Error("bind failed"),
  })

  harness.controller.start()
  await harness.flushStart()
  harness.controller.stop()

  assert.deepEqual(harness.calls(), [
    "start:/tmp/chariox.sock",
    "error:failed to start cli automation socket:/tmp/chariox.sock:bind failed",
    "flash:automation socket failed: bind failed:error",
  ])
})

function createHarness(options: {
  socketPath?: string | undefined
  server?: CliAutomationServer
  startError?: Error
} = {}) {
  const calls: string[] = []
  let startPromise: Promise<void> | null = null
  const socketPath = Object.hasOwn(options, "socketPath")
    ? options.socketPath
    : "/tmp/chariox.sock"
  const deps: CliAutomationServerControllerDeps = {
    socketPath,
    handleRequest: () => ({ ok: true }),
    startServer: async ({ socketPath: requestedSocketPath, onListening }) => {
      calls.push(`start:${requestedSocketPath}`)
      if (options.startError) {
        throw options.startError
      }
      onListening?.(requestedSocketPath)
      return options.server ?? fakeServer("server-default")
    },
    stopServer: (server, stoppedSocketPath) => {
      calls.push(`stop:${String((server as { id?: unknown }).id)}:${stoppedSocketPath}`)
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
    logger: {
      info: (message, fields) => {
        calls.push(`info:${message}:${String(fields?.socket_path)}`)
      },
      error: (message, fields) => {
        calls.push(`error:${message}:${String(fields?.socket_path)}:${String(fields?.error)}`)
      },
    },
    flashFooter: (message, tone) => {
      calls.push(`flash:${message}:${tone}`)
    },
  }
  const controller = createCliAutomationServerController({
    ...deps,
    startServer: (startOptions) => {
      const started = deps.startServer(startOptions)
      startPromise = started.then(() => undefined, () => undefined)
      return started
    },
  })

  return {
    controller,
    calls: () => calls,
    flushStart: async () => {
      await startPromise
      await Promise.resolve()
    },
  }
}

function fakeServer(id: string): CliAutomationServer {
  return { id } as unknown as CliAutomationServer
}
