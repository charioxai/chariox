import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcClient } from "../ipc.js"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"
import type { RuntimeInteraction } from "../cli-types.js"

test("MP-08 MP-10 permissions for agent A are bounded while agent B keeps output backlogged", async () => {
  const harness = installTimerHarness()
  const interaction = { id: "approval-a", agent_id: "agent-a", kind: "permission", native_origin: {
    scope: "prompt", provider_run_id: "run-a", prompt_id: "prompt-a",
  } } as RuntimeInteraction
  let pages = 0
  const projected: RuntimeInteraction[][] = []
  const client = fakeClient(request => requestVariant(request) === "GetSessionState"
    ? { SessionState: { session: { active_interactions: [interaction] }, agent_activity: {} } }
    : { TerminalOutput: { records: (++pages, [{ agent_id: "agent-b", kind: "provider_output", bytes: [66] }]) } })
  const pump = startNativeKernelPumpLoop(client, "session", "attachment", {
    onTerminalRecords: () => {}, onAgentActivity: () => {},
    onInteractions: interactions => { projected.push(interactions) }, pollRuntimeNotices: false,
  })
  try {
    await flushPromises()
    assert.equal(pages, 8, "a tick still bounds its output work")
    assert.deepEqual(projected, [[interaction]], "unrelated session output cannot starve A's approval")
  } finally { pump.stop(); harness.restore() }
})

test("MP-08 MP-10 native kernel pump drives home settlement when the provider renders output", async () => {
  const harness = installTimerHarness()
  const requests: string[] = []
  const client = fakeClient((request) => {
    requests.push(requestVariant(request))
    return requestVariant(request) === "PumpTerminalOutput" ? { TerminalOutput: { records: [] } } : { RuntimeNotices: { notices: [] } }
  })

  try {
    const pump = startNativeKernelPumpLoop(client, "session-1", "attachment-1")
    await flushPromises()

    assert.equal(harness.intervalMs(), 250)
    assert.deepEqual(requests, ["PumpTerminalOutput", "PollRuntimeNotices"])
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

test("MP-08 MP-10 native pump drains final output after snapshot and before settlement", async () => {
  const harness = installTimerHarness()
  const order: string[] = []
  const client = fakeClient(request => {
    const kind = requestVariant(request)
    order.push(kind)
    return kind === "GetSessionState"
      ? { SessionState: { agent_activity: { agent: {} } } }
      : { TerminalOutput: { records: order.filter(k => k === "PumpTerminalOutput").length === 1 ? [{ kind: "provider_output" }] : [] } }
  })
  const pump = startNativeKernelPumpLoop(client, "session", "attachment", {
    onTerminalRecords: () => { order.push("output") },
    onAgentActivity: () => { order.push("settlement") },
    pollRuntimeNotices: false,
  })
  try {
    await flushPromises()
    assert.deepEqual(order, ["GetSessionState", "PumpTerminalOutput", "output", "PumpTerminalOutput", "settlement"])
  } finally { pump.stop(); harness.restore() }
})

test("MP-08 MP-10 stopped native pump suppresses an in-flight snapshot callback", async () => {
  const harness = installTimerHarness()
  let release!: (value: unknown) => void
  let calls = 0
  const client = { send: () => new Promise(resolve => { release = resolve }) } as unknown as LocalIpcClient
  const pump = startNativeKernelPumpLoop(client, "session", "attachment", {
    onTerminalRecords: () => { calls++ }, onAgentActivity: () => { calls++ }, pollRuntimeNotices: false,
  })
  try {
    pump.stop()
    release({ SessionState: { agent_activity: {} } })
    await flushPromises()
    assert.equal(calls, 0)
  } finally { pump.stop(); harness.restore() }
})

test("MP-08 MP-10 native settlement waits for every bounded terminal output page", async () => {
  const harness = installTimerHarness()
  const order: string[] = []
  let page = 0
  const client = fakeClient(request => requestVariant(request) === "GetSessionState"
    ? { SessionState: { agent_activity: { agent: {} } } }
    : { TerminalOutput: { records: ++page <= 2 ? [{ kind: "provider_output" }] : [] } })
  const pump = startNativeKernelPumpLoop(client, "session", "attachment", {
    onTerminalRecords: () => { order.push("output") }, onAgentActivity: () => { order.push("settlement") }, pollRuntimeNotices: false,
  })
  try { await flushPromises(); assert.deepEqual(order, ["output", "output", "settlement"]) }
  finally { pump.stop(); harness.restore() }
})
