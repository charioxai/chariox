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
  createWebSocketReader,
  enqueueAgentAppReplicaDispatch,
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
  publishedTransportConfig,
  readFile,
  registerCloudPublicationDeploymentBackend,
  releaseAgentAppReplicaInvocation,
  rememberAgentAppInvocationRoute,
  rm,
  setOptionalEnv,
  sseEventNames,
  test,
  tmpdir,
  visibleWorkflowRun,
  waitForCondition,
  WebSocket,
  writeFile,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"

test("agent app replica selection preserves caller affinity across hidden sessions", async () => {
  const selectedReplicas: unknown[] = []
  const selectedRequestIds: string[] = []
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-replicas-"))
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
    await app.inject({ method: "GET", url: "/add/apples", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-a" } })
    await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-b" } })
    const queued = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-a" } })
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-replica-state-"))
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-replica-queue-"))
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
    await app.inject({ method: "GET", url: "/add/apples", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-a" } })
    await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-b" } })
    const queued = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-c" } })

    assert.equal(queued.statusCode, 200)
    assert.match(queued.body, /agent_app_pool_queued/)
    assert.deepEqual(invocations.map((invocation) => invocation.replica), ["replica-session-1", "replica-session-2"])
    const saturatedStatus = await app.inject({ method: "GET", url: "/.well-known/arroba/agent-app/status" })
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
    assert.equal(invocations[2]?.caller, "caller-c")
    assert.equal(invocations[2]?.replica, "replica-session-1")
    const drainedStatus = await app.inject({ method: "GET", url: "/.well-known/arroba/agent-app/status" })
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

test("agent app overlay effects cannot write protected paths", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-protected-overlay-"))
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-persistent-reject-"))
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-persistent-allow-"))
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-session-restart-"))
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-persistent-restart-"))
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
  const actionServer = createServer((request, response) => {
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
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-actions-"))
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
          allowed_actions: ["cart.add"],
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
      },
    },
  })

  try {
    const allowed = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/agent-app/actions/cart.add",
      payload: { sku: "banana", quantity: 2 },
    })
    assert.equal(allowed.statusCode, 200)
    assert.deepEqual(allowed.json(), { ok: true, body: { sku: "banana", quantity: 2 } })
    assert.deepEqual(actionCalls, [{ sku: "banana", quantity: 2 }])

    const invalid = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/agent-app/actions/cart.add",
      payload: { quantity: 2 },
    })
    assert.equal(invalid.statusCode, 400)
    assert.match(invalid.json().error, /missing required field sku/)
    assert.equal(actionCalls.length, 1)

    const forbidden = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/agent-app/actions/cart.admin",
      payload: { sku: "banana" },
    })
    assert.equal(forbidden.statusCode, 403)
    assert.equal(actionCalls.length, 1)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
    await new Promise<void>((resolve) => actionServer.close(() => resolve()))
  }
})
