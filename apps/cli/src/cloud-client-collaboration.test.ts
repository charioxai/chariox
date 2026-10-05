import assert from "node:assert/strict"
import test from "node:test"
import { CloudClient } from "./cloud-client.js"
import type { CloudClientCredentialStore } from "./cloud-client-credential-store.js"

// MP-08 / MP-11: collaboration uses the same resumable private CLIENT authority
// as directory/token issuance and never writes it into kernel IPC or URLs.
for (const kind of ["members", "collaborators"] as const) test(`Cloud ${kind} recovers concurrent human-token rotation and encodes its query`, async t => {
  const credential = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "account/&?", userId: "human", clientId: "terminal"}, accessToken: "synthetic-old-access", publicKeyThumbprint: "a".repeat(64)}
  let access = credential.accessToken, requests = 0, rotations = 0
  const store = {load: async () => credential, session: async (_key: string, _force: boolean, rejected?: string) => {
    if (rejected) {assert.equal(rejected, credential.accessToken); rotations++; access = "synthetic-fresh-access"}
    return {...credential, accessToken: access}
  }}
  const client = new CloudClient(store as unknown as CloudClientCredentialStore, () => ({publicKeyThumbprint: credential.publicKeyThumbprint}) as any)
  t.after(() => client.stop())
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input)), headers = new Headers(init?.headers); requests++
    assert.equal(url.searchParams.get("sessionToken"), null)
    assert.equal(url.searchParams.get("accountId"), credential.profile.accountId)
    assert.equal(url.pathname, kind === "members" ? "/sessions/members" : "/collaborators/recent")
    if (kind === "members") assert.equal(url.searchParams.get("sessionId"), "session/&?")
    if (requests === 1) return new Response(JSON.stringify({error: {code: "session_invalid"}}), {status: 401})
    assert.equal(headers.get("authorization"), "Bearer synthetic-fresh-access")
    return new Response(JSON.stringify(kind === "members" ? {sessionId: "session/&?", members: []} : {collaborators: []}), {status: 200})
  })
  const result = kind === "members" ? await client.collaboration.sessionMembers("session/&?") : await client.collaboration.collaborators()
  assert.deepEqual(result, kind === "members" ? {session_id: "session/&?", members: []} : [])
  assert.equal(requests, 2); assert.equal(rotations, 1)
})
