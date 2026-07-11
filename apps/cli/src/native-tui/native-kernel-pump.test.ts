import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcClient } from "../ipc.js"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"

test("native kernel pump uses a liveness cadence when no output projection is needed", async () => {
  const harness = installTimerHarness()
  const requests: string[] = []
  const client = fakeClient((request) => {
    requests.push(requestVariant(request))
    return { RuntimeNotices: { notices: [] } }
  })

  try {
    const pump = startNativeKernelPumpLoop(client, "session-1", "attachment-1")
    await flushPromises()

    assert.equal(harness.intervalMs(), 10_000)
    assert.deepEqual(requests, ["PollRuntimeNotices"])
    pump.stop()
  } finally {
    harness.restore()
  }
})

test("native kernel pump keeps remote output projection responsive without notice polling", async () => {
  const harness = installTimerHarness()
  const requests: string[] = []
  const client = fakeClient((request) => {
    requests.push(requestVariant(request))
    return { TerminalOutput: { records: [] } }
  })

  try {
    const pump = startNativeKernelPumpLoop(client, "session-1", "attachment-1", {
      onTerminalRecords: () => {},
      pollRuntimeNotices: false,
    })
    await flushPromises()

    assert.equal(harness.intervalMs(), 250)
    assert.deepEqual(requests, ["PumpTerminalOutput"])
    pump.stop()
  } finally {
    harness.restore()
  }
})

function fakeClient(
  send: (request: Record<string, unknown>) => Record<string, unknown>,
): LocalIpcClient {
  return { send: async (request: Record<string, unknown>) => send(request) } as unknown as LocalIpcClient
}

function requestVariant(request: Record<string, unknown>): string {
  return Object.keys(request)[0] ?? "unknown"
}

function installTimerHarness() {
  const originalSetInterval = globalThis.setInterval
  const originalClearInterval = globalThis.clearInterval
  const timer = {} as ReturnType<typeof setInterval>
  let selectedIntervalMs: number | undefined

  globalThis.setInterval = ((_: () => void, intervalMs?: number) => {
    selectedIntervalMs = intervalMs
    return timer
  }) as typeof setInterval
  globalThis.clearInterval = (() => {}) as typeof clearInterval

  return {
    intervalMs: () => selectedIntervalMs,
    restore() {
      globalThis.setInterval = originalSetInterval
      globalThis.clearInterval = originalClearInterval
    },
  }
}

async function flushPromises(): Promise<void> {
  await new Promise<void>((resolve) => setImmediate(resolve))
}
