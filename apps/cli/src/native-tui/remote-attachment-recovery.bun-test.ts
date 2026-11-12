import assert from "node:assert/strict"
import test from "node:test"

import { LocalIpcError } from "@chariox/kernel-client/ipc"
import { createRemoteAttachmentRecovery, type RemoteAttachmentRecoveryOptions } from "./remote-attachment-recovery.js"

function kernelError(code: string, retryable = false): LocalIpcError {
  return new LocalIpcError("handle kernel response", `${code} from the target`, code, retryable)
}

function recovery(overrides: Partial<RemoteAttachmentRecoveryOptions> & { attachment: { id: string } }) {
  return createRemoteAttachmentRecovery({
    reattach: async () => "att-2",
    resubscribe: async () => {},
    isActive: () => true,
    detach: async () => {},
    ...overrides,
  })
}

test("an exact attachment_not_found re-attaches, adopts the new id and re-subscribes", async () => {
  const attachment = { id: "att-1" }
  const resubscribed: string[] = []
  let reattachCalls = 0
  const { recover } = recovery({
    attachment,
    reattach: async () => {
      reattachCalls += 1
      return "att-2"
    },
    resubscribe: async (id) => {
      resubscribed.push(id)
    },
  })

  assert.equal(await recover(kernelError("attachment_not_found")), "recovered")
  assert.equal(attachment.id, "att-2")
  assert.equal(reattachCalls, 1)
  assert.deepEqual(resubscribed, ["att-2"])
})

test("a retryable transport failure is transient and never re-attaches a mutation", async () => {
  const attachment = { id: "att-1" }
  let reattachCalls = 0
  const { recover } = recovery({
    attachment,
    reattach: async () => {
      reattachCalls += 1
      return "att-2"
    },
  })

  assert.equal(await recover(kernelError("request_timeout", true)), "transient")
  assert.equal(await recover(kernelError("connection_closed", true)), "transient")
  assert.equal(reattachCalls, 0)
  assert.equal(attachment.id, "att-1")
})

test("a non-retryable refusal that is not attachment_not_found is fatal and is not re-attached", async () => {
  const attachment = { id: "att-1" }
  let reattachCalls = 0
  for (const code of ["unauthorized", "access_denied", "session_not_found"]) {
    let fatal: unknown = null
    const { recover } = recovery({
      attachment,
      reattach: async () => {
        reattachCalls += 1
        return "att-2"
      },
      onFatal: (error) => {
        fatal = error
      },
    })
    assert.equal(await recover(kernelError(code)), "fatal", `${code} is fatal`)
    assert.ok(fatal instanceof LocalIpcError)
  }
  assert.equal(reattachCalls, 0, "a definite refusal never opens a new attachment")
})

test("the session being gone during re-attach is fatal and leaves the id unchanged", async () => {
  const attachment = { id: "att-1" }
  const { recover } = recovery({
    attachment,
    reattach: async () => {
      throw kernelError("session_not_found")
    },
  })

  assert.equal(await recover(kernelError("attachment_not_found")), "fatal")
  assert.equal(attachment.id, "att-1")
})

test("a transport failure while re-attaching stays transient so the caller retries", async () => {
  const attachment = { id: "att-1" }
  const { recover } = recovery({
    attachment,
    reattach: async () => {
      throw kernelError("connection_closed", true)
    },
  })

  assert.equal(await recover(kernelError("attachment_not_found")), "transient")
  assert.equal(attachment.id, "att-1")
})

test("a new attachment opened after the surface is torn down is detached, not leaked", async () => {
  const attachment = { id: "att-1" }
  let active = true
  const detached: string[] = []
  const { recover } = recovery({
    attachment,
    isActive: () => active,
    reattach: async () => {
      active = false // the user exits while the re-attach is in flight
      return "att-2"
    },
    detach: async (id) => {
      detached.push(id)
    },
  })

  assert.equal(await recover(kernelError("attachment_not_found")), "fatal")
  assert.deepEqual(detached, ["att-2"], "the orphaned attachment is released")
})

test("a failed re-subscribe keeps the new id and is retried on the next pump tick", async () => {
  const attachment = { id: "att-1" }
  const recovered: string[] = []
  let subscribeAttempts = 0
  const { recover, settlePendingSubscription } = recovery({
    attachment,
    reattach: async () => "att-2",
    resubscribe: async (id) => {
      subscribeAttempts += 1
      if (subscribeAttempts === 1) throw kernelError("connection_closed", true)
      assert.equal(id, "att-2")
    },
    onRecovered: (id) => recovered.push(id),
  })

  // Recovery adopts the id (the pump already reads it) even though the subscription failed;
  // it is tracked pending, not reported as fully settled.
  assert.equal(await recover(kernelError("attachment_not_found")), "recovered")
  assert.equal(attachment.id, "att-2")
  assert.equal(subscribeAttempts, 1)
  assert.deepEqual(recovered, [])

  // The pump's next tick re-establishes the subscription: never left permanently dead.
  await settlePendingSubscription()
  assert.equal(subscribeAttempts, 2)
  assert.deepEqual(recovered, ["att-2"])

  // Nothing pending now.
  await settlePendingSubscription()
  assert.equal(subscribeAttempts, 2)
})

test("concurrent lapses share one in-flight re-attach", async () => {
  const attachment = { id: "att-1" }
  let reattachCalls = 0
  let release: () => void = () => {}
  const gate = new Promise<void>((resolve) => {
    release = resolve
  })
  const { recover } = recovery({
    attachment,
    reattach: async () => {
      reattachCalls += 1
      await gate
      return "att-2"
    },
  })

  const first = recover(kernelError("attachment_not_found"))
  const second = recover(kernelError("attachment_not_found"))
  release()
  assert.deepEqual(await Promise.all([first, second]), ["recovered", "recovered"])
  assert.equal(reattachCalls, 1)
})
