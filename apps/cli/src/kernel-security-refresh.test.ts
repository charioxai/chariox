import assert from "node:assert/strict"
import test from "node:test"
import { createKernelSecurityRefresh } from "./kernel-security-refresh.js"

test("MP-08/MP-11 security refresh checks Vault and grants and ignores a retired connection", async () => {
  const notices: string[] = []
  let finish!: (value: Record<string, unknown>) => void
  let reads = 0
  const controller = createKernelSecurityRefresh({readVault: () => new Promise(resolve => { finish = resolve }),
    readGrants: async () => { reads++; return {grants: []} }, notify: text => notices.push(text)})
  const old = controller.refresh()
  assert.equal(notices.at(-1), "Checking Vault and access status…")
  controller.invalidate()
  finish({unlocked: true})
  await old
  assert.equal(notices.length, 1)
  const fresh = controller.refresh()
  finish({unlocked: false})
  await fresh
  assert.equal(notices.at(-1), "Vault locked · 0 active access grants")
  assert.equal(reads, 2)
})

// MP-08 / MP-11: local clients reconnect through connectivity state as well as relay events.
test("MP-08/MP-11 a local reconnect immediately checks Vault and grants once", async () => {
  const notices: string[] = []
  let reads = 0
  const security = createKernelSecurityRefresh({readVault: async () => {reads++;return {unlocked: false}},
    readGrants: async () => ({grants: []}), notify: text => notices.push(text)})
  security.connectionChanged(false)
  assert.deepEqual(notices, [], "nothing is being checked while disconnected")
  security.connectionChanged(true)
  security.connectionChanged(true)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(reads, 1)
  assert.equal(notices.at(-1), "Vault locked · 0 active access grants")
})

// MP-08 / MP-11: a relay reconnect starts the check from two places; the footer shows it once.
for (const order of ["event first", "connectivity first"] as const) {
  test(`MP-08/MP-11 one reconnect flashes the security check once (${order})`, async () => {
    const notices: string[] = []
    const security = createKernelSecurityRefresh({readVault: async () => ({unlocked: false}),
      readGrants: async () => ({grants: []}), notify: text => notices.push(text)})
    security.connectionChanged(false)
    const resumed = () => { security.invalidate(); void security.refresh() }
    if (order === "event first") { resumed(); security.connectionChanged(true) }
    else { security.connectionChanged(true); resumed() }
    await new Promise(resolve => setImmediate(resolve))
    assert.equal(notices.filter(text => text.startsWith("Checking")).length, 1)
    assert.equal(notices.at(-1), "Vault locked · 0 active access grants")
  })
}

test("MP-08/MP-11 connectionChanged reports only real transitions", () => {
  const security = createKernelSecurityRefresh({readVault: async () => ({unlocked: false}),
    readGrants: async () => ({grants: []}), notify() {}})
  assert.equal(security.connectionChanged(true), false)
  assert.equal(security.connectionChanged(false), true)
  assert.equal(security.connectionChanged(false), false, "a re-run while disconnected is not a transition")
  assert.equal(security.connectionChanged(true), true)
})
