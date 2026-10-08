import assert from "node:assert/strict"
import test from "node:test"
import { createAccessCommandController } from "./access-command-controller.js"
import { parseSlashCommand } from "./commands.js"
const snapshot = { event: "user_domain_grants_changed", cursor: 1, grants: [{ agent_id: "holder", session_id: "session", kernel_id: "kernel", resources: [{ kind: "note", note_id: "note" }], since_ms: 100, focused: false, idle_since_ms: null, idle_timeout_seconds: 1800 }], notice: null }
function fixture() {
  const requests: any[] = [], lines: string[] = []
  const controller = createAccessCommandController({ client: { localDaemonProtocolVersion: 443, async send<T>(request: unknown): Promise<T> { requests.push(request); if ((request as any).KernelBrowser.command.op === "subscribe_grants") return new Promise(() => {}); return { KernelBrowser: { result: snapshot } } as T } }, appendNotice: message => lines.push(message) })
  return { controller, requests, lines }
}
test("access parser has a word boundary and passes revoke arguments", () => {
  assert.deepEqual(parseSlashCommand("/access revoke holder"), { kind: "access", raw: "/access revoke holder", args: ["revoke", "holder"] })
  assert.equal(parseSlashCommand("/accessory"), null)
})
test("lists scoped grants and uses owner revoke shapes", async () => {
  const h = fixture()
  // Skip the live feed in this focused command test.
  await h.controller.handle([])
  assert.match(h.lines.at(-1)!, /holder · session session · kernel kernel/)
  assert.match(h.lines.at(-1)!, /resources: note note/)
  await h.controller.handle(["revoke", "holder"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_grants", agent_id: "holder" } } })
  await h.controller.handle(["revoke", "all"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_grants", agent_id: null } } })
  h.controller.stop()
})
test("invalid syntax issues no mutations", async () => {
  const h = fixture()
  for (const args of [["revoke"], ["revoke", "all", "extra"], ["other"]]) await assert.rejects(h.controller.handle(args), /Usage/)
  assert.equal(h.requests.length, 0); h.controller.stop()
})
test("retained-use feed prints the shared notice once and stops after cleanup", async () => {
  const lines: string[] = [], pending: ((response: unknown) => void)[] = []
  const controller = createAccessCommandController({ client: { localDaemonProtocolVersion: 443, async send<T>(request: any): Promise<T> {
    if (request.KernelBrowser.command.op === "subscribe_grants") return new Promise(resolve => pending.push(value => resolve(value as T)))
    return { KernelBrowser: { result: snapshot } } as T
  } }, appendNotice: line => lines.push(line) })
  const tick = () => new Promise(resolve => setImmediate(resolve))
  controller.start(); await tick()
  const notice = { agent_id: "holder", resource: { kind: "note", note_id: "note" }, at_ms: 500 }
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 2, notice } } }); await tick()
  assert.deepEqual(lines, ["Agent holder used retained access to note note while not focused."])
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 3, notice } } }); await tick()
  assert.equal(lines.length, 1)
  controller.stop()
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 4, notice: { ...notice, at_ms: 600 } } } }); await tick()
  assert.equal(lines.length, 1)
})
