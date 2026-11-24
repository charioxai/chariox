import assert from "node:assert/strict"
import test from "node:test"
import { UserDomainAccessController, type UserDomainAccessClient } from "./user-domain-access.js"
import type { UserDomainGrantEvent } from "./kernel-types-kernel-browser.js"
const event = (cursor: number, room_computer: UserDomainGrantEvent["room_computer"] = []): UserDomainGrantEvent => ({ event: "user_domain_grants_changed", cursor, room_computer })
const tick = () => new Promise(resolve => setImmediate(resolve))
function fixture(version = 494) {
  const requests: any[] = [], pending: { resolve(value: unknown): void; options: any }[] = []
  let current = event(1), binding = "owner/kernel", connected = true
  const client: UserDomainAccessClient = { localDaemonProtocolVersion: version, async send<T>(request: any, options: any): Promise<T> {
    options?.beforeSend?.(); requests.push(request)
    if (request.KernelBrowser.command.op === "subscribe_grants") return new Promise(resolve => pending.push({ resolve: value => resolve(value as T), options }))
    return { KernelBrowser: { result: current } } as T
  } }
  const controller = new UserDomainAccessController({ client: () => connected ? client : null, bindingKey: () => binding })
  return { controller, requests, pending, set: (value: UserDomainGrantEvent) => { current = value }, move: () => { binding = "other/owner" }, disconnect: () => { connected = false } }
}
test("Room Computer consumers alone require 494 and send nothing to older kernels", () => {
  for (const version of [0, 435, 442, 472, 487, 493]) {
    const h = fixture(version); h.controller.sync(); assert.match(h.controller.error!, /494/); assert.equal(h.requests.length, 0); h.controller.stop()
  }
})
test("the cursor feed follows the kernel and a revoke advancing the snapshot is not a restart", async () => {
  const h = fixture(); h.controller.sync(); await tick()
  assert.deepEqual(h.requests[1], { KernelBrowser: { command: { op: "subscribe_grants", after: 1, wait_ms: 1000 } } })
  h.pending[0]!.resolve({ KernelBrowser: { result: event(2) } }); await tick()
  assert.equal(h.controller.snapshot!.cursor, 2)
  h.set(event(3, [{ agent_id: "a", session_id: "s", allowed: false }])); await h.controller.revoke("a")
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_grants", agent_id: "a" } } })
  h.pending[1]!.resolve({ KernelBrowser: { result: event(2) } }); await tick()
  assert.equal(h.controller.snapshot!.cursor, 3)
  assert.equal(h.requests.filter(r => r.KernelBrowser.command.op === "list_grants").length, 1)
  h.controller.stop()
})
test("revoke all is explicit null", async () => {
  const h = fixture(); h.controller.sync(); await tick()
  h.set(event(2)); await h.controller.revoke(null)
  assert.deepEqual(h.requests.at(-1).KernelBrowser.command, { op: "revoke_grants", agent_id: null })
  h.controller.stop()
})
test("owner/connection changes abort subscriptions and discard late responses", async () => {
  const h = fixture(); h.controller.sync(); await tick(); h.move(); h.controller.sync(); await tick()
  assert.equal(h.pending[0]!.options.signal.aborted, true)
  h.pending[0]!.resolve({ KernelBrowser: { result: event(99) } }); await tick()
  assert.equal(h.controller.snapshot!.cursor, 1)
  h.disconnect(); h.controller.sync(); assert.equal(h.controller.snapshot, null); h.controller.stop()
})
test("failure stops the feed, exposes error, and refresh rebinds", async () => {
  const h = fixture(); h.controller.sync(); await tick()
  h.pending[0]!.resolve({ Error: { message: "owner disconnected" } }); await tick()
  assert.equal(h.controller.error, "owner disconnected")
  assert.equal(h.requests.length, 2)
  h.controller.refresh(); await tick(); assert.equal(h.controller.error, null); h.controller.stop()
})
test("failed feed automatically rebinds and accepts a restarted kernel's lower cursor", async context => {
  context.mock.timers.enable({ apis: ["setTimeout"] })
  const h = fixture(); h.controller.sync(); await tick()
  h.pending[0]!.resolve({ KernelBrowser: { result: event(50) } }); await tick()
  h.pending[1]!.resolve({ Error: { message: "connection closed" } }); await tick()
  assert.match(h.controller.error!, /closed/)
  context.mock.timers.tick(2000); await tick()
  assert.equal(h.controller.snapshot!.cursor, 1); assert.equal(h.controller.error, null)
  h.controller.stop()
})

// MP-08 / MP-10 / MP-11: the new grant operation alone requires 494.
test("Room Computer grant is version gated and uses the existing Access transport", async () => {
  const old = fixture(448); old.controller.sync(); await tick()
  await assert.rejects(old.controller.grantRoomComputer("a"), /494/)
  assert(!old.requests.some(r => r.KernelBrowser.command.op === "grant_room_computer")); old.controller.stop()
  const h = fixture(494); h.controller.sync(); await tick()
  h.set({ ...event(2), room_computer: [{ agent_id: "a", session_id: "s", allowed: true }] })
  await h.controller.grantRoomComputer("a")
  assert.deepEqual(h.requests.at(-1).KernelBrowser.command, { op: "grant_room_computer", agent_id: "a" })
  assert.equal(h.controller.snapshot!.room_computer![0]!.allowed, true); h.controller.stop()
})

// MP-08 / MP-11: bulk restore (null agent) alone requires 494; older kernels receive nothing.
test("Room Computer bulk restore is explicit null and version gated", async () => {
  const old = fixture(460); old.controller.sync(); await tick()
  await assert.rejects(old.controller.grantRoomComputer(null), /494/)
  assert(!old.requests.some(r => r.KernelBrowser.command.op === "grant_room_computer")); old.controller.stop()
  const h = fixture(494); h.controller.sync(); await tick()
  await h.controller.grantRoomComputer(null)
  assert.deepEqual(h.requests.at(-1).KernelBrowser.command, { op: "grant_room_computer", agent_id: null }); h.controller.stop()
})
