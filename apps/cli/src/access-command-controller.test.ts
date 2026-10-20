import { userDomainAccessMinimumProtocol } from "@chariox/kernel-client/user-domain-access"
import { kernelBrowserAgentTabsMinimumProtocolVersion } from "@chariox/kernel-client"
import assert from "node:assert/strict"
import test from "node:test"
import { createAccessCommandController } from "./access-command-controller.js"
import { parseSlashCommand } from "./commands.js"
const snapshot = { event: "user_domain_grants_changed", cursor: 1, grants: [{ agent_id: "holder", session_id: "session", kernel_id: "kernel", resources: [{ kind: "browser_tab", tab_id: "tab" }], since_ms: 100, focused: false, idle_since_ms: null, idle_timeout_seconds: 1800 }], notice: null }
test("MP-08 / MP-11 leased Computer uses explicit target and rejects older kernels", async () => {
  const old = fixture(494)
  await assert.rejects(old.controller.handle(["grant-computer", "holder", "desktop", "g"]), /500/)
  assert.equal(old.requests.length, 0); old.controller.stop()
  const h = fixture(500)
  await h.controller.handle(["grant-computer", "holder", "desktop", "g"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "grant_leased_computer", agent_id: "holder", target: { surface_id: "desktop", generation: "g" } } } })
  await h.controller.handle(["revoke-computer", "holder"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_leased_computer", agent_id: "holder" } } })
  for (const args of [["grant-computer", "holder"], ["grant-computer", "all", "s", "g"], ["revoke-computer", "all"]]) await assert.rejects(h.controller.handle(args), /Usage/)
  h.controller.stop()
})
function fixture(version = 494) {
  const requests: any[] = [], lines: string[] = []
  const controller = createAccessCommandController({ client: { localDaemonProtocolVersion: version, async send<T>(request: unknown): Promise<T> { requests.push(request); if ((request as any).KernelBrowser.command.op === "subscribe_grants") return new Promise(() => {}); return { KernelBrowser: { result: snapshot } } as T } }, appendNotice: message => lines.push(message) })
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
  assert.match(h.lines.at(-1)!, /resources: tab tab/)
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
  const controller = createAccessCommandController({ client: { localDaemonProtocolVersion: 494, async send<T>(request: any): Promise<T> {
    if (request.KernelBrowser.command.op === "subscribe_grants") return new Promise(resolve => pending.push(value => resolve(value as T)))
    return { KernelBrowser: { result: snapshot } } as T
  } }, appendNotice: line => lines.push(line) })
  const tick = () => new Promise(resolve => setImmediate(resolve))
  controller.start(); await tick()
  const notice = { agent_id: "holder", resource: { kind: "browser_tab", tab_id: "tab" }, at_ms: 500 }
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 2, notice } } }); await tick()
  assert.deepEqual(lines, ["Agent holder used retained access to tab tab while not focused."])
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 3, notice } } }); await tick()
  assert.equal(lines.length, 1)
  controller.stop()
  pending.shift()!({ KernelBrowser: { result: { ...snapshot, cursor: 4, notice: { ...notice, at_ms: 600 } } } }); await tick()
  assert.equal(lines.length, 1)
})
test("MP-08 / MP-10 / MP-11 owner re-grants Room Computer through Access", async()=>{
  const h=fixture(494)
  await h.controller.handle(["grant","holder"])
  assert.deepEqual(h.requests.find(r=>r.KernelBrowser.command.op==="grant_room_computer"),{KernelBrowser:{command:{op:"grant_room_computer",agent_id:"holder"}}})
  assert.match(h.lines.at(-1)!,/Granted Room Computer control/)
  h.controller.stop()
})
test("MP-08 / MP-11 bulk revoke and bulk restore print visible notices", async()=>{
  const old=fixture(460)
  await assert.rejects(old.controller.handle(["grant","all"]),/494/)
  assert(!old.requests.some(r=>r.KernelBrowser.command.op==="grant_room_computer")); old.controller.stop()
  const h=fixture(494)
  await h.controller.handle(["revoke","all"])
  assert.match(h.lines.at(-1)!,/^Revoked access and Room Computer control for all agents\./)
  await h.controller.handle(["grant","all"])
  assert.deepEqual(h.requests.at(-1),{KernelBrowser:{command:{op:"grant_room_computer",agent_id:null}}})
  assert.match(h.lines.at(-1)!,/^Restored Room Computer control for all agents\./)
  h.controller.stop()
})

test("MP-08 / MP-10 / MP-11 predecessor kernels receive no access command", async () => {
  for (const version of [472, 487, 493]) {
    const h = fixture(version)
    h.controller.start()
    for (const args of [[], ["list"], ["revoke", "all"], ["grant", "holder"]]) await assert.rejects(h.controller.handle(args), /494/)
    assert.equal(h.requests.length, 0)
    h.controller.stop()
  }
})

test("MP-08 TUI tab inventory lists every kernel tab with opener/title/URL without control", async () => {
  const requests: any[] = [], lines: string[] = []
  const controller = createAccessCommandController({client: {localDaemonProtocolVersion: 507, async send<T>(r: any) {
    requests.push(r)
    if (r.KernelBrowser.command.op === "subscribe_grants") return new Promise(() => {})
    if (r.KernelBrowser.command.op === "list_grants") return {KernelBrowser: {result: snapshot}} as T
    return {KernelBrowser: {result: {generation: 1, tabs: [{tab_id: "agent-tab", url: "https://developer.mozilla.org", title: "MDN", opened_by: {kind: "agent", display_label: "Mara"}}, {tab_id: "human-tab", url: "https://en.wikipedia.org", title: "Wikipedia", opened_by: null}]}}} as T
  }}, appendNotice: line => lines.push(line)})
  try {
    await controller.handle(["tabs"])
    assert.match(lines.at(-1)!, /agent-tab.*MDN.*https:\/\/developer.mozilla.org.*Mara/)
    assert.match(lines.at(-1)!, /human-tab.*Wikipedia.*https:\/\/en.wikipedia.org/)
    assert(requests.every(r => ["state", "list_grants", "subscribe_grants"].includes(r.KernelBrowser.command.op)))
  } finally {controller.stop()}
})

// MP-08/MP-11: published PR937 cannot promise opener provenance.
test("MP-08 TUI agent tabs require the merged minimum", async () => {
  const minimum = Math.max(userDomainAccessMinimumProtocol, kernelBrowserAgentTabsMinimumProtocolVersion)
  const h = fixture(minimum - 1)
  await assert.rejects(h.controller.handle(["tabs"]), new RegExp(`protocol ${minimum}`))
  assert(h.requests.every(r => r.KernelBrowser.command.op !== "state"))
  h.controller.stop()
})
