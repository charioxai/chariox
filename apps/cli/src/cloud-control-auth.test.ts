import assert from "node:assert/strict"
import test from "node:test"
import { cloudControlHeaders } from "./cloud-control-auth.js"
import type { RelayCloudProfile } from "./preferences.js"

test("public kernel enrollment cannot authorize human Cloud actions", () => {
  const profile = { kernelId: "kernel-fixture", kernelEnrolled: true } as RelayCloudProfile
  assert.throws(() => cloudControlHeaders(profile), /require a client\/browser login/)
  assert.throws(() => cloudControlHeaders({ ...profile, cloudSessionToken: "  " }), /require a client\/browser login/)
})
