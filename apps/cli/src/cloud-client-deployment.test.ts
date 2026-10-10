import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"
import { CloudClient } from "./cloud-client.js"
import { CloudClientCredentialStore } from "./cloud-client-credential-store.js"
import { createDeploymentSetup, checkpointDeploymentSetup } from "./deployed-workflow-setup-api.js"
import { createDeploymentProject } from "./deployed-workflow-api.js"
import { changePublicationDeployment } from "./publication-deployment-api.js"

test("MP-08 / MP-11 private control authority cannot cross profile or endpoint boundaries", async t => {
  const original = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "account", userId: "human"}, clientId: "terminal", accessToken: "synthetic-private", publicKeyThumbprint: "a".repeat(64)}
  let credential = original
  const client = new CloudClient({load: async () => credential, session: async () => credential} as any, () => ({publicKeyThumbprint: original.publicKeyThumbprint}) as any)
  t.after(() => client.stop())
  let requests = 0
  t.mock.method(globalThis, "fetch", async () => {requests++; return Response.json({})})
  const authority = (await client.humanProfile())!
  assert.equal(authority.cloudSessionToken, undefined, "public metadata never snapshots private access")
  await assert.rejects(authority.authenticatedFetch!("https://other.example.test/deployment-setups"), /profile_conflict/)
  await assert.rejects(authority.authenticatedFetch!("http://user:password@127.0.0.1:44123/deployment-setups"), /insecure_auth_endpoint/)
  for (const changed of [
    {...original, clientId: "other-terminal"},
    {...original, profile: {...original.profile, accountId: "other-account"}},
    {...original, profile: {...original.profile, userId: "other-human"}},
    {...original, profile: {...original.profile, apiUrl: "http://127.0.0.1:44124"}},
  ]) {
    credential = changed
    await assert.rejects(authority.authenticatedFetch!("http://127.0.0.1:44123/deployment-setups"), /profile_conflict/)
  }
  assert.equal(requests, 0, "reject before sending private human authority")
})

// MP-08 / MP-11: actual private stores model a second terminal process sharing
// the profile. One setup keeps its mutation identities through access rotation.
for (const rotation of ["between-requests", "during-request"] as const) test(`deployment setup survives ${rotation} by a second profile process`, async t => {
  const root = await mkdtemp(join(tmpdir(), "chariox-kauth-deployment-"))
  t.after(() => rm(root, {recursive: true, force: true}))
  const store = new CloudClientCredentialStore(join(root, "profile.json")), other = new CloudClientCredentialStore(store.filePath)
  const key = "a".repeat(64), profile = {apiUrl: "http://127.0.0.1:44123", accountId: "account", userId: "human", clientId: "terminal", email: "human@example.test", accountSlug: "fixture", realmId: "realm", relayUrl: "wss://relay.example.test", issuerId: "fixture"}
  await store.saveLogin({profile, clientId: "terminal", publicKeyThumbprint: key, accessToken: "synthetic-access-1", refreshCredential: "synthetic-refresh-1", expiresAtMs: Date.now()+600_000})
  const client = new CloudClient(store, () => ({publicKeyThumbprint: key}) as any)
  t.after(() => client.stop())
  let current = "synthetic-access-1", rotations = 0
  const checkpoints: string[] = []
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const url = new URL(String(input)), body = JSON.parse(String(init?.body))
    if (url.pathname === "/auth/client/refresh") {
      rotations++; current = `synthetic-access-${rotations+1}`
      return Response.json({refreshCredential: `synthetic-refresh-${rotations+1}`, cloudSessionToken: current, cloudSessionExpiresAt: new Date(Date.now()+600_000).toISOString()})
    }
    if (url.pathname.endsWith("/checkpoints")) {
      checkpoints.push(String(init?.body))
      if (rotation === "during-request" && checkpoints.length === 1) await other.session(key, true)
    }
    if (new Headers(init?.headers).get("authorization") !== `Bearer ${current}`) return Response.json({error: {code: "session_invalid", message: "expired snapshot"}}, {status: 401})
    assert.equal(body.accountId, profile.accountId)
    if (url.pathname === "/deployment-setups") {assert.equal(body.clientRequestId, "stable-create"); return Response.json({setup: {id: "setup", version: 1}})}
    assert.equal(url.pathname, "/deployment-setups/setup/checkpoints")
    assert.equal(body.operationKey, "stable-checkpoint"); assert.equal(body.expectedVersion, 1)
    return Response.json({setup: {id: "setup", version: 2}})
  })
  const authority = (await client.humanProfile())!
  await createDeploymentSetup(authority, {clientRequestId: "stable-create", origin: "draft", sourceSessionId: "session", sourceWorkflowId: "workflow", configuration: {} as any})
  if (rotation === "between-requests") await other.session(key, true)
  const result = await checkpointDeploymentSetup(authority, {setupId: "setup", expectedVersion: 1, operationKey: "stable-checkpoint", checkpoint: {kind: "credentials_ready"}})
  assert.equal(result.setup.version, 2)
  assert.equal(rotations, 1, "rejected access recovery reuses the other process's successor")
  assert.equal(checkpoints.length, rotation === "during-request" ? 2 : 1)
  if (checkpoints.length === 2) assert.equal(checkpoints[0], checkpoints[1], "retry preserves the complete mutation body")
})

for (const api of ["deployment", "publication"] as const) test(`${api} API recovers rejected human access through the private adapter`, async t => {
  const credential = {profile: {apiUrl: "http://127.0.0.1:44123", accountId: "account"}, accessToken: "synthetic-old", publicKeyThumbprint: "a".repeat(64)}
  let access = credential.accessToken, requests = 0
  const client = new CloudClient({load: async () => credential, session: async (_key: string, _force: boolean, rejected?: string) => {
    if (rejected) access = "synthetic-fresh"
    return {...credential, accessToken: access}
  }} as any, () => ({publicKeyThumbprint: credential.publicKeyThumbprint}) as any)
  t.after(() => client.stop())
  t.mock.method(globalThis, "fetch", async (_input: unknown, init?: RequestInit) => {
    requests++
    if (requests === 1) return Response.json({error: {code: "session_invalid"}}, {status: 401})
    assert.equal(new Headers(init?.headers).get("authorization"), "Bearer synthetic-fresh")
    return Response.json({project: {id: "project"}})
  })
  const authority = (await client.humanProfile())!
  if (api === "deployment") await createDeploymentProject(authority, {name: "fixture", kind: "workflow_endpoint", defaultRuntimeMode: "local_runtime"})
  else await changePublicationDeployment(authority, "fixture", "restart")
  assert.equal(requests, 2)
})
