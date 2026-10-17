import assert from "node:assert/strict"
import test from "node:test"

import { createCommandActionHandlers } from "./command-actions.js"
import { makeCommandDeps, makeSession } from "./command-actions-test-support.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createBaseDeps } from "./session-lifecycle.test-support.js"
import { createWaitingRoomPromptBootstrapController } from "./waiting-room-prompt-bootstrap-controller.js"

type Attachment = { id: string; session_id: string }

// Real lifecycle + real /session handlers + real bootstrap controller over one
// stateful view, so the pending window is observed the way the TUI observes it.
function selectionHarness(overrides: Record<string, unknown> = {}, handlerOverrides: Record<string, unknown> = {}) {
  let attachment: Attachment | null = { id: "att-a", session_id: "session-a" }
  let hydrating = false
  const detached: string[] = []
  const starts: string[] = []
  const flashes: string[] = []
  const sessionA = makeSession({ id: "session-a", agents: [] })
  const sessionB = makeSession({ id: "session-b", agents: [] })
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    setAttachmentState: (next: Attachment | null) => { attachment = next },
    setSessionHydratingState: (next: boolean) => { hydrating = next },
    detachAttachment: async (id: string) => { detached.push(id) },
    attachToSession: async (sessionId: string) => ({ id: `att-${sessionId}`, session_id: sessionId }),
    getSessionState: async (sessionId: string) => sessionId === "session-a" ? sessionA : sessionB,
    hydrateAttachedSessionBinding: async (_id: string, _att: string, session: unknown) => session,
    ...overrides,
  })
  const lifecycle = createSessionLifecycleController(deps as never)
  const handlers = createCommandActionHandlers(makeCommandDeps({
    prepareSessionSelection: (reference: string) => lifecycle.prepareSessionSelection({ id: reference }),
    attachBinding: lifecycle.attachBinding,
    resolveSession: async () => sessionB,
    ...handlerOverrides,
  }))
  const bootstrap = createWaitingRoomPromptBootstrapController({
    isAttached: () => attachment !== null,
    isSessionSelecting: () => hydrating,
    startSessionFromWaitingRoomDefaults: async () => { starts.push("created") },
    flashFooter: message => { flashes.push(message) },
  })
  const attach = (ref: string) =>
    handlers.handleSessionCommand({ kind: "session", raw: `/session attach ${ref}`, action: "attach", args: [ref], value: ref })
  return { lifecycle, attach, bootstrap, starts, flashes, detached, attachment: () => attachment }
}

test("MP-08 / MP-10 OWNER-24 Enter during a delayed session switch does not create another session", async () => {
  let release!: () => void
  const slowDetach = new Promise<void>(resolve => { release = resolve })
  const h = selectionHarness({ detachAttachment: () => slowDetach })
  const switching = h.attach("session-b")
  assert.equal(h.attachment(), null)
  assert.equal(await h.bootstrap.bootstrap(), "handled")
  assert.deepEqual(h.starts, [])
  release()
  await switching
  assert.equal(h.attachment()?.session_id, "session-b")
  assert.deepEqual(h.starts, [])
})

test("MP-08 / MP-10 OWNER-24 Enter during a slow reference lookup does not create another session", async () => {
  let release!: () => void
  const slowLookup = new Promise<void>(resolve => { release = resolve })
  const h = selectionHarness({}, { resolveSession: async () => { await slowLookup; return makeSession({ id: "session-b", agents: [] }) } })
  const switching = h.attach("b")
  assert.equal(await h.bootstrap.bootstrap(), "handled")
  assert.deepEqual(h.starts, [])
  release()
  await switching
})

test("MP-08 / MP-10 OWNER-24 a failed reference lookup restores the previous attachment", async () => {
  const h = selectionHarness({}, { resolveSession: async () => { throw new Error("unknown session") } })
  await assert.rejects(h.attach("missing"), /unknown session/)
  assert.equal(h.attachment()?.id, "att-a")
  assert.deepEqual(h.detached, [])
  assert.deepEqual(h.starts, [])
  assert.equal(await h.bootstrap.bootstrap(), "unhandled", "attached again, so the prompt goes to the session")
})

test("MP-08 / MP-10 OWNER-24 a failed detach keeps the previous attachment reachable", async () => {
  const h = selectionHarness({ detachAttachment: async () => { throw new Error("permission denied") } })
  await assert.rejects(h.attach("session-b"), /permission denied/)
  assert.equal(h.attachment()?.id, "att-a")
})

test("MP-08 / MP-10 OWNER-24 a failed restore detaches the retained attachment and shows the waiting room", async () => {
  const h = selectionHarness(
    { getSessionState: async () => { throw new Error("session gone") } },
    { resolveSession: async () => { throw new Error("unknown session") } },
  )
  await assert.rejects(h.attach("missing"), /unknown session/)
  assert.equal(h.attachment(), null)
  assert.deepEqual(h.detached, ["att-a"])
})

test("MP-08 / MP-10 OWNER-24 cleanup sees the attachment retained during a pending switch", async () => {
  let release!: () => void
  const slowLookup = new Promise<void>(resolve => { release = resolve })
  const h = selectionHarness({}, { resolveSession: async () => { await slowLookup; return makeSession({ id: "session-b", agents: [] }) } })
  const switching = h.attach("b")
  assert.equal(h.attachment(), null)
  assert.equal(h.lifecycle.currentAttachment()?.id, "att-a")
  release()
  await switching
  assert.equal(h.lifecycle.currentAttachment()?.id, "att-session-b")
})

test("MP-08 / MP-10 OWNER-24 a stale rollback cannot restore the previous session over a newer selection", async () => {
  let releaseRollback!: () => void
  const rollbackGate = new Promise<void>(resolve => { releaseRollback = resolve })
  const sessionA = makeSession({ id: "session-a", agents: [] })
  const sessionB = makeSession({ id: "session-b", agents: [] })
  let lookups = 0
  const h = selectionHarness(
    {
      getSessionState: async (sessionId: string) => {
        if (sessionId !== "session-a") return sessionB
        await rollbackGate
        return sessionA
      },
    },
    {
      resolveSession: async () => {
        if (lookups++ === 0) throw new Error("unknown session")
        return sessionB
      },
    },
  )
  const failed = h.attach("missing")
  failed.catch(() => {})
  await new Promise(resolve => setImmediate(resolve))
  await h.attach("session-b")
  assert.equal(h.attachment()?.session_id, "session-b")
  releaseRollback()
  await assert.rejects(failed, /unknown session/)
  assert.equal(h.attachment()?.session_id, "session-b", "the superseded rollback must not reapply the previous session")
  assert.deepEqual(h.detached, ["att-a"], "the previous attachment is disposed exactly once")
})
