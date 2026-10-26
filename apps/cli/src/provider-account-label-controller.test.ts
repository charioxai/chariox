// MP-08/MP-10/MP-11: focused projection checks; live-client drills remain required.
import assert from "node:assert/strict"
import test from "node:test"
import type { ProviderAccountProfile } from "./cli-types.js"
import { createProviderAccountLabelController } from "./provider-account-label-controller.js"

const profile = { provider: "codex", profile_id: "replica", label: "Replica account", auth_state: "authenticated", usage: { meters: [] } } as unknown as ProviderAccountProfile
const settle = () => new Promise(resolve => setTimeout(resolve, 0))

test("MP-08/MP-10/MP-11 replica status is read by exact identity, not missing home inventory", async () => {
  let requests = 0, repaints = 0
  const inventory: ProviderAccountProfile[] = []
  const actual = createProviderAccountLabelController({ profiles: () => inventory, kernelId: () => "worker", changed: () => { repaints++ }, readProfile: async (provider, reference) => {
    assert.equal(provider, "codex"); assert.equal(reference, "replica"); requests++; return profile
  } })
  assert.equal(actual.label("codex", "replica"), "Account status not checked")
  await settle()
  assert.equal(actual.label("codex", "replica"), "Replica account")
  assert.equal(requests, 1)
  assert.equal(repaints, 1, "a settled metadata read repaints a static Done footer")
  actual.dispose()
})

test("MP-08/MP-10/MP-11 failed reads never invent unavailability or retain stale sign-in", async () => {
  const inventory: ProviderAccountProfile[] = []
  let time = 0, fail = false
  const controller = createProviderAccountLabelController({ profiles: () => inventory, kernelId: () => "worker", now: () => time, readProfile: async () => {
    if (fail) throw new Error("socket closed"); return profile
  } })
  controller.label("codex", "replica"); await settle()
  assert.equal(controller.label("codex", "replica"), "Replica account")
  time = 30_001; fail = true
  assert.equal(controller.label("codex", "replica"), "Account status not checked")
  await settle()
  assert.equal(controller.label("codex", "replica"), "Account status not checked · refresh needed")
  controller.dispose()
})

test("MP-08/MP-10/MP-11 kernel switching fences old observations and sign-out has a reason", async () => {
  const inventory: ProviderAccountProfile[] = []
  let kernel = "old"
  let resolve!: (p: ProviderAccountProfile) => void
  const controller = createProviderAccountLabelController({ profiles: () => inventory, kernelId: () => kernel, readProfile: () => kernel === "old" ? new Promise<ProviderAccountProfile>(r => { resolve = r }) : Promise.resolve({ ...profile, auth_state: "not_configured" }) })
  controller.label("codex", "replica")
  kernel = "new"; controller.label("codex", "replica")
  resolve(profile); await settle()
  assert.equal(controller.label("codex", "replica"), "Replica account · sign in")
  controller.dispose()
})

test("MP-08/MP-10/MP-11 retained home aliases display the canonical public account", async () => {
  const inventory: ProviderAccountProfile[] = []
  const controller = createProviderAccountLabelController({ profiles: () => inventory, kernelId: () => "home", readProfile: async () => profile })
  controller.label("codex", "Replica account"); await settle()
  assert.equal(controller.label("codex", "Replica account"), "Replica account")
  controller.dispose()
})

test("MP-08/MP-10/MP-11 legacy namespace refusal stays unchecked instead of denying an available replica", async () => {
  const inventory: ProviderAccountProfile[] = []
  const controller = createProviderAccountLabelController({ profiles: () => inventory, kernelId: () => "home", readProfile: async () => { throw new Error("account profile `missing` is not registered for codex") } })
  controller.label("codex", "missing"); await settle()
  assert.equal(controller.label("codex", "missing"), "Account status not checked · profile lookup failed")
  controller.dispose()
})
