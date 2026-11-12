import assert from "node:assert/strict"
import test from "node:test"
import { cloudShellClient } from "./cloud-shell-client.js"
import { CloudClient } from "./cloud-client.js"
import type { LocalIpcClient } from "./ipc.js"

test("CLI shared-shell invites carry the connected kernel's id to Cloud", async () => {
  const calls: unknown[] = []
  const cloud = { collaboration: { createSessionInvite: async (session: string, options: unknown) => { calls.push([session, options]); return { invite: {} } } } } as unknown as CloudClient
  const kernel = { send: async () => ({ RelayStatus: { status: { daemon_id: "recorded-kernel" } } }) } as unknown as LocalIpcClient
  const account = cloudShellClient(cloud, kernel)!
  await account.send({ CreateCloudSessionInvite: { session_id: "room", display_name: "Plan", collaboration_level: "full" } })
  assert.deepEqual(calls, [["room", { kernelId: "recorded-kernel", displayName: "Plan", expiresInMs: undefined, maxUses: undefined, collaborationLevel: "full" }]])
})

test("CLI refuses to publish an invite when the connected kernel has no id", async () => {
  let published = false
  const cloud = { collaboration: { createSessionInvite: async () => { published = true } } } as unknown as CloudClient
  const kernel = { send: async () => ({ RelayStatus: { status: {} } }) } as unknown as LocalIpcClient
  await assert.rejects(cloudShellClient(cloud, kernel)!.send({ CreateCloudSessionInvite: { session_id: "room" } }), /did not return its id/)
  assert.equal(published, false)
})

test("CLI publishes the connected kernel id through the human-authenticated Cloud invite request", async t => {
  const credential = { profile: { apiUrl: "http://127.0.0.1:44123", accountId: "owner", userId: "owner-user", clientId: "terminal" }, clientId: "terminal", accessToken: "synthetic-client-session", publicKeyThumbprint: "a".repeat(64) }
  const store = { load: async () => credential, session: async () => credential, rememberSessionScope: async () => {} }
  const cloud = new CloudClient(store as unknown as ConstructorParameters<typeof CloudClient>[0], () => ({ publicKeyThumbprint: credential.publicKeyThumbprint }) as any)
  t.after(() => cloud.stop())
  const kernel = { send: async () => ({ RelayStatus: { status: { daemon_id: "actual-kernel" } } }) } as unknown as LocalIpcClient
  let published = false
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(new URL(String(input)).pathname, "/sessions/invites")
    const body = JSON.parse(String(init?.body))
    assert.equal(body.kernelId, "actual-kernel")
    assert.equal(body.sessionId, "room")
    assert.equal(body.sessionToken, credential.accessToken)
    published = true
    return Response.json({ inviteId: "invite", inviteToken: "synthetic-invite", sessionId: "room", accountId: "owner" })
  })
  await cloudShellClient(cloud, kernel)!.send({ CreateCloudSessionInvite: { session_id: "room", collaboration_level: "full" } })
  assert.equal(published, true)
})
