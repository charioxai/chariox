// MP-08 / MP-10 / MP-11: serialization regression only; no provider login or turn.
import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { runInNewContext } from "node:vm"

const source = await readFile(new URL("./live-claude-two-accounts.mjs", import.meta.url), "utf8")
// Execute the drill's actual IPC helper without its login-dependent entry point.
const start = source.indexOf("const payload =")
const end = source.indexOf("const hash =", start)
assert(start >= 0 && end > start, "drill request helper not found")
const requestWith = client => runInNewContext(`${source.slice(start, end)}\nrequest`, { assert, client })

test("session discovery sends the unit ListSessions request as null", async () => {
  const request = requestWith({
    async send(message) {
      assert.equal(JSON.stringify(message), '{"ListSessions":null}')
      return { SessionsListed: { sessions: [{ id: "owned", alias: "drill" }] } }
    },
  })
  const session = (await request("ListSessions")).sessions.find(value => value.alias === "drill")
  assert.equal(session.id, "owned")
})

test("cleanup discovers an unattached session and preserves the DeleteSession body", async () => {
  let sessions = [{ id: "owned", alias: "drill", workspace_id: "workspace" }, { id: "other", alias: "unrelated" }]
  const requests = []
  const request = requestWith({
    async send(message) {
      const wire = JSON.parse(JSON.stringify(message))
      requests.push(wire)
      if ("ListSessions" in wire) {
        assert.equal(wire.ListSessions, null)
        return { SessionsListed: { sessions } }
      }
      assert.deepEqual(wire, { DeleteSession: { session_ref: "owned", workspace_id: "workspace" } })
      sessions = sessions.filter(value => value.id !== wire.DeleteSession.session_ref)
      return { SessionDeleted: { session_id: "owned" } }
    },
  })
  const session = (await request("ListSessions")).sessions.find(value => value.alias === "drill")
  await request("DeleteSession", { session_ref: session.id, workspace_id: session.workspace_id })
  assert(!(await request("ListSessions")).sessions.some(value => value.alias === "drill"))
  assert.deepEqual(sessions.map(value => value.id), ["other"])
  assert.equal(requests.length, 3)
})
