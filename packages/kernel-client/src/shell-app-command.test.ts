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
