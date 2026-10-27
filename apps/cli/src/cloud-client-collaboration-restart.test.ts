import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test, {type TestContext} from "node:test"
import { CloudClient } from "./cloud-client.js"
import { CloudClientCredentialStore, type CloudClientCredential } from "./cloud-client-credential-store.js"
import { createCliCommandActionComposition, type CliCommandActionCompositionDeps } from "./cli-command-action-composition.js"
import { makeCommandDeps, makeSession } from "./command-actions-test-support.js"

// MP-08 / MP-10 / MP-11: stateful pinned Cloud 3620f1245 contracts, with
// distinct personal/owner accounts. This fixture establishes storage/filter
// semantics, not a deployed Cloud/PostgreSQL acceptance result.
async function fixture(t: TestContext, kind: "cloud" | "collab") {
  const root = await mkdtemp(join(tmpdir(), "chariox-collaboration-restart-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const session = makeSession(), notices: string[] = [], ipc: string[] = [], reads: string[] = []
  const credential: CloudClientCredential = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "personal-B", userId: "B", clientId: "terminal-B", realmId: "personal-realm", relayUrl: "wss://relay.test", email: "B@example.test", accountSlug: "personal-B", issuerId: "fixture"}, clientId: "terminal-B", publicKeyThumbprint: "b".repeat(64), accessToken: "synthetic-B-access", refreshCredential: "synthetic-B-refresh", expiresAtMs: Date.now()+900_000}
  const filePath = join(root, "client.json")
  const store = new CloudClientCredentialStore(filePath)
  await store.saveLogin(credential)
  const clients: CloudClient[] = []
  let client: CloudClient, handlers: ReturnType<typeof createCliCommandActionComposition>, attached = false, accepts = 0
  const invite = {sessionId: session.id, accountId: "owner-A", createdByUserId: "A", maxUses: 1}
  const members = [{accountId: invite.accountId, sessionId: session.id, userId: "A", email: "A@example.test"}]
  const contacts: {accountId: string; userId: string; collaboratorUserId: string; hiddenAt: string | null; lastCollaboratedAt: string; sharedSessionCount: number}[] = []
  const recordContact = (accountId: string, userId: string, collaboratorUserId: string) => {
    const existing = contacts.find(row => row.accountId === accountId && row.userId === userId && row.collaboratorUserId === collaboratorUserId)
    if (existing) {existing.sharedSessionCount++; existing.hiddenAt = null; existing.lastCollaboratedAt = "2026-10-06T10:00:00Z"}
    else contacts.push({accountId, userId, collaboratorUserId, hiddenAt: null, lastCollaboratedAt: "2026-10-06T10:00:00Z", sharedSessionCount: 1})
  }
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const url = new URL(String(input)), body = init?.body ? JSON.parse(String(init.body)) : null
    assert.equal(body?.sessionToken ?? new Headers(init?.headers).get("authorization"), body ? credential.accessToken : `Bearer ${credential.accessToken}`)
    assert.equal(url.searchParams.has("sessionToken"), false)
    if (url.pathname.startsWith("/sessions/invites/") && url.pathname.endsWith("/accept")) {
      const name = url.pathname.split("/")[3]
      const selected = name === "one-use" ? invite : name === "second" ? {...invite, sessionId: "second-session", accountId: "owner-C"} : name === "third" ? {...invite, sessionId: "third-session"} : null
      if (!selected) return Response.json({error: {code: "session_invite_invalid"}}, {status: 403})
      if (name === "one-use" && accepts >= invite.maxUses) return Response.json({error: {code: "session_invite_invalid"}}, {status: 403})
      if (name === "one-use") accepts++
      members.push({accountId: selected.accountId, sessionId: selected.sessionId, userId: "B", email: credential.profile.email})
      recordContact(selected.accountId, "A", "B"); recordContact(selected.accountId, "B", "A")
      return Response.json({...selected, userId: "B", invitedByUserId: "A", joinedAt: "2026-10-06T10:00:00Z"})
    }
    const accountId = url.searchParams.get("accountId")!
    reads.push(accountId)
    if (url.pathname === "/sessions/members") {
      const sessionId = url.searchParams.get("sessionId")
      if (!members.some(row => row.accountId === accountId && row.sessionId === sessionId && row.userId === "B")) return Response.json({error: {code: "session_invite_invalid"}}, {status: 403})
      return Response.json({sessionId, members: members.filter(row => row.accountId === accountId && row.sessionId === sessionId)})
    }
    assert.equal(url.pathname, "/collaborators/recent")
    return Response.json({collaborators: contacts.filter(row => row.accountId === accountId && row.userId === "B" && row.hiddenAt === null)
      .sort((a, b) => Date.parse(b.lastCollaboratedAt)-Date.parse(a.lastCollaboratedAt)).slice(0, 25)
      .map(row => ({userId: row.collaboratorUserId, email: `${row.collaboratorUserId}@example.test`, lastCollaboratedAt: row.lastCollaboratedAt, sharedSessionCount: row.sharedSessionCount}))})
  })
  const kernel = {isRelayTransport: () => true, send: async (request: Record<string, any>) => {
    const variant = Object.keys(request)[0]!; ipc.push(variant)
    if (variant === "JoinSessionInvite") {
      assert.equal(request.JoinSessionInvite.user_id, "B")
      return {SessionInviteJoined: {session, member: {user_id: "B"}}}
    }
    assert.equal(variant, "AttachToSession", "B never requests owner-only CloudRelayStatus")
    assert.equal(request.AttachToSession.session_id, session.id)
    attached = true
    return {SessionAttached: {session, attachment: {session_id: session.id}}}
  }}
  const attach = async () => {await kernel.send({AttachToSession: {session_id: session.id, client_id: "terminal-B"}})}
  const start = () => {
    client = new CloudClient(new CloudClientCredentialStore(filePath), () => ({publicKeyThumbprint: credential.publicKeyThumbprint}) as any)
    clients.push(client)
    const base = {...makeCommandDeps(), client: kernel, cloudClient: client, options: {}, preferencesState: () => ({}), kernelConnected: () => true, sessionState: () => session, isAttached: () => attached, appendNotice: (text: string) => notices.push(text), applySessionState: () => {}, attachBinding: attach}
    const deps = new Proxy(base, {get: (target, key) => key in target ? target[key as keyof typeof target] : () => {}})
    handlers = createCliCommandActionComposition(deps as unknown as CliCommandActionCompositionDeps)
  }
  start()
  t.after(() => {for (const client of clients) client.stop()})
  const run = (args: string[]) => kind === "cloud" ? handlers.handleCloudCommand({kind, raw: `/cloud ${args.join(" ")}`, args}) : handlers.handleCollabCommand({kind, raw: `/collab ${args.join(" ")}`, args})
  return {session, credential, store, contacts, notices, ipc, reads, recordContact, run,
    accept: () => run(["invite", "accept", "http://127.0.0.1/invite?cloud_invite=one-use&local_invite=local-invite"]),
    get client() {return client}, accepts: () => accepts,
    async restart() {client.stop(); attached = false; start(); await client.resume(); await attach()},
  }
}

for (const kind of ["cloud", "collab"] as const) {
  test(`MP-08 / MP-11: /${kind} members restores one-use cross-account scope from the private profile after reattachment`, async t => {
    const f = await fixture(t, kind)
    await f.accept()
    await assert.rejects(f.client.collaboration.acceptSessionInvite("one-use"), /session_invite_invalid/)
    await f.restart()
    await f.run(["members"]); await f.run(["members", "list"])
    assert.equal(f.accepts(), 1)
    assert.deepEqual(f.ipc, ["JoinSessionInvite", "AttachToSession", "AttachToSession"])
    assert.ok(f.notices.some(value => value.includes("A A@example.test") && value.includes("B B@example.test")))
    assert.deepEqual(f.reads, ["owner-A", "owner-A"])
    const publicProfile = await f.client.profile()
    assert.equal(publicProfile?.accountId, "personal-B")
    assert.ok(!Object.keys(publicProfile!).some(key => /Scope|loginId|Credential|Token/.test(key)))
  })

  test(`MP-08 / MP-11: /${kind} collaborators includes reciprocal owner-account contacts immediately and after restart`, async t => {
    const f = await fixture(t, kind)
    await f.accept()
    // The caller's own account contacts also remain visible. Hidden rows,
    // reciprocal rows belonging to A, and unknown account scopes stay private.
    f.recordContact("personal-B", "B", "friend")
    f.recordContact("owner-A", "B", "hidden"); f.contacts.at(-1)!.hiddenAt = "2026-10-06T11:00:00Z"
    f.recordContact("unrelated", "B", "foreign")
    for (const restart of [false, true]) {
      if (restart) await f.restart()
      f.notices.length = 0
      await f.run(["collaborators", "list"])
      assert.ok(f.notices.some(value => value.includes("A A@example.test shared_sessions=1") && value.includes("friend friend@example.test shared_sessions=1")))
      assert.ok(!f.notices.some(value => /hidden|foreign|B@example/.test(value)))
      assert.deepEqual(f.reads.splice(0).sort(), ["owner-A", "personal-B"])
      assert.equal((await f.client.profile())?.accountId, "personal-B")
    }
    assert.equal(f.accepts(), 1)
  })
}


test("MP-08 / MP-11: recent contacts merge unique account scopes by user, total counts, newest metadata and caller-wide limit", async t => {
  const f = await fixture(t, "cloud")
  await f.accept()
  await f.client.collaboration.acceptSessionInvite("second")
  await f.client.collaboration.acceptSessionInvite("third")
  f.recordContact("personal-B", "B", "A"); f.recordContact("personal-B", "B", "A")
  const newest = f.contacts.find(row => row.accountId === "owner-C" && row.userId === "B")!
  newest.lastCollaboratedAt = "2026-10-06T11:00:00Z"
  await f.restart()
  const listed = await f.client.collaboration.collaborators()
  assert.equal(listed.length, 1)
  assert.equal(listed[0]!.user_id, "A")
  assert.equal(listed[0]!.shared_session_count, 5)
  assert.equal(listed[0]!.last_collaborated_at, newest.lastCollaboratedAt)
  assert.deepEqual(f.reads.splice(0).sort(), ["owner-A", "owner-C", "personal-B"], "two sessions under A do not double-query its contact counts")
  for (let index = 0; index < 30; index++) {
    f.recordContact("personal-B", "B", `friend-${index}`)
    f.contacts.at(-1)!.lastCollaboratedAt = new Date(Date.parse("2026-10-06T12:00:00Z")+index*1000).toISOString()
  }
  const bounded = await f.client.collaboration.collaborators()
  assert.equal(bounded.length, 25)
  assert.equal(bounded[0]!.user_id, "friend-29")
  assert.equal(bounded.at(-1)!.user_id, "friend-5")
})
