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
    const url = new URL(String(input)), headers = new Headers(init?.headers)
    if (url.pathname === "/sessions/invites/synthetic-invite/accept") {
      assert.equal(JSON.parse(String(init?.body)).sessionToken, credential.accessToken)
      return Response.json({sessionId: "session/&?", accountId: "shared-owner/&?", userId: credential.profile.userId})
    }
    requests++
    assert.equal(url.searchParams.get("sessionToken"), null)
    assert.equal(url.searchParams.get("accountId"), kind === "members" ? "shared-owner/&?" : credential.profile.accountId)
    assert.equal(url.pathname, kind === "members" ? "/sessions/members" : "/collaborators/recent")
    if (kind === "members") assert.equal(url.searchParams.get("sessionId"), "session/&?")
    if (requests === 1) return new Response(JSON.stringify({error: {code: "session_invalid"}}), {status: 401})
    assert.equal(headers.get("authorization"), "Bearer synthetic-fresh-access")
    return new Response(JSON.stringify(kind === "members" ? {sessionId: "session/&?", members: []} : {collaborators: []}), {status: 200})
  })
  if (kind === "members") await client.collaboration.acceptSessionInvite("synthetic-invite")
  const result = kind === "members" ? await client.collaboration.sessionMembers("session/&?") : await client.collaboration.collaborators()
  assert.deepEqual(result, kind === "members" ? {session_id: "session/&?", members: []} : [])
  assert.equal(requests, 2); assert.equal(rotations, 1)
})

// MP-08 / MP-11: scope metadata is local to a Cloud caller and exact session.
// It never grants membership, changes personal-account queries or imports
// another Cloud origin's session context.
test("MP-08 / MP-11: accepted account scopes remain separate across sessions and private callers", async t => {
  const original = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "personal", userId: "human", clientId: "terminal"}, clientId: "terminal", accessToken: "synthetic-access", publicKeyThumbprint: "a".repeat(64)}
  let credential = original
  const client = new CloudClient({load: async () => credential, session: async () => credential} as unknown as CloudClientCredentialStore, () => ({publicKeyThumbprint: original.publicKeyThumbprint}) as any)
  t.after(() => client.stop())
  const queries: string[][] = []
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input))
    if (url.pathname.startsWith("/sessions/invites/")) {
      const sessionId = url.pathname.split("/")[3]!
      if (sessionId === "denied") return Response.json({error: {code: "session_invite_invalid"}}, {status: 403})
      return Response.json({sessionId, accountId: `owner-${sessionId}`, userId: credential.profile.userId})
    }
    assert.equal(new Headers(init?.headers).get("authorization"), `Bearer ${credential.accessToken}`)
    const sessionId = url.searchParams.get("sessionId")!
    queries.push([url.searchParams.get("accountId")!, sessionId])
    return Response.json({sessionId, members: []})
  })
  await client.collaboration.acceptSessionInvite("first")
  await client.collaboration.acceptSessionInvite("second")
  await assert.rejects(client.collaboration.acceptSessionInvite("denied"), /session_invite_invalid/)
  for (const sessionId of ["first", "second", "personal-session", "denied"]) await client.collaboration.sessionMembers(sessionId)
  assert.deepEqual(queries.splice(0), [["owner-first", "first"], ["owner-second", "second"], ["personal", "personal-session"], ["personal", "denied"]])
  for (const changed of [
    {...original, profile: {...original.profile, apiUrl: "http://127.0.0.1:44124"}},
    {...original, profile: {...original.profile, userId: "other-human"}},
    {...original, profile: {...original.profile, accountId: "other-personal"}},
    {...original, clientId: "other-terminal"},
  ]) {
    credential = changed
    await client.collaboration.sessionMembers("first")
    assert.deepEqual(queries.pop(), [changed.profile.accountId, "first"])
  }
  credential = original
  await client.collaboration.sessionMembers("first")
  assert.deepEqual(queries.pop(), ["owner-first", "first"])
  assert.equal((await client.profile())?.accountId, "personal")
})
