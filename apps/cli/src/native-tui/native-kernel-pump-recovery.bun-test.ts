import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { startNativeKernelPumpLoop } from "./native-kernel-pump.js"
import type { RemoteAttachmentRecovery } from "./remote-attachment-recovery.js"

type Client = Parameters<typeof startNativeKernelPumpLoop>[0]

function attachmentNotFound(id: string): LocalIpcError {
  return new LocalIpcError("handle kernel response", `attachment ${id} gone`, "attachment_not_found", false)
}

// RR-07 (Codex/OpenCode native pump): the swept attachment is re-attached, the pump adopts
// the new id, and new output resumes on the existing display.
test("native-kernel pump re-attaches after attachment_not_found and resumes output", async () => {
  const attachment = { id: "att-1" }
  const delivered: unknown[] = []
  let reattachCalls = 0

  const client = {
    async send(request: Record<string, { attachment_id?: string }>) {
      if (!("PumpTerminalOutput" in request)) return {}
      const id = request.PumpTerminalOutput?.attachment_id
      if (id === "att-2") return { TerminalOutput: { records: [{ bytes: [1], kind: "provider_output" }] } }
      throw attachmentNotFound(String(id))
    },
  } as unknown as Client

  const recovery: RemoteAttachmentRecovery = {
    recover: async () => {
      reattachCalls += 1
      attachment.id = "att-2"
      return "recovered"
    },
    settlePendingSubscription: async () => {},
  }

  const pump = startNativeKernelPumpLoop(client, "sess-1", attachment, {
    onTerminalRecords: (records) => {
      if (records.length) delivered.push(...records)
    },
    pollRuntimeNotices: false,
    recovery,
  })

  const start = Date.now()
  while (delivered.length === 0 && Date.now() - start < 4_000) await sleep(25)
  pump.stop()

  assert.equal(reattachCalls, 1, "re-attached once on attachment_not_found")
  assert.equal(attachment.id, "att-2", "pump adopted the recovered attachment id")
  assert.ok(delivered.length >= 1, "output resumed after recovery")
})

test("native-kernel pump stops when recovery is fatal", async () => {
  const attachment = { id: "att-1" }
  let sends = 0
  const client = {
    async send(request: Record<string, unknown>) {
      if ("PumpTerminalOutput" in request) {
        sends += 1
        throw attachmentNotFound("att-1")
      }
      return {}
    },
  } as unknown as Client

  const recovery: RemoteAttachmentRecovery = {
    recover: async () => "fatal",
    settlePendingSubscription: async () => {},
  }

  const pump = startNativeKernelPumpLoop(client, "sess-1", attachment, { pollRuntimeNotices: false, recovery })
  await sleep(600)
  const sendsAtFatal = sends
  await sleep(600)
  pump.stop()
  assert.equal(sends, sendsAtFatal, "the pump stopped sending after a fatal recovery outcome")
})
