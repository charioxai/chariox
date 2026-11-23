import { kernelBrowserAgentTabsMinimumProtocolVersion } from "@chariox/kernel-client"
import assert from "node:assert/strict"
import test from "node:test"
import { createAccessCommandController } from "./access-command-controller.js"
import { parseSlashCommand } from "./commands.js"
test("MP-08 / MP-11 leased Computer uses explicit target and rejects older kernels", async () => {
  const old = fixture(494)
  await assert.rejects(old.controller.handle(["grant-computer", "holder", "desktop", "g"]), /500/)
  assert.equal(old.requests.length, 0)
  const h = fixture(500)
  await h.controller.handle(["grant-computer", "holder", "desktop", "g"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "grant_leased_computer", agent_id: "holder", target: { surface_id: "desktop", generation: "g" } } } })
  await h.controller.handle(["revoke-computer", "holder"])
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_leased_computer", agent_id: "holder" } } })
  for (const args of [["grant-computer", "holder"], ["grant-computer", "all", "s", "g"], ["revoke-computer", "all"]]) await assert.rejects(h.controller.handle(args), /Usage/)
})
function fixture(version = 507) {
  const requests: any[] = [], lines: string[] = []
  const controller = createAccessCommandController({ client: { localDaemonProtocolVersion: version, async send<T>(request: unknown): Promise<T> {
    requests.push(request)
    const command = (request as any).KernelBrowser.command
    if (command.op === "state") return {KernelBrowser: {result: {generation: 1, tabs: [{tab_id: "agent-tab", url: "https://developer.mozilla.org", title: "MDN", opened_by: {kind: "agent", display_label: "Mara"}}, {tab_id: "human-tab", url: "https://en.wikipedia.org", title: "Wikipedia", opened_by: {kind: "human", display_label: "Pat"}}]}}} as T
    return {KernelBrowser: {result: {agent_id: command.agent_id, leased_computer: command.op === "grant_leased_computer"}}} as T
  } }, appendNotice: line => lines.push(line) })
  return { controller, requests, lines }
}
test("access parser has a word boundary and passes its arguments", () => {
  assert.deepEqual(parseSlashCommand("/access tabs"), { kind: "access", raw: "/access tabs", args: ["tabs"] })
  assert.equal(parseSlashCommand("/accessory"), null)
})
// Local 1015: the owner's agents need no per-agent Room Computer switch, so the switch commands are gone.
test("the per-agent Room Computer switches are gone: /access list, revoke and grant issue no request", async () => {
  const h = fixture()
  for (const args of [[], ["list"], ["revoke", "holder"], ["revoke", "all"], ["grant", "holder"], ["grant", "all"], ["tabs", "extra"]]) await assert.rejects(h.controller.handle(args), /Usage/)
  assert.equal(h.requests.length, 0)
})
test("MP-08 TUI tab inventory lists every kernel tab with opener/title/URL without control", async () => {
  const h = fixture()
  await h.controller.handle(["tabs"])
  assert.match(h.lines.at(-1)!, /agent-tab.*MDN.*https:\/\/developer.mozilla.org.*Mara/)
  assert.match(h.lines.at(-1)!, /human-tab.*Wikipedia.*https:\/\/en.wikipedia.org/)
  assert(h.requests.every(r => r.KernelBrowser.command.op === "state"))
})
// MP-08/MP-11: published PR937 cannot promise opener provenance.
test("MP-08 TUI agent tabs require the merged minimum", async () => {
  const minimum = kernelBrowserAgentTabsMinimumProtocolVersion
  const h = fixture(minimum - 1)
  await assert.rejects(h.controller.handle(["tabs"]), new RegExp(`protocol ${minimum}`))
  assert.equal(h.requests.length, 0)
})
