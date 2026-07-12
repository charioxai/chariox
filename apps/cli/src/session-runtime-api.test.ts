import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { resizeSessionTerminal } from "./session-runtime-api.js"

test("terminal resize tolerates an attached session without an active provider run", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      throw new Error("session `session-remote` has no active provider run")
    },
  } as unknown as LocalIpcClient

  await withTerminalDimensions(() => resizeSessionTerminal(client, "session-remote"))

  assert.deepEqual(requests, [{
    ResizeTerminal: {
      session_id: "session-remote",
      provider_run_id: null,
      cols: 120,
      rows: 40,
    },
  }])
})

test("terminal resize remains best-effort when its transport is unavailable", async () => {
  const client = {
    send: async () => {
      throw new Error("relay connection refused")
    },
  } as unknown as LocalIpcClient

  await withTerminalDimensions(() => resizeSessionTerminal(client, "session-remote"))
})

async function withTerminalDimensions<T>(run: () => Promise<T>): Promise<T> {
  const properties = ["isTTY", "columns", "rows"] as const
  const descriptors = Object.fromEntries(properties.map((property) => [
    property,
    Object.getOwnPropertyDescriptor(process.stdout, property),
  ]))
  Object.defineProperties(process.stdout, {
    isTTY: { configurable: true, value: true },
    columns: { configurable: true, value: 120 },
    rows: { configurable: true, value: 40 },
  })
  try {
    return await run()
  } finally {
    for (const property of properties) {
      const descriptor = descriptors[property]
      if (descriptor) Object.defineProperty(process.stdout, property, descriptor)
      else delete (process.stdout as unknown as Record<string, unknown>)[property]
    }
  }
}
