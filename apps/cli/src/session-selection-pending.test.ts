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
  const detached: string[] = []
  const starts: string[] = []
  const flashes: string[] = []
  const sessionA = makeSession({ id: "session-a", agents: [] })
  const sessionB = makeSession({ id: "session-b", agents: [] })
  const { deps } = createBaseDeps({
    attachmentState: () => attachment,
    setAttachmentState: (next: Attachment | null) => { attachment = next },
    detachAttachment: async (id: string) => { detached.push(id) },
    attachToSession: async (sessionId: string) => ({ id: `att-${sessionId}`, session_id: sessionId }),
    getSessionState: async (sessionId: string) => sessionId === "session-a" ? sessionA : sessionB,
    hydrateAttachedSessionBinding: async (_id: string, _att: string, session: unknown) => session,
    ...overrides,
  })
  const lifecycle = createSessionLifecycleController(deps as never)
  const handlers = createCommandActionHandlers(makeCommandDeps({
    beginSessionSelection: (reference: string) => lifecycle.beginSessionSelection({ id: reference }),
    attachBinding: lifecycle.attachBinding,
    resolveSession: async () => sessionB,
    ...handlerOverrides,
  }))
  const bootstrap = createWaitingRoomPromptBootstrapController({
    isAttached: () => attachment !== null,
    isSessionSelecting: lifecycle.isSessionSelecting,
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
  // Kernel work runs one selection at a time: the newer one shows its
  // placeholder now and attaches once the superseded restore unwinds.
  const next = h.attach("session-b")
  assert.equal(h.attachment(), null)
  assert.equal(await h.bootstrap.bootstrap(), "handled")
  releaseRollback()
  await next
  await assert.rejects(failed, /unknown session/)
  assert.equal(h.attachment()?.session_id, "session-b", "the superseded rollback must not reapply the previous session")
  assert.deepEqual(h.detached, ["att-a"], "the previous attachment is disposed exactly once")
})

// BOUNDED RULE: every async outcome of a selection or restore applies only while
// its generation is current. One row per outcome kind.
type Gate = { promise: Promise<void>; release: () => void }
const gate = (): Gate => {
  let release!: () => void
  const promise = new Promise<void>(resolve => { release = resolve })
  return { promise, release }
}
const tick = () => new Promise(resolve => setImmediate(resolve))

const generationOutcomes: Array<{
  name: string
  run: () => Promise<void>
}> = [
  {
    name: "a lookup failing after a newer selection started leaves the newer selection in charge",
    run: async () => {
      const first = gate()
      const second = gate()
      const sessionB = makeSession({ id: "session-b", agents: [] })
      let lookups = 0
      const h = selectionHarness({}, {
        resolveSession: async () => {
          if (lookups++ === 0) { await first.promise; throw new Error("unknown session") }
          await second.promise
          return sessionB
        },
      })
      const failed = h.attach("missing")
      failed.catch(() => {})
      await tick()
      const next = h.attach("b")
      await tick()
      first.release()
      await failed // a superseded lookup failure is not reported
      assert.equal(h.attachment(), null, "the stale failure must not restore the previous session")
      assert.equal(await h.bootstrap.bootstrap(), "handled", "the newer selection still blocks bootstrap")
      second.release()
      await next
      assert.equal(h.attachment()?.session_id, "session-b")
    },
  },
  {
    name: "a failed restore cleanup finishing after a newer attach keeps the newer attachment",
    run: async () => {
      const cleanup = gate()
      const sessionB = makeSession({ id: "session-b", agents: [] })
      let lookups = 0
      const h = selectionHarness(
        {
          getSessionState: async (sessionId: string) => {
            if (sessionId === "session-a") throw new Error("session gone")
            return sessionB
          },
          detachAttachment: async (id: string) => { if (id === "att-a") await cleanup.promise },
        },
        { resolveSession: async () => { if (lookups++ === 0) throw new Error("unknown session"); return sessionB } },
      )
      const failed = h.attach("missing")
      failed.catch(() => {})
      await tick()
      await tick()
      const next = h.attach("session-b")
      await tick()
      cleanup.release()
      await next
      await assert.rejects(failed, /unknown session/)
      assert.equal(h.attachment()?.session_id, "session-b", "the stale cleanup must not clear the newer view")
    },
  },
  {
    name: "a current failed restore still detaches and shows the waiting room",
    run: async () => {
      const h = selectionHarness(
        { getSessionState: async () => { throw new Error("session gone") } },
        { resolveSession: async () => { throw new Error("unknown session") } },
      )
      await assert.rejects(h.attach("missing"), /unknown session/)
      assert.equal(h.attachment(), null)
      assert.deepEqual(h.detached, ["att-a"])
    },
  },
  {
    name: "a current failed lookup restores the previous session",
    run: async () => {
      const h = selectionHarness({}, { resolveSession: async () => { throw new Error("unknown session") } })
      await assert.rejects(h.attach("missing"), /unknown session/)
      assert.equal(h.attachment()?.id, "att-a")
    },
  },
]

generationOutcomes.push(
  {
    name: "Enter while a failed detach restores the previous session starts nothing",
    run: async () => {
      const restore = gate()
      const sessionA = makeSession({ id: "session-a", agents: [] })
      const h = selectionHarness({
        detachAttachment: async () => { throw new Error("permission denied") },
        getSessionState: async (sessionId: string) => {
          if (sessionId === "session-a") await restore.promise
          return sessionId === "session-a" ? sessionA : makeSession({ id: sessionId, agents: [] })
        },
      })
      const failed = h.attach("session-b")
      failed.catch(() => {})
      await tick()
      await tick()
      assert.equal(h.attachment(), null, "the restore is still loading")
      assert.equal(await h.bootstrap.bootstrap(), "handled")
      assert.deepEqual(h.starts, [])
      restore.release()
      await assert.rejects(failed, /permission denied/)
      assert.equal(h.attachment()?.id, "att-a")
      assert.equal(h.lifecycle.isSessionSelecting(), false)
    },
  },
  {
    name: "an older attach failing while a newer lookup is pending keeps the guard",
    run: async () => {
      const attach = gate()
      const lookup = gate()
      const h = selectionHarness(
        { attachToSession: async () => { await attach.promise; throw new Error("attach refused") } },
        { resolveSession: async (reference: string) => {
          if (reference === "c") await lookup.promise
          return makeSession({ id: `session-${reference}`, agents: [] })
        } },
      )
      const older = h.attach("b")
      older.catch(() => {})
      await tick()
      const newer = h.attach("c")
      await tick()
      attach.release()
      await older
      assert.equal(h.attachment(), null)
      assert.equal(await h.bootstrap.bootstrap(), "handled", "the newer lookup still owns the selection")
      assert.deepEqual(h.starts, [])
      lookup.release()
      await assert.rejects(newer, /attach refused/)
      assert.equal(h.attachment(), null, "the older switch already detached att-a, so the waiting room shows")
      assert.equal(h.lifecycle.isSessionSelecting(), false)
    },
  },
)

for (const outcome of generationOutcomes) {
  test(`MP-08 / MP-10 OWNER-24 generation rule: ${outcome.name}`, outcome.run)
}

test("MP-08 / MP-10 OWNER-24 leaving for the waiting room during a lookup drops the retained binding", async () => {
  let release!: () => void
  const slowLookup = new Promise<void>(resolve => { release = resolve })
  const attaches: string[] = []
  const h = selectionHarness({
    attachToSession: async (sessionId: string) => {
      attaches.push(sessionId)
      return { id: `att-${sessionId}-${attaches.length}`, session_id: sessionId }
    },
  }, { resolveSession: async () => { await slowLookup; return makeSession({ id: "session-b", agents: [] }) } })
  const switching = h.attach("b")
  const left = h.lifecycle.currentAttachment()
  assert.equal(left?.id, "att-a")
  await h.lifecycle.transitionToNoSession()
  assert.equal(h.lifecycle.currentAttachment(), null, "nothing retained after leaving")
  await h.lifecycle.attachBinding({ id: "session-a" } as never, false)
  assert.deepEqual(attaches, ["session-a"], "reopening attaches fresh instead of restoring the detached binding")
  assert.equal(h.attachment()?.id, "att-session-a-1")
  release()
  await switching
  assert.equal(h.attachment()?.session_id, "session-a", "the abandoned selection does not take over")
})

test("MP-08 / MP-10 OWNER-24 transcript priming carries the selection ownership", async () => {
  let release!: () => void
  const primed = new Promise<void>(resolve => { release = resolve })
  const owners: Array<() => boolean> = []
  const h = selectionHarness({
    primeAttachedSessionBinding: async (session: { id: string }, owned: () => boolean) => {
      if (session.id !== "session-b") return
      owners.push(owned)
      await primed
    },
  })
  const first = h.attach("session-b")
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(owners.length, 1)
  assert.equal(owners[0]!(), true)
  const second = h.lifecycle.attachBinding({ id: "session-c" } as never, false)
  assert.equal(owners[0]!(), false, "a newer selection takes the priming away")
  release()
  await Promise.all([first, second])
  assert.equal(h.attachment()?.session_id, "session-c")
})
