import assert from "node:assert/strict"
import test from "node:test"

import { importRoomNativeProviderAccount, roomProviderSandboxConfigLines } from "./live-room-real-provider.mjs"

test("real-provider Room slices request the nested provider sandbox boundary", () => {
  assert.deepEqual(roomProviderSandboxConfigLines({ provider: "codex" }), [
    "allow_provider_sandbox_compatibility = true",
  ])
  assert.deepEqual(roomProviderSandboxConfigLines(null), [])
})

test("fresh Room drill imports the selected native account through the kernel", async () => {
  const calls = []
  const requests = {
    importNativeProviderAccountProfileRequest: (provider) => ({ ImportNativeProviderAccountProfile: { provider } }),
  }
  const client = { send: async (request) => {
    calls.push(request)
    return { ProviderAccountProfile: { profile: {
      provider: "codex", profile_id: "native-default-fixture", label: "codex-1", is_default: true,
    } } }
  } }
  const options = { provider: "codex", accountProfile: "codex-1", model: "gpt-6-luna" }
  const resolved = await importRoomNativeProviderAccount({ client, requests, options })
  assert.deepEqual(calls, [{ ImportNativeProviderAccountProfile: { provider: "codex" } }])
  assert.equal(resolved.accountProfile, "native-default-fixture")
  assert.equal(resolved.requestedAccountProfile, "codex-1")
  assert.equal(options.accountProfile, "codex-1")
})

test("fresh Room drill rejects an account-label mismatch before provider launch", async () => {
  const requests = { importNativeProviderAccountProfileRequest: () => ({ import: true }) }
  const client = { send: async () => ({ ProviderAccountProfile: { profile: {
    provider: "codex", profile_id: "native-default-fixture", label: "codex-2", is_default: true,
  } } }) }
  await assert.rejects(importRoomNativeProviderAccount({ client, requests, options: {
    provider: "codex", accountProfile: "codex-1",
  } }), /does not match/)
})
