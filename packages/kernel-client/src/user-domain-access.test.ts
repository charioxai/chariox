import assert from "node:assert/strict"
import test from "node:test"
import { UserDomainAccessController, userDomainGrantExpiry, userDomainUseNotice, type UserDomainAccessClient } from "./user-domain-access.js"
import type { UserDomainGrant, UserDomainGrantEvent } from "./kernel-types-kernel-browser.js"
const grant: UserDomainGrant = { agent_id: "a", session_id: "s", kernel_id: "k", resources: [{ kind: "note", note_id: "n" }], since_ms: 100, focused: false, idle_since_ms: null, idle_timeout_seconds: 1800, expiry_rule: "kernel policy" }
const event = (cursor: number, grants = [grant], notice: UserDomainGrantEvent["notice"] = null): UserDomainGrantEvent => ({ event: "user_domain_grants_changed", cursor, grants, notice })
const tick = () => new Promise(resolve => setImmediate(resolve))
function fixture(version = 443) {
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
test("grant consumers alone require 443 and send nothing to older kernels", () => {
  for (const version of [0, 435, 442]) {
    const h = fixture(version); h.controller.sync(); assert.match(h.controller.error!, /443/); assert.equal(h.requests.length, 0); h.controller.stop()
  }
})
test("cursor feed observes retained use and suppresses repeats, revoke cancels authority", async () => {
  const h = fixture(); h.controller.sync(); await tick()
  assert.deepEqual(h.requests[1], { KernelBrowser: { command: { op: "subscribe_grants", after: 1, wait_ms: 1000 } } })
  const notice = { agent_id: "a", resource: grant.resources[0]!, at_ms: 500 }
  h.pending[0]!.resolve({ KernelBrowser: { result: event(2, [grant], notice) } }); await tick()
  assert.equal(h.controller.lastObservedRetainedUse(grant), 500)
  assert.match(userDomainUseNotice(h.controller.snapshot!.notice)!, /not focused/)
  h.set(event(3, [])); await h.controller.revoke("a")
  assert.deepEqual(h.requests.at(-1), { KernelBrowser: { command: { op: "revoke_grants", agent_id: "a" } } })
  h.pending[1]!.resolve({ KernelBrowser: { result: event(2, [grant], notice) } }); await tick()
  assert.equal(h.controller.snapshot!.cursor, 3); assert.deepEqual(h.controller.snapshot!.grants, [])
  assert.equal(h.requests.filter(r => r.KernelBrowser.command.op === "list_grants").length, 1, "a revoke advancing the snapshot must not be mistaken for a restart")
  h.controller.stop()
})
test("revoke all is explicit null and stale notices cannot survive refocus", async () => {
  const h = fixture(); h.controller.sync(); await tick()
  const newGrant = { ...grant, since_ms: 600 }
  h.set(event(2, [newGrant], { agent_id: "a", resource: grant.resources[0]!, at_ms: 500 }))
  await h.controller.revoke(null)
  assert.deepEqual(h.requests.at(-1).KernelBrowser.command, { op: "revoke_grants", agent_id: null })
  assert.equal(h.controller.snapshot!.notice, null); assert.equal(h.controller.lastObservedRetainedUse(newGrant), null)
  h.controller.stop()
})
test("owner/connection changes abort subscriptions and discard late responses", async () => {
  const h = fixture(); h.controller.sync(); await tick(); h.move(); h.controller.sync(); await tick()
  assert.equal(h.pending[0]!.options.signal.aborted, true)
  h.pending[0]!.resolve({ KernelBrowser: { result: event(99) } }); await tick()
  assert.equal(h.controller.snapshot!.cursor, 1)
  h.disconnect(); h.controller.sync(); assert.equal(h.controller.snapshot, null); h.controller.stop()
})
test("a revoked notice is filtered and last use is never invented", async () => {
  const h = fixture(); h.set(event(1, [], { agent_id: "a", resource: grant.resources[0]!, at_ms: 500 }))
  h.controller.sync(); await tick()
  assert.equal(h.controller.snapshot!.notice, null); assert.equal(h.controller.lastObservedRetainedUse(grant), null); h.controller.stop()
})
test("expiry projection follows focus, activity and kernel idle duration", () => {
  assert.match(userDomainGrantExpiry({ ...grant, focused: true }), /pending wake/)
  assert.equal(userDomainGrantExpiry({ ...grant, focused: true, idle_since_ms: 1000 }), "1970-01-01T00:30:01.000Z")
  assert.match(userDomainGrantExpiry(grant), /pending wake/)
  assert.equal(userDomainGrantExpiry({ ...grant, expires_at_ms: 28_800_001 }), "1970-01-01T08:00:00.001Z")
  assert.equal(userDomainGrantExpiry({ ...grant, idle_since_ms: 1000, expires_at_ms: 5000 }), "1970-01-01T00:00:05.000Z")
  assert.equal(userDomainGrantExpiry({ ...grant, idle_since_ms: 1000 }), "1970-01-01T00:30:01.000Z")
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

test("successful lower-cursor replay refreshes authority and fences concurrent old mutation replies", async t => {
  const notice = { agent_id: "a", resource: grant.resources[0]!, at_ms: 500 }
  let current = event(100, [grant], notice)
  const requests: any[] = [], pending: { options: any; resolve(value: unknown): void }[] = []
  const client: UserDomainAccessClient = { localDaemonProtocolVersion: 443, async send<T>(request: any, options: any): Promise<T> {
    options?.beforeSend?.()
    const command = request.KernelBrowser.command
    requests.push(command)
    if (command.op === "list_grants") return { KernelBrowser: { result: current } } as T
    return new Promise(resolve => pending.push({ options, resolve: value => resolve(value as T) }))
  } }
  const controller = new UserDomainAccessController({ client: () => client, bindingKey: () => "same/kernel/owner" })
  t.after(() => controller.stop())
  controller.sync(); await tick()
  assert.equal(controller.lastObservedRetainedUse(grant), 500)
  const revocation = controller.revoke("a")
  const rejected = assert.rejects(revocation, /Kernel or owner changed/)
  current = event(1)
  pending[0]!.resolve({ KernelBrowser: { result: current } }); await tick()
  assert.equal(pending[0]!.options.signal.aborted, true)
  assert.equal(pending[1]!.options.signal.aborted, true)
  assert.equal(controller.snapshot!.cursor, 1)
  assert.equal(controller.snapshot!.notice, null)
  assert.equal(controller.lastObservedRetainedUse(grant), null, "old observed use cannot survive a cursor reset")
  assert.equal(controller.busy, false)
  assert.deepEqual(requests.map(r => [r.op, r.after]), [
    ["list_grants", undefined], ["subscribe_grants", 100], ["revoke_grants", undefined],
    ["list_grants", undefined], ["subscribe_grants", 1],
  ])
  // The old grant/revocation response cannot restore the previous high cursor.
  pending[1]!.resolve({ KernelBrowser: { result: event(200, [grant], notice) } }); await rejected
  assert.equal(controller.snapshot!.cursor, 1)
  assert.equal(controller.error, null)
  pending[2]!.resolve({ KernelBrowser: { result: event(2, [grant], { ...notice, at_ms: 600 }) } }); await tick()
  assert.equal(controller.lastObservedRetainedUse(grant), 600)
  assert.equal(requests.at(-1).after, 2)
})
