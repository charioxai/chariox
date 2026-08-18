import { createHmac, generateKeyPairSync, sign } from "node:crypto"

import {
  acquireAgentAppReplica,
  appendCloudPublicationDeploymentLogs,
  assert,
  baseConfig,
  buildServer,
  clearAgentAppEffectStoresForTests,
  clearAgentAppReplicaPoolsForTests,
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  createServer,
  enqueueAgentAppReplicaDispatch,
  ensurePublicationRuntimeAttached,
  findWorkflowRunByInvocationRequestId,
  firstSetCookieValue,
  invokePublicationInput,
  join,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  mkdir,
  mkdtemp,
  promptFromInvocationInput,
  providerCatalogResponse,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
  publicationForAgentAppInvocation,
  publicationInvocationEnvelope,
  publishedHttpConfig,
  readFile,
  registerCloudPublicationDeploymentBackend,
  releaseAgentAppReplicaInvocation,
  rememberAgentAppInvocationRoute,
  rm,
  setOptionalEnv,
  test,
  tmpdir,
  visibleWorkflowRun,
  waitForCondition,
  writeFile,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"
import {
  configurePublicationCallerClaimsRuntimeForTests,
  readPrivatePublicationCallerClaimsConfigFile,
  verifyPublicationCallerClaims,
} from "../publication-caller-claims.js"
import {
  agentAppCallerSession,
} from "../publication-agent-app-replicas.js"

test("agent app replica selection preserves caller affinity across hidden sessions", async () => {
  const selectedReplicas: unknown[] = []
  const selectedRequestIds: string[] = []
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-replicas-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    replica_session_ids: ["replica-session-1", "replica-session-2"],
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      replicas: { count: 2, per_caller_ordering: true },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
      }],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      selectedReplicas.push((invocation.caller.proof as Record<string, unknown>).replica_session_id)
      selectedRequestIds.push(invocation.request_id)
      return {
        accepted: true,
        workflow_run: { id: `run-${selectedReplicas.length}`, status: "Running" },
      }
    },
  })

  try {
    const callerA = await app.inject({ method: "GET", url: "/add/apples", headers: { accept: "text/html" } })
    const callerACookie = firstSetCookieValue(callerA.headers["set-cookie"])
    await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html", "x-chariox-agent-app-caller": "caller-b" } })
    const queued = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html", cookie: callerACookie } })
    assert.match(queued.body, /agent_app_pool_queued/)
    assert.deepEqual(selectedReplicas, ["replica-session-1", "replica-session-2"])

    releaseAgentAppReplicaInvocation(baseConfig, selectedRequestIds[0])
    await waitForCondition(
      () => selectedReplicas.length === 3,
      "same caller should resume after its affinity replica is released",
    )
    assert.deepEqual(selectedReplicas, ["replica-session-1", "replica-session-2", "replica-session-1"])
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app caller affinity ignores the legacy caller header outside managed Cloud", () => {
  configurePublicationCallerClaimsRuntimeForTests(null)
  try {
    const caller = agentAppCallerSession({
      "x-chariox-agent-app-caller": "forged-caller",
      "x-chariox-caller-roles": "admin",
      "x-chariox-caller-subject": "forged-subject",
    }, () => "generated-session")
    assert.equal(caller.callerKey, "generated-session")
    assert.match(caller.setCookie ?? "", /chariox_agent_app_session=generated-session/)
  } finally {
    configurePublicationCallerClaimsRuntimeForTests(undefined)
  }
})

test("managed agent app runtime verifies caller claims before affinity and role selection", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-caller-claims-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>secure app</main>")
  await writeFile(join(root, "publication.json"), JSON.stringify({
    schema_version: 1,
    package_version: 3,
    publication_id: "pub-caller-claims",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }))
  await writeFile(join(root, "deployment-contract.json"), JSON.stringify({
    schema_version: 1,
    package_id: `sha256:${"3".repeat(64)}`,
    artifact: {
      content_digest: `sha256:${"4".repeat(64)}`,
      digest_algorithm: "sha256",
      digest_scope: "package_files_excluding_deployment_contract",
    },
    source: {
      publication_id: "pub-caller-claims",
      session_id: "session-1",
      workflow_id: "workflow-1",
      endpoint_id: "endpoint-1",
      creator_user_id: "user-1",
      captured_at_ms: 1,
    },
    compatibility: {
      package_version: 3,
      minimum_kernel_version: "0.1.0",
      minimum_local_daemon_protocol_version: 1,
    },
    routes: [{ id: "secure-hook", path: "/secure/*", required_roles: ["member"] }],
    provider_requirements: [],
    credential_slots: [],
    configuration: [],
    capabilities: {
      network: {
        policy_version: 1,
        default_action: "deny",
        destinations: [],
        provider_access: [],
      },
    },
    resources: {},
    presentation: {},
    signatures: [],
  }))
  const callerSessions: unknown[] = []
  const invocationCallers: unknown[] = []
  configurePublicationCallerClaimsRuntimeForTests({
    deploymentId: "deployment-1",
    environmentId: "environment-1",
    secret: CALLER_CLAIMS_SECRET,
    now: () => new Date(CALLER_CLAIMS_NOW_SECONDS * 1_000),
  })
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-caller-claims",
    hook_id: "secure-hook",
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/secure/*",
        hook_id: "secure-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
      }],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      callerSessions.push((invocation.caller.proof as Record<string, unknown>).agent_app_session)
      invocationCallers.push(invocation.caller)
      return {
        accepted: true,
        workflow_run: { id: `run-${callerSessions.length}`, status: "Running" },
      }
    },
  })

  try {
    const asset = await app.inject({
      method: "GET",
      url: "/",
      headers: signedCallerClaimsHeaders({
        invocationId: "invocation-asset",
        nonce: "nonce-asset",
        subject: "user:trusted-user",
        roles: ["member"],
      }),
    })
    assert.equal(asset.statusCode, 200)
    assert.deepEqual(callerSessions, [])

    const validHeaders = signedCallerClaimsHeaders({
      invocationId: "invocation-valid",
      nonce: "nonce-valid",
      subject: "user:trusted-user",
      roles: ["member"],
    })
    const valid = await app.inject({
      method: "GET",
      url: "/secure/report",
      headers: {
        accept: "text/html",
        ...validHeaders,
        "x-chariox-agent-app-caller": "forged-affinity",
        "x-chariox-caller-subject": "forged-subject",
        "x-chariox-caller-roles": "admin",
      },
    })
    assert.equal(valid.statusCode, 200)
    assert.deepEqual(callerSessions, ["user:trusted-user"])
    assert.deepEqual(invocationCallers, [{
      type: "authenticated",
      proof: {
        transport: "agent_app_human_http",
        route: "/secure/*",
        agent_app_session: "user:trusted-user",
        agent_app_request_id: (invocationCallers[0] as { proof: { agent_app_request_id: string } }).proof.agent_app_request_id,
        replica_session_id: "session-1",
        agent_app_actions: {},
        agent_app_audit: undefined,
        publication_caller: {
          account_id: "account-1",
          deployment_id: "deployment-1",
          environment_id: "environment-1",
          invocation_id: "invocation-valid",
          roles: ["member"],
          subject: "user:trusted-user",
        },
      },
    }])
    assert.equal(valid.headers["set-cookie"], undefined)

    const replay = await app.inject({ method: "GET", url: "/secure/replay", headers: validHeaders })
    assert.equal(replay.statusCode, 401)
    assert.equal(replay.json().error.code, "caller_claims_invalid")

    const rejectedRequests: Array<{
      readonly label: string
      readonly headers: Record<string, string>
      readonly statusCode: 401 | 403
    }> = [
      {
        label: "missing claims",
        headers: {
          "x-chariox-agent-app-caller": "forged-affinity",
        },
        statusCode: 401,
      },
      {
        label: "malformed claims",
        headers: {
          "x-chariox-caller-claims": "not-a-token",
          "x-chariox-invocation-id": "invocation-malformed",
        },
        statusCode: 401,
      },
      {
        label: "forged signature",
        headers: signedCallerClaimsHeaders({
          invocationId: "invocation-forged",
          nonce: "nonce-forged",
          secret: "forged-secret-that-is-at-least-32-bytes",
        }),
        statusCode: 401,
      },
      {
        label: "wrong deployment",
        headers: signedCallerClaimsHeaders({
          deploymentId: "deployment-2",
          invocationId: "invocation-deployment",
          nonce: "nonce-deployment",
        }),
        statusCode: 401,
      },
      {
        label: "wrong environment",
        headers: signedCallerClaimsHeaders({
          environmentId: "environment-2",
          invocationId: "invocation-environment",
          nonce: "nonce-environment",
        }),
        statusCode: 401,
      },
      {
        label: "expired claims",
        headers: signedCallerClaimsHeaders({
          issuedAtSeconds: CALLER_CLAIMS_NOW_SECONDS - 120,
          expiresAtSeconds: CALLER_CLAIMS_NOW_SECONDS - 60,
          invocationId: "invocation-expired",
          nonce: "nonce-expired",
        }),
        statusCode: 401,
      },
      {
        label: "wrong role",
        headers: signedCallerClaimsHeaders({
          invocationId: "invocation-role",
          nonce: "nonce-role",
          roles: ["public"],
        }),
        statusCode: 403,
      },
      {
        label: "wrong invocation projection",
        headers: signedCallerClaimsHeaders({
          invocationId: "invocation-claim",
          invocationHeader: "invocation-header",
          nonce: "nonce-invocation",
        }),
        statusCode: 401,
      },
      {
        label: "replayed nonce",
        headers: signedCallerClaimsHeaders({
          invocationId: "invocation-replayed-nonce",
          nonce: "nonce-asset",
        }),
        statusCode: 401,
      },
      {
        label: "replayed invocation id",
        headers: signedCallerClaimsHeaders({
          invocationId: "invocation-asset",
          nonce: "nonce-replayed-invocation",
        }),
        statusCode: 401,
      },
    ]
    for (const rejected of rejectedRequests) {
      const response = await app.inject({
        method: "GET",
        url: `/secure/${encodeURIComponent(rejected.label)}`,
        headers: { accept: "text/html", ...rejected.headers },
      })
      assert.equal(response.statusCode, rejected.statusCode, rejected.label)
      const body = response.json() as { error: { code: string; message: string } }
      if (rejected.statusCode === 403) {
        assert.equal(body.error.code, "caller_role_denied", rejected.label)
        assert.equal(body.error.message, "Publication caller does not have the required role", rejected.label)
      } else {
        assert.ok(
          body.error.code === "caller_authentication_required" || body.error.code === "caller_claims_invalid",
          rejected.label,
        )
        assert.equal(body.error.message, "Publication caller authentication is required", rejected.label)
      }
      assert.equal(response.headers["cache-control"], "no-store", rejected.label)
    }
    assert.deepEqual(callerSessions, ["user:trusted-user"])
  } finally {
    await app.close()
    configurePublicationCallerClaimsRuntimeForTests(undefined)
    clearAgentAppReplicaPoolsForTests()
    await rm(root, { recursive: true, force: true })
  }
})

test("publication caller claims config file is private and consumed once", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-caller-claims-config-"))
  const configPath = join(root, "caller-claims.json")
  const insecureConfigPath = join(root, "caller-claims-insecure.json")
  const publicKeyConfigPath = join(root, "caller-claims-public-key.json")
  try {
    await writeFile(configPath, JSON.stringify({
      schema_version: 1,
      deployment_id: "deployment-1",
      environment_id: "environment-1",
      secret: CALLER_CLAIMS_SECRET,
    }), { mode: 0o600 })
    assert.deepEqual(readPrivatePublicationCallerClaimsConfigFile(configPath), {
      deploymentId: "deployment-1",
      environmentId: "environment-1",
      secret: CALLER_CLAIMS_SECRET,
    })
    await assert.rejects(readFile(configPath, "utf8"), /ENOENT/)

    const publicKeyPem = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA/pMgE2dD4Y9eL57S6f9+lve+T2A4M0ueD5GmOZfHjkI=\n-----END PUBLIC KEY-----\n"
    await writeFile(publicKeyConfigPath, JSON.stringify({
      schema_version: 1,
      deployment_id: "deployment-1",
      environment_id: "environment-1",
      public_key_pem: publicKeyPem,
    }), { mode: 0o600 })
    assert.deepEqual(readPrivatePublicationCallerClaimsConfigFile(publicKeyConfigPath), {
      deploymentId: "deployment-1",
      environmentId: "environment-1",
      publicKeyPem,
    })

    await writeFile(insecureConfigPath, JSON.stringify({
      schema_version: 1,
      deployment_id: "deployment-1",
      environment_id: "environment-1",
      secret: CALLER_CLAIMS_SECRET,
    }), { mode: 0o640 })
    assert.throws(
      () => readPrivatePublicationCallerClaimsConfigFile(insecureConfigPath),
      /owned regular file with mode 0600/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("publication caller claims accept Ed25519 signatures with a public-only runtime verifier", (t) => {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519")
  const publicKeyPem = publicKey.export({ format: "pem", type: "spki" }).toString()
  configurePublicationCallerClaimsRuntimeForTests({
    deploymentId: "deployment-1",
    environmentId: "environment-1",
    publicKeyPem,
    now: () => new Date(CALLER_CLAIMS_NOW_SECONDS * 1_000),
  })
  t.after(() => configurePublicationCallerClaimsRuntimeForTests(undefined))

  const encodedHeader = Buffer.from(JSON.stringify({ alg: "EdDSA", typ: "JWT" })).toString("base64url")
  const encodedPayload = Buffer.from(JSON.stringify({
    iss: "chariox-cloud",
    aud: "deployment-1",
    sub: "user:user-1",
    org: "account-1",
    roles: ["member"],
    deployment_id: "deployment-1",
    environment_id: "environment-1",
    invocation_id: "invocation-ed25519",
    nonce: "nonce-ed25519",
    iat: CALLER_CLAIMS_NOW_SECONDS,
    exp: CALLER_CLAIMS_NOW_SECONDS + 60,
  })).toString("base64url")
  const unsigned = `${encodedHeader}.${encodedPayload}`
  const token = `${unsigned}.${sign(null, Buffer.from(unsigned), privateKey).toString("base64url")}`

  assert.deepEqual(verifyPublicationCallerClaims({
    "x-chariox-caller-claims": token,
    "x-chariox-invocation-id": "invocation-ed25519",
  }), {
    accountId: "account-1",
    deploymentId: "deployment-1",
    environmentId: "environment-1",
    expiresAt: new Date((CALLER_CLAIMS_NOW_SECONDS + 60) * 1_000),
    invocationId: "invocation-ed25519",
    issuedAt: new Date(CALLER_CLAIMS_NOW_SECONDS * 1_000),
    nonce: "nonce-ed25519",
    roles: ["member"],
    subject: "user:user-1",
  })
})

test("agent app replica scheduler preserves caller order without blocking other callers", async () => {
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-replica-caller-order",
    replica_session_ids: ["replica-session-1", "replica-session-2"],
    agent_app: {
      enabled: true,
      replicas: { count: 2, per_caller_ordering: true, max_queue_depth: 4 },
      routes: [],
    },
  }
  const callerA = acquireAgentAppReplica(publication, "caller-a")
  const callerB = acquireAgentAppReplica(publication, "caller-b")
  assert.equal(callerA?.publication.session_id, "replica-session-1")
  assert.equal(callerB?.publication.session_id, "replica-session-2")
  assert.equal(acquireAgentAppReplica(publication, "caller-a"), null)

  const dispatched: string[] = []
  assert.equal(enqueueAgentAppReplicaDispatch(publication, "caller-a", async (lease) => {
    dispatched.push(`caller-a:${lease.publication.session_id}`)
    lease.release()
  }), true)
  assert.equal(enqueueAgentAppReplicaDispatch(publication, "caller-c", async (lease) => {
    dispatched.push(`caller-c:${lease.publication.session_id}`)
    lease.release()
  }), true)

  callerB?.release()
  await waitForCondition(
    () => dispatched.length === 1,
    "an eligible caller should use the idle replica",
  )
  assert.deepEqual(dispatched, ["caller-c:replica-session-2"])

  callerA?.release()
  await waitForCondition(
    () => dispatched.length === 2,
    "the queued affinity caller should resume in order",
  )
  assert.deepEqual(dispatched, [
    "caller-c:replica-session-2",
    "caller-a:replica-session-1",
  ])
})

test("agent app single-replica configuration admits only one invocation at a time", () => {
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-single-replica-capacity",
    replica_session_ids: ["replica-session-1"],
    agent_app: {
      enabled: true,
      replicas: { count: 1, per_caller_ordering: true },
      routes: [],
    },
  }
  const first = acquireAgentAppReplica(publication, "caller-a")
  assert.equal(first?.publication.session_id, "replica-session-1")
  assert.equal(acquireAgentAppReplica(publication, "caller-b"), null)
  first?.release()
  const second = acquireAgentAppReplica(publication, "caller-b")
  assert.equal(second?.publication.session_id, "replica-session-1")
  second?.release()
})

test("agent app replica caller affinity survives gateway restart with runtime storage", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-replica-state-"))
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-replica-state",
    package_root: root,
    replica_session_ids: ["replica-session-1", "replica-session-2"],
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      replicas: { count: 2, per_caller_ordering: true },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
      }],
    },
  }

  try {
    const first = acquireAgentAppReplica(publication, "caller-a")
    assert.equal(first?.publication.session_id, "replica-session-1")
    first?.release()
    const second = acquireAgentAppReplica(publication, "caller-b")
    assert.equal(second?.publication.session_id, "replica-session-2")
    second?.release()

    clearAgentAppReplicaPoolsForTests()
    const restored = acquireAgentAppReplica(publication, "caller-a")
    assert.equal(restored?.publication.session_id, "replica-session-1")
    restored?.release()
  } finally {
    clearAgentAppReplicaPoolsForTests()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app replica scheduler queues different callers until a replica is idle", async () => {
  const invocations: Array<{ caller: unknown; requestId: string; replica: unknown }> = []
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-replica-queue-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-replica-queue",
    transport: "human_http",
    package_root: root,
    replica_session_ids: ["replica-session-1", "replica-session-2"],
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      replicas: { count: 2, per_caller_ordering: true, max_queue_depth: 2 },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
      }],
    },
  }
  const { app } = buildServer(publication, {
    invokeWorkflow: async (invocation) => {
      const proof = invocation.caller.proof as Record<string, unknown>
      invocations.push({
        caller: proof.agent_app_session,
        requestId: invocation.request_id,
        replica: proof.replica_session_id,
      })
      return {
        accepted: true,
        workflow_run: { id: `run-${invocations.length}`, status: "Running" },
      }
    },
  })

  try {
    await app.inject({ method: "GET", url: "/add/apples", headers: { accept: "text/html" } })
    await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html" } })
    const queued = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html", "x-chariox-agent-app-caller": "caller-c" } })
    const queuedCookie = firstSetCookieValue(queued.headers["set-cookie"])

    assert.equal(queued.statusCode, 200)
    assert.match(queued.body, /agent_app_pool_queued/)
    assert.deepEqual(invocations.map((invocation) => invocation.replica), ["replica-session-1", "replica-session-2"])
    const saturatedStatus = await app.inject({ method: "GET", url: "/.well-known/chariox/agent-app/status" })
    assert.equal(saturatedStatus.statusCode, 200)
    assert.deepEqual(saturatedStatus.json(), {
      publication_id: "pub-replica-queue",
      replicas: {
        totalReplicaCount: 2,
        activeReplicaCount: 2,
        readyReplicaCount: 0,
        queueDepth: 1,
      },
    })

    releaseAgentAppReplicaInvocation(publication, invocations[0]?.requestId)
    await waitForCondition(
      () => invocations.length === 3,
      "queued caller should dispatch after a replica is released",
    )
    assert.equal(invocations[2]?.caller, agentAppSessionId(queuedCookie))
    assert.equal(invocations[2]?.replica, "replica-session-1")
    const drainedStatus = await app.inject({ method: "GET", url: "/.well-known/chariox/agent-app/status" })
    assert.equal(drainedStatus.statusCode, 200)
    assert.deepEqual(drainedStatus.json(), {
      publication_id: "pub-replica-queue",
      replicas: {
        totalReplicaCount: 2,
        activeReplicaCount: 2,
        readyReplicaCount: 0,
        queueDepth: 0,
      },
    })
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app invocation event streams resolve the selected replica session", () => {
  const route = {
    path: "/add/*",
    hook_id: "pub-test-hook",
    prompt_source: "path_tail" as const,
    response: "streaming_shell" as const,
  }
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-replica-stream",
    session_id: "base-session",
    agent_app: {
      enabled: true,
      routes: [route],
    },
  }
  rememberAgentAppInvocationRoute(
    publication,
    "request-1",
    route,
    { runtimeSessionId: "replica-session-2" },
  )

  assert.equal(publicationForAgentAppInvocation(publication, "request-1").session_id, "replica-session-2")
  assert.equal(publicationForAgentAppInvocation(publication, "missing-request").session_id, "base-session")
})

test("agent app replica sessions use distinct kernel attachment clients", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    async send(request: Record<string, unknown>) {
      requests.push(request)
      return { SessionAttached: { attachment: { id: `attachment-${requests.length}` } } }
    },
  }
  const publication = {
    ...baseConfig,
    publication_id: "pub-replica-runtime-attachments",
  }

  await ensurePublicationRuntimeAttached(client, { ...publication, session_id: "replica-session-1" })
  await ensurePublicationRuntimeAttached(client, { ...publication, session_id: "replica-session-2" })

  const attachments = requests.map((request) => request.AttachToSession as {
    readonly client_id: string
    readonly session_id: string
  })
  assert.deepEqual(attachments.map(({ session_id }) => session_id), ["replica-session-1", "replica-session-2"])
  assert.equal(new Set(attachments.map(({ client_id }) => client_id)).size, 2)
})

test("agent app overlay effects cannot write protected paths", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-protected-overlay-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          scope: "session",
          allowed_paths: ["/payments/**"],
          protected_paths: ["/payments/**"],
        },
      }],
    },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-shopping",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/payments/checkout.html" },
            effects: {
              overlay: [{
                path: "/payments/checkout.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>protected checkout</main>",
              }],
            },
          }),
        },
      },
    }),
  })

  try {
    const invoke = await app.inject({
      method: "GET",
      url: "/add/1%20kg%20bananas",
      headers: { accept: "text/html" },
    })
    assert.equal(invoke.statusCode, 200)

    const checkout = await app.inject({ method: "GET", url: "/payments/checkout.html" })
    assert.equal(checkout.statusCode, 404)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app persistent patch effects require package and route opt-in", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-persistent-reject-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      persistent_patch: { enabled: false },
      routes: [{
        path: "/admin/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "admin",
        manipulation: {
          level: "persistent_patch",
          scope: "persistent",
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-patch",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/banner.html" },
            effects: {
              persistent_patch: [{
                path: "/generated/banner.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>patched banner</main>",
              }],
            },
          }),
        },
      },
    }),
  })

  try {
    const invoke = await app.inject({ method: "GET", url: "/admin/banner", headers: { accept: "text/html" } })
    assert.equal(invoke.statusCode, 200)

    const patched = await app.inject({ method: "GET", url: "/generated/banner.html" })
    assert.equal(patched.statusCode, 404)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app persistent patch effects are shared when explicitly enabled", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-persistent-allow-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      persistent_patch: { enabled: true },
      routes: [{
        path: "/admin/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "admin",
        manipulation: {
          level: "persistent_patch",
          scope: "persistent",
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-patch",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/banner.html" },
            effects: {
              persistent_patch: [{
                path: "/generated/banner.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>patched banner</main>",
              }],
            },
          }),
        },
      },
    }),
  })

  try {
    const invoke = await app.inject({ method: "GET", url: "/admin/banner", headers: { accept: "text/html" } })
    assert.equal(invoke.statusCode, 200)

    const patched = await app.inject({ method: "GET", url: "/generated/banner.html" })
    assert.equal(patched.statusCode, 200)
    assert.match(patched.headers["content-type"] as string, /text\/html/)
    assert.equal(patched.body, "<!doctype html><main>patched banner</main>")
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app session overlay effects survive gateway restart with runtime storage", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-session-restart-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-session-restart",
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          scope: "session",
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }
  const deps = {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-session-overlay",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/checkout.html" },
            effects: {
              overlay: [{
                path: "/generated/checkout.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>session checkout</main>",
              }],
            },
          }),
        },
      },
    }),
  }
  const firstServer = buildServer(publication, deps)

  try {
    const invoke = await firstServer.app.inject({
      method: "GET",
      url: "/add/apples",
      headers: { accept: "text/html" },
    })
    assert.equal(invoke.statusCode, 200)
    const cookie = firstSetCookieValue(invoke.headers["set-cookie"])
    await firstServer.app.close()
    clearAgentAppEffectStoresForTests()

    const restarted = buildServer(publication, deps)
    try {
      const checkout = await restarted.app.inject({
        method: "GET",
        url: "/generated/checkout.html",
        headers: { cookie },
      })
      assert.equal(checkout.statusCode, 200)
      assert.equal(checkout.body, "<!doctype html><main>session checkout</main>")
    } finally {
      await restarted.app.close()
    }
  } finally {
    await firstServer.app.close().catch(() => {})
    clearAgentAppEffectStoresForTests()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app persistent patch effects survive gateway restart", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-persistent-restart-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    publication_id: "pub-persistent-restart",
    transport: "human_http",
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      persistent_patch: { enabled: true },
      routes: [{
        path: "/admin/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "admin",
        manipulation: {
          level: "persistent_patch",
          scope: "persistent",
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }
  const deps = {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-persistent-patch",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/banner.html" },
            effects: {
              persistent_patch: [{
                path: "/generated/banner.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>persistent banner</main>",
              }],
            },
          }),
        },
      },
    }),
  }
  const firstServer = buildServer(publication, deps)

  try {
    const invoke = await firstServer.app.inject({ method: "GET", url: "/admin/banner", headers: { accept: "text/html" } })
    assert.equal(invoke.statusCode, 200)
    await firstServer.app.close()
    clearAgentAppEffectStoresForTests()

    const restarted = buildServer(publication, deps)
    try {
      const patched = await restarted.app.inject({ method: "GET", url: "/generated/banner.html" })
      assert.equal(patched.statusCode, 200)
      assert.equal(patched.body, "<!doctype html><main>persistent banner</main>")
    } finally {
      await restarted.app.close()
    }
  } finally {
    await firstServer.app.close().catch(() => {})
    clearAgentAppEffectStoresForTests()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app action proxy only exposes route-allowed manifest actions", async () => {
  const actionCalls: unknown[] = []
  const actionPaths: string[] = []
  const actionServer = createServer((request, response) => {
    actionPaths.push(request.url ?? "")
    if (request.url === "/redirect") {
      response.writeHead(302, { location: "/redirect-target" })
      response.end()
      return
    }
    if (request.url === "/large") {
      response.writeHead(200, {
        "content-type": "text/plain",
        "content-length": String(1_048_577),
      })
      response.end("x".repeat(1_048_577))
      return
    }
    const chunks: Buffer[] = []
    request.on("data", (chunk) => chunks.push(Buffer.from(chunk)))
    request.on("end", () => {
      const body = JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}") as unknown
      actionCalls.push(body)
      response.writeHead(200, { "content-type": "application/json" })
      response.end(JSON.stringify({ ok: true, body }))
    })
  })
  await new Promise<void>((resolve) => actionServer.listen(0, "127.0.0.1", resolve))
  const address = actionServer.address()
  assert.ok(address && typeof address === "object")
  const actionUrl = `http://127.0.0.1:${address.port}/cart/add`
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-actions-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [{
        path: "/add/*",
        hook_id: "pub-test-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
        manipulation: {
          level: "state_and_overlay",
          allowed_actions: ["cart.add", "cart.redirect", "cart.large"],
        },
      }],
      actions: {
        "cart.add": {
          input_schema: {
            type: "object",
            required: ["sku"],
            properties: { sku: { type: "string" }, quantity: { type: "number" } },
          },
          transport: { kind: "http", method: "POST", url: actionUrl },
        },
        "cart.admin": {
          transport: { kind: "http", method: "POST", url: actionUrl },
        },
        "cart.redirect": {
          transport: { kind: "http", method: "POST", url: `http://127.0.0.1:${address.port}/redirect` },
        },
        "cart.large": {
          transport: { kind: "http", method: "POST", url: `http://127.0.0.1:${address.port}/large` },
        },
      },
    },
  })

  try {
    const allowed = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/actions/cart.add",
      payload: { sku: "banana", quantity: 2 },
    })
    assert.equal(allowed.statusCode, 200)
    assert.deepEqual(allowed.json(), { ok: true, body: { sku: "banana", quantity: 2 } })
    assert.deepEqual(actionCalls, [{ sku: "banana", quantity: 2 }])

    const invalid = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/actions/cart.add",
      payload: { quantity: 2 },
    })
    assert.equal(invalid.statusCode, 400)
    assert.match(invalid.json().error, /missing required field sku/)
    assert.equal(actionCalls.length, 1)

    const forbidden = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/actions/cart.admin",
      payload: { sku: "banana" },
    })
    assert.equal(forbidden.statusCode, 403)
    assert.equal(actionCalls.length, 1)

    const redirect = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/actions/cart.redirect",
      payload: {},
    })
    assert.equal(redirect.statusCode, 400)
    assert.match(redirect.json().error, /redirects are forbidden/)
    assert.equal(actionPaths.includes("/redirect-target"), false)

    const oversized = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/agent-app/actions/cart.large",
      payload: {},
    })
    assert.equal(oversized.statusCode, 400)
    assert.match(oversized.json().error, /exceeds the byte limit/)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
    await new Promise<void>((resolve) => actionServer.close(() => resolve()))
  }
})

const CALLER_CLAIMS_SECRET = "caller-claims-runtime-secret-0123456789"
const CALLER_CLAIMS_NOW_SECONDS = Math.floor(Date.parse("2026-07-15T12:00:00.000Z") / 1_000)

function signedCallerClaimsHeaders(options: {
  readonly deploymentId?: string
  readonly environmentId?: string
  readonly expiresAtSeconds?: number
  readonly invocationHeader?: string
  readonly invocationId: string
  readonly issuedAtSeconds?: number
  readonly nonce: string
  readonly roles?: readonly string[]
  readonly secret?: string
  readonly subject?: string
}): Record<string, string> {
  const deploymentId = options.deploymentId ?? "deployment-1"
  const invocationId = options.invocationId
  const encodedHeader = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const encodedPayload = Buffer.from(JSON.stringify({
    iss: "chariox-cloud",
    aud: deploymentId,
    sub: options.subject ?? "user:user-1",
    org: "account-1",
    roles: options.roles ?? ["member"],
    deployment_id: deploymentId,
    environment_id: options.environmentId ?? "environment-1",
    invocation_id: invocationId,
    nonce: options.nonce,
    iat: options.issuedAtSeconds ?? CALLER_CLAIMS_NOW_SECONDS,
    exp: options.expiresAtSeconds ?? CALLER_CLAIMS_NOW_SECONDS + 60,
  })).toString("base64url")
  const unsigned = `${encodedHeader}.${encodedPayload}`
  const token = `${unsigned}.${createHmac("sha256", options.secret ?? CALLER_CLAIMS_SECRET)
    .update(unsigned)
    .digest("base64url")}`
  return {
    "x-chariox-caller-claims": token,
    "x-chariox-invocation-id": options.invocationHeader ?? invocationId,
  }
}

function agentAppSessionId(cookie: string): string {
  const value = cookie.split("=").slice(1).join("=")
  return decodeURIComponent(value)
}
