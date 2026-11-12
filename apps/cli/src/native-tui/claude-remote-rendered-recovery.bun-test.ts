import assert from "node:assert/strict"
import test from "node:test"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcError } from "@chariox/kernel-client/ipc"
import {
  sendClaudeRemoteRenderedInputWithRecovery,
  forwardClaudeRemoteRenderedStdin,
  startClaudeRemoteRenderedPumpLoop,
} from "./claude-remote-rendered-io.js"
import type { RemoteAttachmentRecovery, RemoteAttachmentRecoveryOutcome } from "./remote-attachment-recovery.js"

type Client = Parameters<typeof startClaudeRemoteRenderedPumpLoop>[0]

function attachmentNotFound(id: string): LocalIpcError {
  return new LocalIpcError("handle kernel response", `attachment ${id} gone`, "attachment_not_found", false)
}

// RR-07: a remote native-TUI attachment carried through the gateway lapses after a >30s
// relay outage (the target sweeps it). The pump must re-attach on a definite
// attachment_not_found, adopt the new id, and resume (the fresh attachment's catch-up
// redraws the screen).
test("native remote pump re-attaches after attachment_not_found and resumes output", async () => {
  const attachment = { id: "att-1" }
  const delivered: unknown[] = []
  let reattachCalls = 0

  const client = {
    async send(request: Record<string, { attachment_id?: string }>) {
      const id = request?.PumpTerminalOutput?.attachment_id
      if (id === "att-2") return { TerminalOutput: { records: [{ marker: "after-recovery" }] } }
      throw attachmentNotFound(String(id))
    },
  } as unknown as Client

  const recovery: RemoteAttachmentRecovery = {
    recover: async (): Promise<RemoteAttachmentRecoveryOutcome> => {
      reattachCalls += 1
      attachment.id = "att-2"
      return "recovered"
    },
    settlePendingSubscription: async () => {},
  }

  const pump = startClaudeRemoteRenderedPumpLoop(
    client,
    "sess-1",
    attachment,
    "run-1",
    "agent-1",
    recovery,
    (records) => {
      if (Array.isArray(records) && records.length) delivered.push(...records)
    },
  )

  const start = Date.now()
  while (delivered.length === 0 && Date.now() - start < 4_000) await sleep(25)
  pump.stop()

  assert.equal(reattachCalls, 1, "re-attached exactly once on attachment_not_found")
  assert.equal(attachment.id, "att-2", "pump adopted the recovered attachment id")
  assert.ok(delivered.length >= 1, "terminal output resumed after recovery")
})

test("native remote pump stops when recovery is fatal", async () => {
  const attachment = { id: "att-1" }
  let sends = 0
  const client = {
    async send() {
      sends += 1
      throw attachmentNotFound("att-1")
    },
  } as unknown as Client

  const recovery: RemoteAttachmentRecovery = {
    recover: async () => "fatal",
    settlePendingSubscription: async () => {},
  }

  const pump = startClaudeRemoteRenderedPumpLoop(client, "sess-1", attachment, "run-1", "agent-1", recovery)
  await sleep(600)
  const sendsAtFatal = sends
  await sleep(600)
  pump.stop()
  assert.equal(sends, sendsAtFatal, "the pump stopped sending after a fatal recovery outcome")
})

// Fix 1 (transactional subscription): a subscription deferred during recovery must be
// re-established by the pump's per-tick settle even while polls keep succeeding.
test("native remote pump settles a deferred subscription on later ticks", async () => {
  const attachment = { id: "att-1" }
  let settleCalls = 0
  const client = {
    async send() {
      return { TerminalOutput: { records: [] } }
    },
  } as unknown as Client

  const recovery: RemoteAttachmentRecovery = {
    recover: async () => "recovered",
    settlePendingSubscription: async () => {
      settleCalls += 1
    },
  }

  const pump = startClaudeRemoteRenderedPumpLoop(client, "sess-1", attachment, "run-1", "agent-1", recovery)
  await sleep(700)
  pump.stop()
  assert.ok(settleCalls >= 2, "the pump settles the pending subscription every tick")
})

// Fix 2 (stdin): a key refused with attachment_not_found drives recovery and is replayed
// exactly once under the new attachment id (a fresh send => a fresh command id).
test("a refused keystroke recovers and replays once on the new attachment", async () => {
  const attachment = { id: "att-1" }
  const sends: string[] = []
  const send = async () => {
    sends.push(attachment.id)
    if (attachment.id === "att-1") throw attachmentNotFound("att-1")
  }
  await sendClaudeRemoteRenderedInputWithRecovery(send, async () => {
    attachment.id = "att-2"
    return "recovered"
  })
  assert.deepEqual(sends, ["att-1", "att-2"], "replayed once on the recovered attachment")
})

test("a refused keystroke is dropped, not replayed, on a transient or fatal outcome", async () => {
  for (const outcome of ["transient", "fatal"] as const) {
    const sends: string[] = []
    const send = async () => {
      sends.push("x")
      throw attachmentNotFound("att-1")
    }
    await sendClaudeRemoteRenderedInputWithRecovery(send, async () => outcome)
    assert.equal(sends.length, 1, `${outcome} drops the key; a mutation is never retried blindly`)
  }
})


test("MP-08 RR-07 stdin recovery never replays the accepted prefix of a pasted prompt", async () => {
  const attachment = {id:"old"}, accepted: string[] = [];
  let refused = false;
  const client = {send: async (request: any) => {
    const input = request.SendTerminalInput;
    if (!input) return {};
    const text = Buffer.from(input.data_base64,"base64").toString();
    if (text === "\r" && !refused) {refused = true;throw attachmentNotFound("old")}
    accepted.push(text);return {};
  }} as unknown as Client;
  const paused = process.stdin.isPaused();
  const restore = forwardClaudeRemoteRenderedStdin({client, sessionId:"room", attachment,
    agentId:"agent",providerRunId:"run",worktree:"/tmp",inlineLocalAttachments:false,
    debug:()=>{},recover:async()=>{attachment.id="new";return "recovered"}});
  try {
    process.stdin.emit("data",Buffer.from("hello\r"));
    for(let i=0;i<100 && !accepted.includes("\r");i++)await sleep(5);
    assert.deepEqual(accepted,["hello","\r"],"only the refused Enter may be sent again");
  } finally {restore();if(paused)process.stdin.pause()}
});
