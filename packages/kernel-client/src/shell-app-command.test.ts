import assert from "node:assert/strict"
import { test } from "node:test"
import { executeAppCommand } from "./shell-app-command.js"

test("App list preserves large generations and pages without unbounded collection", async () => {
  const requests: Record<string, unknown>[] = []
  const result = await executeAppCommand(["list", "--limit", "1"], { send: async request => {
    requests.push(request)
    return { AppInstallationsListed: { installations: [{ installation_id: "todo", app_id: "com.chariox.todo", generation: "9223372036854775807", active_release: null, pending_generation: "9223372036854775806", admission_paused: true }], next_cursor: "todo" } }
  } })
  assert.equal(requests.length, 1)
  assert.deepEqual(requests[0], { ListAppInstallations: { after: null, limit: 1 } })
  assert.equal(result.ok, true)
  assert.match(result.message!, /generation 9223372036854775807/)
  assert.match(result.message!, /Next page: app list --after "todo"/)
})

test("App command rejects unsupported or malformed arguments before sending", async () => {
  for (const args of [["list", "--limit", "101"], ["list", "--limit", "1e2"], ["list", "--after"], ["list", "--limit", "1", "--limit", "2"], ["status"], ["status", "todo", "extra"], ["install", "/host/file"]]) {
    const result = await executeAppCommand(args, { send: async () => { throw new Error("unexpected request") } })
    assert.equal(result.ok, false)
  }
})

test("App status and journal use the shared requests and stable errors", async () => {
  for (const [action, variant] of [["status", "GetAppInstallation"], ["journal", "GetAppInstallationJournal"]] as const) {
    const result = await executeAppCommand([action, "todo"], { send: async request => {
      assert.deepEqual(request, { [variant]: { installation_id: "todo" } })
      return { AppRequestFailed: { code: "not_found" } }
    } })
    assert.equal(result.ok, false)
    assert.equal(result.message, "App installation not found.")
  }
})

test("App worker control uses owner-free requests and shows dormant Apps", async () => {
  const worker = { installation_id: "todo", phase: "dormant", enabled: true, failure: null, updated_at_ms: 1 }
  for (const [args, request] of [
    [["worker", "todo"], { GetAppWorker: { installation_id: "todo" } }],
    [["restart", "todo"], { ControlAppWorker: { installation_id: "todo", action: "restart" } }],
    [["stop", "todo"], { ControlAppWorker: { installation_id: "todo", action: "stop" } }],
  ] as const) {
    const result = await executeAppCommand([...args], { send: async sent => {
      assert.deepEqual(sent, request)
      return { AppWorker: { worker } }
    } })
    assert.equal(result.ok, true)
    assert.equal(result.message, "todo · dormant")
  }
})

test("App automation commands route one event to one workflow and validate arguments", async () => {
  const automation = { automation_id: "reminders", revision: 1, event_name: "todo_due", event_version: 1, session_id: "s", publication_id: "p", endpoint_id: "e", queue_id: "q", scheduled: true, status: "active" }
  const result = await executeAppCommand(["automation", "add", "todo", "reminders", "todo_due", "s", "todo-flow", "--scheduled", "--queue", "main"], { send: async sent => {
    assert.deepEqual(sent, { ConfigureAppAutomation: { installation_id: "todo", automation_id: "reminders", expected_revision: 0, event_name: "todo_due", session_id: "s", publication_ref: "todo-flow", queue_ref: "main", scheduled: true } })
    return { AppAutomation: { installation_id: "todo", automation } }
  } })
  assert.equal(result.ok, true)
  assert.match(result.message!, /reminders · todo_due v1 → workflow p \(queue q\) · active · revision 1 · scheduled/)
  const disabled = await executeAppCommand(["automation", "disable", "todo", "reminders", "1"], { send: async sent => {
    assert.deepEqual(sent, { DisableAppAutomation: { installation_id: "todo", automation_id: "reminders", expected_revision: 1 } })
    return { AppAutomations: { installation_id: "todo", automations: [] } }
  } })
  assert.equal(disabled.message, "No App automations.")
  for (const args of [["automation", "add", "todo", "reminders"], ["automation", "disable", "todo", "reminders", "x"], ["automation", "add", "todo", "a", "e", "s", "w", "--bogus"], ["worker"]]) {
    const invalid = await executeAppCommand(args, { send: async () => { throw new Error("unexpected request") } })
    assert.equal(invalid.ok, false)
  }
})

test("App automation errors describe revision conflicts, missing targets and limits", async () => {
  const run = (code: string) => executeAppCommand(["automation", "disable", "todo", "reminders", "1"],
    { send: async () => ({ AppRequestFailed: { code } }) })
  assert.match((await run("conflict")).message!, /stale revision/)
  assert.match((await run("not_found")).message!, /session, workflow or automation/)
  assert.equal((await run("limit_exceeded")).message, "An App limit was reached.")
})

test("App open targets the attached session unless one is given", async () => {
  const opened = { AppViewOpened: { installation_id: "todo", target_id: "t1", origin: "https://a1.app.chariox.internal" } }
  const sent: Record<string, unknown>[] = []
  const client = { send: async (request: Record<string, unknown>) => { sent.push(request); return opened } }
  const attached = await executeAppCommand(["open", "todo"], client, { sessionId: "s1" })
  assert.equal(attached.ok, true)
  assert.match(attached.message!, /Room browser Tab/)
  await executeAppCommand(["open", "todo", "--session", "s2"], client, { sessionId: "s1" })
  assert.deepEqual(sent, [
    { OpenAppView: { session_id: "s1", installation_id: "todo" } },
    { OpenAppView: { session_id: "s2", installation_id: "todo" } },
  ])
  const detached = await executeAppCommand(["open", "todo"], { send: async () => { throw new Error("unexpected request") } })
  assert.equal(detached.ok, false)
  assert.match(detached.message!, /Attach to a session/)
})
