import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { createCliCommandActionComposition, type CliCommandActionCompositionDeps } from "./cli-command-action-composition.js"
import { CloudClient } from "./cloud-client.js"
import type { CloudClientCredentialStore } from "./cloud-client-credential-store.js"
import { makeCommandDeps, makeSession } from "./command-actions-test-support.js"

// MP-08 / MP-11: actual composition + slash handler, with both authority profiles.
test("signed-in attached terminal preserves kernel enrollment when unlink is rejected", async t => {
  const root=mkdtempSync(join(tmpdir(),"chariox-kauth-unlink-"))
  const previous=process.env.CHARIOX_HOME
  process.env.CHARIOX_HOME=root
  t.after(()=>{if(previous===undefined)delete process.env.CHARIOX_HOME;else process.env.CHARIOX_HOME=previous;rmSync(root,{recursive:true,force:true})})
  const notices:string[]=[]
  let preferenceWrites=0, unlinkRequests=0
  const human={apiUrl:"http://cloud.test",accountId:"account",clientId:"terminal",cloudSessionToken:"synthetic-client-access"}
  const client={send:async(request:Record<string,unknown>)=>{
    if("CloudRelayStatus" in request)return {CloudRelayStatus:{profile:{api_url:"http://cloud.test",account_id:"account",kernel_id:"kernel",kernel_enrolled:true}}}
    if("LogoutCloudRelay" in request){unlinkRequests++;throw new Error("Cloud rejected unlink")}
    throw new Error("unexpected request")
  }}
  const deps=new Proxy({...makeCommandDeps(),client,options:{clientId:"terminal",accountProfile:"default"},preferencesState:()=>({}),setPreferencesState:()=>{preferenceWrites++},kernelConnected:()=>true,cloudClient:{humanProfile:async()=>human},appendCloudNotice:(notice:string)=>notices.push(notice)}, {get:(target,key)=>key in target?target[key as keyof typeof target]:()=>{}})
  const handlers=createCliCommandActionComposition(deps as unknown as CliCommandActionCompositionDeps)
  await assert.rejects(handlers.handleCloudCommand({kind:"cloud",raw:"/cloud unlink",args:["unlink"]}),/Cloud rejected unlink/)
  assert.equal(unlinkRequests,1)
  assert.equal(preferenceWrites,0)
  assert.equal(notices.includes("cloud link cleared"),false)
})

// MP-08 / MP-11: the enrolled kernel contains no human authority. Cloud HTTP
// receives the private terminal profile; only local session mutations use IPC.
// A nonowner's private account may differ from the kernel owner's account.
const cloudCommandCases: {action: string; kind: "cloud" | "collab"; nonowner: boolean; rejectAt?: "cloud" | "kernel"}[] = [
  ...["deployments list", "invite create", "invite accept", "members", "collaborators"].map(action => ({action, kind: "cloud" as const, nonowner: false})),
  ...(["cloud", "collab"] as const).flatMap(kind =>
    ["invite create", "invite accept", "members", "collaborators"].map(action => ({action, kind, nonowner: true}))),
  ...(["cloud", "collab"] as const).flatMap(kind =>
    (["cloud", "kernel"] as const).map(rejectAt => ({action: "invite accept", kind, nonowner: true, rejectAt}))),
]
for (const {action, kind, nonowner, rejectAt} of cloudCommandCases) {
  test(`signed-in ${nonowner ? "nonowner" : "owner"} terminal uses human authority for /${kind} ${action}${rejectAt ? ` with ${rejectAt} denial` : ""}`, async t => {
    const session = makeSession(), notices: string[] = [], ipc: string[] = [], paths: string[] = []
    let applied = 0, attached = 0
    const profile = {apiUrl: "http://127.0.0.1:44123", accountId: nonowner ? "collaborator-account" : "account", userId: "human", clientId: "terminal", realmId: "realm", relayUrl: "wss://relay.test", email: "human@example.test", accountSlug: "fixture", issuerId: "fixture"}
    const credential = {profile, clientId: "terminal", publicKeyThumbprint: "a".repeat(64), accessToken: "synthetic-human-access", refreshCredential: "synthetic-refresh", expiresAtMs: Date.now()+60_000}
    const cloudClient = new CloudClient({load: async () => credential, session: async () => credential} as unknown as CloudClientCredentialStore, () => ({publicKeyThumbprint: credential.publicKeyThumbprint}) as any)
    t.after(() => cloudClient.stop())
    t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
      const url = new URL(String(input)); paths.push(url.pathname)
      const body = init?.body ? JSON.parse(String(init.body)) : null
      assert.equal(body?.sessionToken ?? new Headers(init?.headers).get("authorization"), body ? credential.accessToken : `Bearer ${credential.accessToken}`)
      assert.equal(body?.machineId, undefined)
      assert.equal(body?.kernelCredential, undefined)
      const results: Record<string, unknown> = {
        "/deployment-projects": {portfolio: []},
        "/sessions/invites": {inviteId: "cloud-invite", inviteToken: "cloud-invite-token", sessionId: session.id, accountId: profile.accountId, createdByUserId: profile.userId},
        "/sessions/invites/cloud-invite-token/accept": {userId: profile.userId, sessionId: session.id},
        "/sessions/members": {sessionId: session.id, members: [{userId: profile.userId, email: profile.email, displayName: "Human"}]},
        "/collaborators/recent": {collaborators: [{userId: "friend", email: "friend@example.test", sharedSessionCount: 2}]},
      }
      assert.ok(url.pathname in results, "expected client control-plane route")
      if (url.pathname === "/sessions/invites") {assert.equal(body.sessionId, session.id); assert.equal(body.collaborationLevel, "full")}
      if (init?.method !== "POST") assert.equal(url.searchParams.get("accountId"), profile.accountId)
      if (rejectAt === "cloud") return new Response(JSON.stringify({error: {code: "collaboration_denied"}}), {status: 403})
      return new Response(JSON.stringify(results[url.pathname]), {status: 200})
    })
    const client = {isRelayTransport: () => true, send: async (request: Record<string, any>) => {
      const variant = Object.keys(request)[0]!; ipc.push(variant)
      if (variant === "CloudRelayStatus") {
        if (nonowner) throw new Error("CloudRelayStatus requires the kernel owner")
        return {CloudRelayStatus: {profile: {api_url: profile.apiUrl, account_id: "account", kernel_id: "kernel", kernel_enrolled: true}}}
      }
      if (variant === "CreateSessionInvite") return {SessionInviteCreated: {session, invite: {invite_token: "local-invite-token", invite: {invite_id: "local-invite"}}}}
      if (variant === "JoinSessionInvite") {
        assert.equal(request.JoinSessionInvite.user_id, profile.userId)
        assert.equal(request.JoinSessionInvite.invite_token, "local-invite-token")
        if (rejectAt === "kernel") throw new Error("kernel membership denied")
        return {SessionInviteJoined: {session, member: {user_id: profile.userId}}}
      }
      throw new Error(`human Cloud requests cannot use kernel IPC: ${variant}`)
    }}
    const base = {...makeCommandDeps(), client, cloudClient, options: {clientId: "terminal", accountProfile: "default"}, preferencesState: () => ({}), kernelConnected: () => true, sessionState: () => session, appendNotice: (text: string) => notices.push(text), appendCloudNotice: (text: string) => notices.push(text), applySessionState: () => {applied++}, attachBinding: async () => {attached++}}
    const deps = new Proxy(base, {get: (target, key) => key in target ? target[key as keyof typeof target] : () => {}})
    const handlers = createCliCommandActionComposition(deps as unknown as CliCommandActionCompositionDeps)
    const args = action === "invite create" ? ["invite", "create", "--level", "full"] : action === "invite accept" ? ["invite", "accept", "http://127.0.0.1/invites?cloud_invite=cloud-invite-token&local_invite=local-invite-token"] : action.split(" ")
    const run = () => kind === "collab"
      ? handlers.handleCollabCommand({kind, raw: `/collab ${action}`, args})
      : handlers.handleCloudCommand({kind, raw: `/cloud ${action}`, args})
    if (rejectAt) await assert.rejects(run, rejectAt === "cloud" ? /collaboration_denied/ : /kernel membership denied/)
    else await run()
    assert.equal(paths.length, 1)
    if (nonowner) assert.equal(ipc.includes("CloudRelayStatus"), false, "human collaboration does not read owner-only enrollment status")
    assert.ok(!ipc.some(variant => /^(CreateCloud|AcceptCloud|ListCloud)/.test(variant)))
    assert.equal(ipc.filter(variant => variant === "JoinSessionInvite").length, action === "invite accept" && rejectAt !== "cloud" ? 1 : 0)
    assert.equal(applied, !rejectAt && action.startsWith("invite") ? 1 : 0)
    assert.equal(attached, !rejectAt && action === "invite accept" ? 1 : 0)
    if (action === "members") assert.ok(notices.some(value => value.includes("human human@example.test (Human)")))
    if (action === "collaborators") assert.ok(notices.some(value => value.includes("friend friend@example.test shared_sessions=2")))
  })
}

// MP-08 / MP-11: enrollment cannot stand in for a terminal login or cross realms.
for (const state of ["signed-out", "foreign-account", "foreign-cloud"] as const) {
  test(`attached deployment command rejects ${state} human authority`, async t => {
    const notices: string[] = []
    const human = state === "signed-out" ? null : {apiUrl: state === "foreign-cloud" ? "https://foreign.example.test" : "https://cloud.example.test", accountId: state === "foreign-account" ? "foreign" : "account", cloudSessionToken: "synthetic-human-access"}
    t.mock.method(globalThis, "fetch", () => {throw new Error("conflicting or absent authority reached Cloud")})
    const client = {send: async () => ({CloudRelayStatus: {profile: {api_url: "https://cloud.example.test", account_id: "account", kernel_enrolled: true}}})}
    const base = {...makeCommandDeps(), client, cloudClient: {humanProfile: async () => human}, options: {}, preferencesState: () => ({}), kernelConnected: () => true, flashFooter: (value: string) => notices.push(value)}
    const deps = new Proxy(base, {get: (target, key) => key in target ? target[key as keyof typeof target] : () => {}})
    const handlers = createCliCommandActionComposition(deps as unknown as CliCommandActionCompositionDeps)
    const command = {kind: "cloud" as const, raw: "/cloud deployments list", args: ["deployments", "list"]}
    if (human) await assert.rejects(handlers.handleCloudCommand(command), /Cloud account conflict/)
    else {await handlers.handleCloudCommand(command); assert.deepEqual(notices, ["sign in with /cloud login first"])}
  })
}
