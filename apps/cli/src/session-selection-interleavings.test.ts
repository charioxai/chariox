import assert from "node:assert/strict"
import test from "node:test"

import { createCommandActionHandlers } from "./command-actions.js"
import { makeCommandDeps, makeSession } from "./command-actions-test-support.js"
import { createSessionLifecycleController } from "./session-lifecycle.js"
import { createBaseDeps } from "./session-lifecycle.test-support.js"

type Attachment = { id: string; session_id: string }
type Op = { label: string; canFail: boolean; settle: (fail: boolean) => void }

const flush = () => new Promise(resolve => setImmediate(resolve))

// One world per explored path: every kernel round trip of a selection (lookup,
// detach, attach, state read) waits until the explorer completes it.
function createWorld(targets: string[], attached: boolean) {
  const pending: Op[] = []
  const live = new Set(attached ? ["att-a"] : [])
  const errors: string[] = []
  let view: Attachment | null = attached ? { id: "att-a", session_id: "session-a" } : null
  // The session the screen shows: a placeholder, an attached session or none.
  let shown = view?.session_id ?? "no-session"
  let attachSeq = 0
  const op = <T>(label: string, canFail: boolean, value: () => T) =>
    new Promise<T>((resolve, reject) => {
      pending.push({ label, canFail, settle: fail => fail ? reject(new Error(`${label} failed`)) : resolve(value()) })
    })
  const session = (id: string) => makeSession({ id, agents: [] })
  const { deps } = createBaseDeps({
    attachmentState: () => view,
    setAttachmentState: (next: Attachment | null) => { view = next },
    setSessionState: (next: { id: string }) => { shown = next.id },
    detachAttachment: (id: string) => op(`detach ${id}`, id === "att-a", () => {
      if (!live.delete(id)) errors.push(`detached ${id} twice`)
    }),
    attachToSession: (sessionId: string) => op(`attach ${sessionId}`, true, () => {
      const id = `att-${sessionId}-${++attachSeq}`
      live.add(id)
      return { id, session_id: sessionId }
    }),
    getSessionState: (sessionId: string) => op(`state ${sessionId}`, true, () => session(sessionId)),
    hydrateAttachedSessionBinding: async (_id: string, _att: string, value: unknown) => value,
  })
  const lifecycle = createSessionLifecycleController(deps as never)
  const handlers = createCommandActionHandlers(makeCommandDeps({
    beginSessionSelection: (reference: string) => lifecycle.beginSessionSelection({ id: reference }),
    attachBinding: lifecycle.attachBinding,
    resolveSession: (reference: string) => op(`lookup ${reference}`, true, () => session(reference)),
  }))
  const selections: Array<{ target: string; shownAtBegin: string | null; done: boolean; error: unknown }> = []
  const start = () => {
    const target = targets[selections.length]!
    const selection = { target, shownAtBegin: lifecycle.currentAttachment()?.session_id ?? null, done: false, error: null as unknown }
    selections.push(selection)
    handlers.handleSessionCommand({ kind: "session", raw: `/session attach ${target}`, action: "attach", args: [target], value: target })
      .catch(error => { selection.error = error })
      .finally(() => { selection.done = true })
  }
  return {
    pending, live, errors, selections, start,
    view: () => view,
    shown: () => shown,
    selecting: () => lifecycle.isSessionSelecting(),
  }
}

type Choice = { kind: "start" } | { kind: "settle"; index: number; fail: boolean }

function choicesFor(world: ReturnType<typeof createWorld>, targets: string[], failuresLeft: number): Choice[] {
  const choices: Choice[] = []
  if (world.selections.length < targets.length) choices.push({ kind: "start" })
  world.pending.forEach((op, index) => {
    choices.push({ kind: "settle", index, fail: false })
    if (op.canFail && failuresLeft > 0) choices.push({ kind: "settle", index, fail: true })
  })
  return choices
}

// The newest selection owns the view: until it ends, the view is its
// placeholder, its own session, or the session it restores; Enter cannot
// bootstrap a new session while nothing is attached.
function checkStep(world: ReturnType<typeof createWorld>, trace: string[]) {
  const newest = world.selections.at(-1)
  if (!newest || newest.done) return
  const shown = world.shown()
  assert.ok(
    shown === newest.target || shown === newest.shownAtBegin,
    `screen shows ${shown} while ${newest.target} is pending\n${trace.join("\n")}`,
  )
  const view = world.view()
  if (view === null) {
    assert.ok(world.selecting(), `selection guard dropped while ${newest.target} is pending\n${trace.join("\n")}`)
  } else {
    assert.ok(
      view.session_id === newest.target || view.session_id === newest.shownAtBegin,
      `stale view ${view.session_id} while ${newest.target} is pending\n${trace.join("\n")}`,
    )
  }
}

function checkEnd(world: ReturnType<typeof createWorld>, failed: string[], trace: string[]) {
  const where = `\n${trace.join("\n")}`
  assert.ok(world.selections.every(selection => selection.done), `a selection never settled${where}`)
  assert.deepEqual(world.errors, [], where)
  assert.equal(world.selecting(), false, `selection guard left on${where}`)
  const newest = world.selections.at(-1)!
  const view = world.view()
  const newestUntouched = failed.every(label => !label.includes(newest.target) && !label.startsWith("detach"))
  if (newestUntouched) {
    assert.equal(view?.session_id, newest.target, `newest selection lost${where}`)
  } else {
    assert.ok(
      view === null || view.session_id === newest.target || view.session_id === newest.shownAtBegin,
      `stale final view ${view?.session_id}${where}`,
    )
  }
  assert.deepEqual([...world.live], view ? [view.id] : [], `kernel attachments do not match the view${where}`)
  assert.equal(world.shown(), view?.session_id ?? "no-session", `screen does not match the view${where}`)
}

async function runPath(targets: string[], maxFailures: number, attached: boolean, prefix: number[]) {
  const world = createWorld(targets, attached)
  const taken: number[] = []
  const widths: number[] = []
  const failed: string[] = []
  const trace: string[] = []
  for (let step = 0; step < 200; step += 1) {
    await flush()
    checkStep(world, trace)
    const choices = choicesFor(world, targets, maxFailures - failed.length)
    if (choices.length === 0) {
      checkEnd(world, failed, trace)
      return { taken, widths }
    }
    const index = step < prefix.length ? prefix[step]! : 0
    taken.push(index)
    widths.push(choices.length)
    const choice = choices[index]!
    if (choice.kind === "start") {
      trace.push(`start ${targets[world.selections.length]}`)
      world.start()
    } else {
      const [op] = world.pending.splice(choice.index, 1)
      trace.push(`${choice.fail ? "fail" : "complete"} ${op!.label}`)
      if (choice.fail) failed.push(op!.label)
      op!.settle(choice.fail)
    }
  }
  throw new Error(`path did not finish\n${trace.join("\n")}`)
}

async function explore(targets: string[], maxFailures: number, attached = true) {
  const stack: number[][] = [[]]
  let paths = 0
  while (stack.length > 0) {
    const prefix = stack.pop()!
    const { taken, widths } = await runPath(targets, maxFailures, attached, prefix)
    paths += 1
    for (let step = prefix.length; step < widths.length; step += 1) {
      for (let choice = 1; choice < widths[step]!; choice += 1) {
        stack.push([...taken.slice(0, step), choice])
      }
    }
  }
  return paths
}

test("MP-08 / MP-10 OWNER-24 every interleaving of two overlapping selections ends on the newest or a clean detach", async t => {
  const paths = await explore(["session-b", "session-c"], 2)
  t.diagnostic(`explored ${paths} paths`)
  assert.ok(paths > 50, `explored ${paths} paths`)
})

test("MP-08 / MP-10 OWNER-24 a lookup of the attached session cannot reopen it over a newer selection", async t => {
  const paths = await explore(["session-a", "session-b"], 2)
  t.diagnostic(`explored ${paths} paths`)
  assert.ok(paths > 20, `explored ${paths} paths`)
})

test("MP-08 / MP-10 OWNER-24 selections from the waiting room show their own placeholder", async t => {
  const paths = await explore(["session-b", "session-c"], 2, false)
  t.diagnostic(`explored ${paths} paths`)
  assert.ok(paths > 20, `explored ${paths} paths`)
})

test("MP-08 / MP-10 OWNER-24 every interleaving of three overlapping selections ends on the newest or a clean detach", async t => {
  const paths = await explore(["session-b", "session-c", "session-d"], 0)
  t.diagnostic(`explored ${paths} paths`)
  assert.ok(paths > 50, `explored ${paths} paths`)
})
