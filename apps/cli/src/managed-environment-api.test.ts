import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "./ipc.js"
import type { CloudControlProfile } from "./cloud-control-auth.js"
import { listManagedEnvironmentCatalog, getManagedEnvironment, getManagedEnvironmentReimagePreflight, createManagedEnvironment, requestManagedEnvironmentLifecycle, requestManagedEnvironmentReimage, prepareManagedEnvironmentContextTransfer, observeManagedEnvironmentPreReimage, startManagedContextTransfer, getManagedContextTransferStatus, getManagedContextLaunchTarget } from "./managed-environment-api.js"

const metadata = {apiUrl: "http://127.0.0.1:44123", accountId: "account-1", userId: "owner-1"} as CloudControlProfile
const contextPlan = {sourceTargetId: null, kernelContext: "empty" as const, developmentSetup: {kind: "empty" as const}, providerAccounts: {kind: "none" as const}, gitCredentials: {kind: "none" as const}}
const reimageInput = {environmentId: "environment-1", expectedGeneration: 3, expectedProviderServerId: "123456789", expectedProviderImageId: "987654321", expectedProviderProfileId: "hetzner-path1", expectedProviderProfileDigest: `sha256:${"b".repeat(64)}`, expectedRuntimeReleaseDigest: `sha256:${"a".repeat(64)}`, expectedRuntimeSourceCommit: "c".repeat(40), expectedRuntimeSourceTree: "d".repeat(40), contextPlan, idempotencyKey: "reimage-1"}
function authority(respond: (url: URL, body: any) => unknown): CloudControlProfile {
  return {...metadata, authenticatedFetch: async (input, options) => Response.json(respond(new URL(input), options?.body ? JSON.parse(String(options.body)) : null))}
}

for (const action of ["create", "reimage"] as const) test(`MP-08 / MP-11 ${action} cannot bypass kernel credential portability preflight`, async () => {
  let requests = 0
  const profile = authority((url) => {
    requests++
    if (url.pathname.endsWith("/stop")) return {environment: {environmentId: "environment-1", accountId: metadata.accountId, runtimeGeneration: 4}, operation: {environmentId: "environment-1", requestedByUserId: metadata.userId, operationId: "reimage-operation", kind: "reimage", idempotencyKey: "reimage-1", requestDigest: `sha256:${"a".repeat(64)}`}}
    const result = reimageResult()
    return {...result, environment: {...result.environment, accountId: metadata.accountId}}
  })
  const selected = {...contextPlan, providerAccounts: {kind: "selected" as const, accounts: [{provider: "claude", accountProfile: "fixture"}]}}
  const request = action === "create"
    ? createManagedEnvironment(profile, {clientRequestId: "create-1", name: "fixture", region: "hel1", computeClass: "agent-small", managedRepositoryRoot: "/srv/chariox/repos", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: selected})
    : requestManagedEnvironmentReimage(profile, {...reimageInput, contextPlan: selected})
  await assert.rejects(request, /kernel credential portability preflight/)
  assert.equal(requests, 0, "reject before STOP or provisioning authority is spent")
})

// MP-08 / MP-11: selection is checked by the connected kernel before any
// human Cloud authority is spent, with no credential bytes in either transport.
for (const action of ["create", "reimage"] as const) for (const rejected of [false, true]) test(`MP-08 / MP-11 selected-provider ${action} ${rejected ? "rejects before mutation" : "preflights then mutates"}`, async () => {
  const calls: string[] = []
  const providerAccounts = {kind: "selected" as const, accounts: [{provider: "codex", accountProfile: "synthetic-profile"}]}
  const selected = {...contextPlan, providerAccounts}
  const client = {send: async (request: unknown) => {
    calls.push("kernel-preflight")
    assert.deepEqual(request, {PreflightProviderAccountPortability: {providerAccounts}})
    if (rejected) throw new Error("selected provider account has no transferable credentials")
    return {ProviderAccountPortabilityPreflightPassed: {}}
  }} as unknown as LocalIpcClient
  const profile = authority((url, body) => {
    calls.push(url.pathname.endsWith("/stop") ? "stop" : "mutation")
    assert.deepEqual(body.contextPlan, selected)
    if (url.pathname.endsWith("/stop")) return {environment: {environmentId: "environment-1", accountId: metadata.accountId, runtimeGeneration: 4}, operation: {environmentId: "environment-1", requestedByUserId: metadata.userId, operationId: "reimage-operation", kind: "reimage", idempotencyKey: "reimage-1", requestDigest: `sha256:${"a".repeat(64)}`}}
    const result = reimageResult()
    return {...result, environment: {...result.environment, accountId: metadata.accountId}}
  })
  const request = action === "create"
    ? createManagedEnvironment(profile, {clientRequestId: "create-1", name: "fixture", region: "hel1", computeClass: "agent-small", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: selected}, client)
    : requestManagedEnvironmentReimage(profile, {...reimageInput, contextPlan: selected}, client)
  if (rejected) await assert.rejects(request, /no transferable credentials/)
  else await request
  assert.deepEqual(calls, rejected ? ["kernel-preflight"] : action === "create" ? ["kernel-preflight", "mutation"] : ["kernel-preflight", "stop", "mutation"])
})

for (const response of [undefined, {ProviderAccountPortabilityPreflightPassed: {credential: "must-not-cross"}}, {Other: {}}]) test("MP-08 / MP-11 unexpected portability acknowledgement fails before Cloud", async () => {
  const profile = authority(() => {throw new Error("must not spend Cloud authority")})
  const client = {send: async () => response} as unknown as LocalIpcClient
  await assert.rejects(createManagedEnvironment(profile, {clientRequestId: "create-1", name: "fixture", region: "hel1", computeClass: "agent-small", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: {...contextPlan, providerAccounts: {kind: "selected", accounts: [{provider: "codex", accountProfile: "fixture"}]}}}, client), /acknowledgement|ProviderAccountPortabilityPreflightPassed/)
})

test("MP-08 / MP-11 selected-provider older kernel gives protocol 478 diagnostic", async () => {
  const profile = authority(() => {throw new Error("must not spend Cloud authority")})
  const client = {send: async () => {throw new Error("unknown variant `PreflightProviderAccountPortability`")}} as unknown as LocalIpcClient
  await assert.rejects(createManagedEnvironment(profile, {clientRequestId: "create-1", name: "fixture", region: "hel1", computeClass: "agent-small", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan: {...contextPlan, providerAccounts: {kind: "selected", accounts: [{provider: "codex", accountProfile: "fixture"}]}}}, client), /requires kernel protocol 478/)
})

// MP-08 / MP-11: human control transport has no kernel credentials, with local
// coordination covered separately on the ordinary LocalDaemon variants.
test("managed catalog and mutations use bound human HTTP metadata", async () => {
  const paths: string[] = [], bodies: any[] = []
  const profile = authority((url, body) => {
    paths.push(url.pathname)
    if (body) {bodies.push(body); assert.equal(body.accountId, metadata.accountId); assert.equal(body.kernelCredential, undefined); assert.equal(body.environmentId, undefined, "Cloud strictly accepts environment identity only in the path")}
    else assert.equal(url.searchParams.get("accountId"), metadata.accountId)
    if (url.pathname.endsWith("/options")) return {computeClasses: [], contextSources: []}
    if (url.pathname === "/managed-environments" && !body) return {environments: []}
    if (url.pathname.endsWith("/reimage/preflight")) return reimagePreflight()
    if (url.pathname.endsWith("/context-transfer")) return ticket()
    return {environment: {environmentId: "environment-1", accountId: metadata.accountId, managedRepositoryRoot: "/srv/chariox/repos"}, operation: {environmentId: "environment-1"}}
  })
  assert.deepEqual(await listManagedEnvironmentCatalog(profile), {computeClasses: [], contextSources: [], environments: []})
  assert.equal((await getManagedEnvironment(profile, "environment-1")).environmentId, "environment-1")
  await getManagedEnvironmentReimagePreflight(profile, "environment-1")
  const created = await createManagedEnvironment(profile, {clientRequestId: "create-root-1", name: "fixture", region: "hel1", computeClass: "agent-small", managedRepositoryRoot: "/srv/chariox/repos", autoStopPolicy: {minimumRuntimeSeconds: 0, idleDelaySeconds: 900}, contextPlan})
  assert.equal(created.environment.managedRepositoryRoot, "/srv/chariox/repos")
  await requestManagedEnvironmentLifecycle(profile, {environmentId: "environment-1", action: "start", idempotencyKey: "stable-start"})
  await prepareManagedEnvironmentContextTransfer(profile, "environment-1")
  assert.equal(bodies[0].clientRequestId, "create-root-1"); assert.equal(bodies[0].managedRepositoryRoot, "/srv/chariox/repos")
  assert.equal(bodies[1].idempotencyKey, "stable-start")
  assert.deepEqual(paths, ["/managed-environments/options", "/managed-environments", "/managed-environments/environment-1", "/managed-environments/environment-1/reimage/preflight", "/managed-environments", "/managed-environments/environment-1/lifecycle", "/managed-environments/environment-1/context-transfer"])
})

test("context transfer and installed-generation observation remain kernel-owned", async () => {
  const requests: unknown[] = [], responses = [
    {ManagedEnvironmentPreReimageObserved: {acknowledgement: {environmentId: "environment-1", generation: 3, observedAt: "2026-10-05T00:00:00Z"}}},
    {ManagedContextTransferStarted: {status: status("preparing")}},
    {ManagedContextTransferStatus: {status: status("completed")}},
    {ManagedContextLaunchTarget: {target: launchTarget()}},
  ]
  const client = {send: async (request: unknown) => {requests.push(request); return responses.shift()}} as unknown as LocalIpcClient
  await observeManagedEnvironmentPreReimage(client, {environmentId: "environment-1", expectedGeneration: 3})
  await startManagedContextTransfer(client, ticket())
  await getManagedContextTransferStatus(client, "context-1")
  await getManagedContextLaunchTarget(client, "context-1", "sha256:plan")
  assert.deepEqual(requests, [{ObserveManagedEnvironmentPreReimage: {environmentId: "environment-1", expectedGeneration: 3}}, {StartManagedContextTransfer: {ticket: ticket(), interactive: true}}, {GetManagedContextTransferStatus: {contextId: "context-1"}}, {GetManagedContextLaunchTarget: {contextId: "context-1", planDigest: "sha256:plan"}}])
})

for (const kind of ["get", "preflight", "ticket", "lifecycle"] as const) test(`managed ${kind} rejects another environment`, async () => {
  const profile = authority(() => ({...reimagePreflight(), ...ticket(), environmentId: "foreign", environment: {environmentId: "foreign", accountId: metadata.accountId}, operation: {environmentId: "foreign"}}))
  const promise = kind === "get" ? getManagedEnvironment(profile, "environment-1") : kind === "preflight" ? getManagedEnvironmentReimagePreflight(profile, "environment-1") : kind === "ticket" ? prepareManagedEnvironmentContextTransfer(profile, "environment-1") : requestManagedEnvironmentLifecycle(profile, {environmentId: "environment-1", action: "start", idempotencyKey: "stable"})
  await assert.rejects(promise, /different|mismatched/)
})

for (const invalid of [false, true]) test(`managed reimage ${invalid ? "rejects stale receipt" : "preserves explicit-empty context after exact replay admission"}`, async () => {
  const bodies: any[] = [], result = reimageResult()
  const profile = authority((url, body) => {
    bodies.push(body)
    assert.equal(body.environmentId, undefined, "STOP and reimage share Cloud's strict mutation schema")
    if (url.pathname.endsWith("/stop")) return {environment: {environmentId: "environment-1", accountId: metadata.accountId, runtimeGeneration: 4}, operation: {environmentId: "environment-1", requestedByUserId: metadata.userId, operationId: "reimage-operation", kind: "reimage", idempotencyKey: "reimage-1", requestDigest: `sha256:${"a".repeat(64)}`}}
    if (invalid) result.receipt.generation = 5
    return {...result, environment: {...result.environment, accountId: metadata.accountId}}
  })
  if (invalid) await assert.rejects(requestManagedEnvironmentReimage(profile, reimageInput), /reimage evidence/)
  else await requestManagedEnvironmentReimage(profile, reimageInput)
  assert.deepEqual(bodies.map(body => body.contextPlan), [contextPlan, contextPlan])
  assert.ok(bodies.every(body => body.idempotencyKey === "reimage-1"))
})

function ticket() {
  return {
    environmentId: "environment-1",
    contextPlan: {
      schemaVersion: 1 as const,
      contextId: "context-1",
      planDigest: "sha256:plan",
      source: null,
      kernelContext: "empty" as const,
      developmentSetup: { kind: "empty" as const },
      providerAccounts: { kind: "none" as const },
      gitCredentials: { kind: "none" as const },
    },
    target: {
      relayRealmId: "realm-1",
      machineId: "machine-1",
      kernelId: "kernel-1",
      relayPublicKey: "public-key",
      keyThumbprint: "sha256:key",
    },
  }
}

function status(phase: "preparing" | "completed") {
  return {
    contextId: "context-1",
    planDigest: "sha256:plan",
    phase,
    acceptedBytes: 0,
    packageSizeBytes: 0,
    retryable: false,
    updatedAtMs: 1,
  }
}

function launchTarget() {
  return {
    environmentId: "environment-1",
    kernelId: "kernel-1",
    contextId: "context-1",
    planDigest: "sha256:plan",
    development: { kind: "empty" as const, workspacePath: "/managed/workspace" },
  }
}

function reimageResult() {
  return {
    environment: { environmentId: "environment-1" },
    operation: {
      operationId: "operation-reimage-1",
      environmentId: "environment-1",
      kind: "reimage",
      idempotencyKey: "reimage-1",
    },
    receipt: {
      receiptId: "receipt-1",
      environmentId: "environment-1",
      operationId: "operation-reimage-1",
      previousGeneration: 3,
      generation: 4,
      providerServerId: "123456789",
      providerImageId: "987654321",
      providerProfileId: "hetzner-path1",
      providerProfileDigest: `sha256:${"b".repeat(64)}`,
      runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
      sourceEvidence: {
        providerImageId: "987654321",
        providerProfileId: "hetzner-path1",
        providerProfileDigest: `sha256:${"b".repeat(64)}`,
        runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
        runtimeSourceCommit: "c".repeat(40),
        runtimeSourceTree: "d".repeat(40),
      },
    },
  }
}

function reimagePreflight() {
  return {
    environmentId: "environment-1",
    retained: {
      providerServerId: "123456789",
      generation: 3,
      desiredRevision: 7,
      observedRevision: 7,
      runtimeMachineId: "managed-machine-3",
      runtimeKernelId: "managed-kernel-3",
      runtimeRelayRealmId: "managed-realm-3",
      runtimeReleaseDigest: `sha256:${"a".repeat(64)}`,
    },
    desiredRelease: {
      providerId: "hetzner" as const,
      providerImageId: "987654321",
      providerProfileId: "hetzner-path1",
      providerProfileDigest: `sha256:${"b".repeat(64)}`,
      runtimeReleaseDigest: `sha256:${"c".repeat(64)}`,
      runtimeSourceCommit: "d".repeat(40),
      runtimeSourceTree: "e".repeat(40),
    },
  }
}
