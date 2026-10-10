import { LocalIpcError } from "./local-ipc-error.js"
import assert from "node:assert/strict"
import { test } from "node:test"
import { appCommandArgs, executeAppCommand } from "./shell-app-command.js"

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
  for (const args of [["list", "--limit", "101"], ["list", "--limit", "1e2"], ["list", "--after"], ["list", "--limit", "1", "--limit", "2"], ["status"], ["status", "todo", "extra"], ["install", "/host/file"], ["update", "todo", "/host/file"]]) {
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

test("App status shows the full active package digest and handles uninstalled Apps", async () => {
  const digest = `sha256:${"a".repeat(64)}`
  const installation = { installation_id: "todo", app_id: "com.chariox.todo", generation: "7", active_release: { version: "1.0.0", package_digest: digest }, pending_generation: null, admission_paused: false, data_kept: false }
  const status = await executeAppCommand(["status", "todo"], { send: async () => ({ AppInstallation: { installation } }) })
  assert.equal(status.message, `todo · com.chariox.todo · version 1.0.0; digest ${digest}; generation 7`)
  const removed = await executeAppCommand(["status", "todo"], { send: async () => ({ AppInstallation: { installation: { ...installation, active_release: null } } }) })
  assert.equal(removed.message, "todo · com.chariox.todo · no active release; generation 7")
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

test("A worker control or uninstall whose answer was lost is reported as unknown, not resent", async () => {
  for (const [args, check] of [
    [["restart", "todo"], "app worker todo"],
    [["stop", "todo"], "app worker todo"],
    [["uninstall", "todo", "--generation", "3"], "app status todo"],
  ] as const) {
    let sends = 0
    const result = await executeAppCommand([...args], { send: async () => {
      sends += 1
      throw new LocalIpcError("handle kernel response", "the connection closed before the answer", "outcome_unknown")
    } })
    assert.equal(result.ok, false)
    assert.equal(sends, 1)
    assert.match(result.message!, new RegExp(`may still happen\\. Check with ${check} before trying again`))
  }
  // A request that never reached the kernel keeps its transport error: retrying it is safe.
  await assert.rejects(executeAppCommand(["restart", "todo"], { send: async () => {
    throw new LocalIpcError("connect kernel websocket", "connect ECONNREFUSED 127.0.0.1:1", "connection_closed", true)
  } }), /ECONNREFUSED/)
  await assert.rejects(executeAppCommand(["restart", "todo"], { send: async () => {
    throw new LocalIpcError("kernel websocket", "closed", "client_closed", false)
  } }), /closed/)
})

test("a worker refused for low host disk space points at the App log that says how much to free", async () => {
  const worker = { installation_id: "docs", phase: "failed", enabled: true, failure: "app_lifecycle_disk_space", updated_at_ms: 1 }
  const result = await executeAppCommand(["worker", "docs"], { send: async () => ({ AppWorker: { worker } }) })
  assert.equal(result.message, "docs · failed · app_lifecycle_disk_space: not enough free disk space on the host; "
    + "app logs docs says how much to free")
  const other = await executeAppCommand(["worker", "docs"], {
    send: async () => ({ AppWorker: { worker: { ...worker, failure: "app_lifecycle_preparation" } } }),
  })
  assert.equal(other.message, "docs · failed · app_lifecycle_preparation")
})

test("App automation commands route one event to one workflow and validate arguments", async () => {
  const automation = { automation_id: "reminders", revision: 1, event_name: "todo_due", event_version: 1, session_id: "s", publication_id: "p", endpoint_id: "e", queue_id: "q", scheduled: true, status: "active" }
  const result = await executeAppCommand(["automation", "add", "todo", "reminders", "todo_due", "s", "todo-flow", "--scheduled", "--queue", "main"], { send: async sent => {
    assert.deepEqual(sent, { ConfigureAppAutomation: { installation_id: "todo", automation_id: "reminders", expected_revision: 0, event_name: "todo_due", session_id: "s", publication_ref: "todo-flow", queue_ref: "main", scheduled: true, delivery_mode: "queue" } })
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
  const opened = { AppViewOpened: { installation_id: "todo", target_id: "t1", origin: "https://a1.app.chariox.internal", bound_agent_id: "agent-1" } }
  const sent: Record<string, unknown>[] = []
  const client = { send: async (request: Record<string, unknown>) => { sent.push(request); return opened } }
  const attached = await executeAppCommand(["open", "todo"], client, { sessionId: "s1" })
  assert.equal(attached.ok, true)
  assert.match(attached.message!, /Room browser Tab/)
  assert.match(attached.message!, /available to the focus agent \(agent-1\)/)
  await executeAppCommand(["open", "todo", "--session", "s2"], client, { sessionId: "s1" })
  assert.deepEqual(sent, [
    { OpenAppView: { session_id: "s1", installation_id: "todo" } },
    { OpenAppView: { session_id: "s2", installation_id: "todo" } },
  ])
  const detached = await executeAppCommand(["open", "todo"], { send: async () => { throw new Error("unexpected request") } })
  assert.equal(detached.ok, false)
  assert.match(detached.message!, /Attach to a session/)
})

test("App uninstall is fenced by the generation it just read", async () => {
  const sent: Record<string, unknown>[] = []
  const installation = { installation_id: "todo", app_id: "com.chariox.todo", generation: "7", active_release: null, pending_generation: null, admission_paused: false, data_kept: true }
  const result = await executeAppCommand(["uninstall", "todo"], { send: async (request) => {
    sent.push(request)
    return { AppInstallation: { installation } }
  } })
  assert.deepEqual(sent, [
    { GetAppInstallation: { installation_id: "todo" } },
    { UninstallApp: { installation_id: "todo", expected_generation: "7" } },
  ])
  assert.equal(result.ok, true)
  assert.match(result.message!, /Uninstalled todo\. Its data is kept: app update todo <package> reinstalls into it\./)
  const missing = await executeAppCommand(["uninstall", "gone"], { send: async () => ({ AppRequestFailed: { code: "not_found" } }) })
  assert.equal(missing.message, "App installation not found.")
  const pinned: Record<string, unknown>[] = []
  await executeAppCommand(["uninstall", "todo", "--generation", "5"], { send: async (request) => {
    pinned.push(request)
    return { AppInstallation: { installation } }
  } })
  assert.deepEqual(pinned, [{ UninstallApp: { installation_id: "todo", expected_generation: "5" } }])
  const deleted: Record<string, unknown>[] = []
  const removed = await executeAppCommand(["uninstall", "todo", "--delete-data", "--generation", "5"], { send: async (request) => {
    deleted.push(request)
    return { AppInstallation: { installation: { ...installation, data_kept: false } } }
  } })
  assert.deepEqual(deleted, [{ UninstallApp: { installation_id: "todo", expected_generation: "5", delete_data: true } }])
  assert.match(removed.message!, /Uninstalled todo\. Its data is deleted\./)
  const repeated = await executeAppCommand(["uninstall", "todo", "--delete-data", "--delete-data"], { send: async () => ({}) })
  assert.equal(repeated.ok, false)
})

test("App logs page by sequence and escape App-authored control characters", async () => {
  const sent: Record<string, unknown>[] = []
  const result = await executeAppCommand(["logs", "todo", "--after", "7"], { send: async (request) => {
    sent.push(request)
    return { AppLogs: { installation_id: "todo", entries: [
      { sequence: "8", at_ms: 0, level: "warn", message: "bad\u001b[31mred\u009b2J\u202eevil", fields: { n: 1 } },
    ] } }
  } })
  assert.deepEqual(sent, [{ GetAppLogs: { installation_id: "todo", after_sequence: "7" } }])
  assert.match(result.message!, /1970-01-01T00:00:00.000Z WARN  bad\\u001b\[31mred\\u009b2J\\u202eevil \{"n":1\}/)
  assert.match(result.message!, /More: app logs todo --after 8/)
  assert.equal((await executeAppCommand(["logs", "todo", "--after", "x"], { send: async () => ({}) })).ok, false)
})

test("app inbox configures routes and test occurrences through the shared requests", async () => {
  const sent: Record<string, unknown>[] = []
  const route = { route_id: "mail", event_name: "todo_requested", source_event_type: "dev.chariox.dummy/dummy.test",
    source_event_version: 2, active: true, pending: 1, delivered: 3, failed: 0, expired: 0 }
  const client = { send: async (request: Record<string, unknown>) => {
    sent.push(request)
    if (request.TestAppInboxRoute) return { AppInboxOccurrenceAccepted: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", duplicate: true } }
    return { AppInboxRoutes: { installation_id: "todo", routes: [route] } }
  } }
  const added = await executeAppCommand(["inbox", "add", "todo", "mail", "todo_requested", "dev.chariox.dummy/dummy.test", "--version", "2"], client)
  assert.match(added.message!, /mail · dev.chariox.dummy\/dummy.test v2 → todo_requested · 1 pending, 3 delivered, 0 failed, 0 expired/)
  const tested = await executeAppCommand(["inbox", "test", "todo", "mail", "occ-1", '{"title":"x', 'y"}'], client)
  assert.equal(tested.message, "Occurrence occ-1 on mail was already accepted.")
  await executeAppCommand(["inbox", "remove", "todo", "mail"], client)
  await executeAppCommand(["inbox", "list", "todo"], client)
  assert.deepEqual(sent, [
    { CreateAppInboxRoute: { installation_id: "todo", route_id: "mail", event_name: "todo_requested", source_event_type: "dev.chariox.dummy/dummy.test", source_event_version: 2 } },
    { TestAppInboxRoute: { installation_id: "todo", route_id: "mail", occurrence_id: "occ-1", payload: { title: "x y" } } },
    { RemoveAppInboxRoute: { installation_id: "todo", route_id: "mail" } },
    { ListAppInboxRoutes: { installation_id: "todo" } },
  ])
  for (const args of [["inbox", "test", "todo", "mail", "occ", "{bad"], ["inbox", "add", "todo", "mail", "e", "t", "--version", "0"], ["inbox", "list"]]) {
    const result = await executeAppCommand(args, { send: async () => { throw new Error("unexpected request") } })
    assert.equal(result.ok, false)
    assert.match(result.message!, /'<json-payload>'/)
  }
  const refused = await executeAppCommand(["inbox", "list", "todo"], { send: async () => ({ AppRequestFailed: { code: "invalid_request" } }) })
  assert.match(refused.message!, /declares as incoming/)
})

test("an inbox test payload is taken from the raw line in every shell", () => {
  const raw = '/app inbox test todo mail occ-1 {"title":"a  b", "note":"x"}'
  assert.deepEqual(appCommandArgs(raw, ["inbox", "test", "todo", "mail", "occ-1", "{title:a", "b,"]), [
    "inbox", "test", "todo", "mail", "occ-1", '{"title":"a  b", "note":"x"}',
  ])
  assert.deepEqual(appCommandArgs("/app list", ["list"]), ["list"])
})


test("App open argument errors show only open usage without sending", async () => {
  for (const args of [["open"], ["open", "todo", "--session"], ["open", "--session", "s"], ["open", "todo", "--session", "--bad"], ["open", "todo", "extra"]]) {
    const result = await executeAppCommand(args, { send: async () => { throw new Error("unexpected request") } })
    assert.equal(result.ok, false)
    assert.equal(result.message, "usage: app open <installation-id> [--session <session-id>]")
  }
})


test("App quarantine shows explicit start recovery without relabelling ordinary failures", async () => {
  for (const [phase, enabled, label] of [
    ["quarantined", true, 'quarantined · explicit start required: app start "todo"'],
    ["failed", true, "failed"],
    ["stopped", false, "stopped (stopped by user)"],
  ] as const) {
    const result = await executeAppCommand(["worker", "todo"], { send: async request => {
      assert.deepEqual(request, { GetAppWorker: { installation_id: "todo" } })
      return { AppWorker: { worker: { installation_id: "todo", phase, enabled,
        failure: "app_worker_exited", updated_at_ms: 1 } } }
    } })
    assert.equal(result.ok, true)
    assert.equal(result.message, `todo · ${label} · app_worker_exited`)
  }
  const recovered = await executeAppCommand(["start", "todo"], { send: async request => {
    assert.deepEqual(request, { ControlAppWorker: { installation_id: "todo", action: "start" } })
    return { AppWorker: { worker: { installation_id: "todo", phase: "running", enabled: true,
      failure: null, updated_at_ms: 2 } } }
  } })
  assert.equal(recovered.message, "todo · running")
})

test("App worker start causes distinguish readiness cancellation from deadline", async () => {
  for (const suffix of ["cancelled", "deadline"]) {
    const failure = `app_lifecycle_registration_${suffix}`
    for (const action of ["worker", "start"]) {
      const result = await executeAppCommand([action, "docs"], { send: async () => ({
        AppWorker: { worker: { installation_id: "docs", phase: "failed", enabled: true, failure, updated_at_ms: 1 } },
      }) })
      assert.equal(result.message, `docs · failed · ${failure}`)
      if (suffix === "cancelled") assert.doesNotMatch(result.message!, /deadline|timed out|timeout/)
    }
  }
})

 test("MP-08 / MP-10 App automation attach supports inject", async () => {
  let request: Record<string, unknown> | undefined
  const result = await executeAppCommand(["automation", "add", "app", "sub", "changed", "s", "p", "--delivery", "inject"], { send: async sent => {
    request = sent
    return { AppAutomation: { installation_id: "app", automation: { automation_id: "sub", revision: 1, event_name: "changed", event_version: 1, session_id: "s", publication_id: "p", endpoint_id: "e", queue_id: "q", scheduled: false, status: "active", delivery_mode: "inject" } } }
  } })
  assert.equal(result.ok, true)
  assert.equal((request?.ConfigureAppAutomation as { delivery_mode: string }).delivery_mode, "inject")
 })
