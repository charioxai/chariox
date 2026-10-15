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
  assert.equal(notices.at(-1), "Checking Vault and access status…")
  security.connectionChanged(true)
  security.connectionChanged(true)
  await new Promise(resolve => setImmediate(resolve))
  assert.equal(reads, 1)
  assert.equal(notices.at(-1), "Vault locked · 0 active access grants")
})
