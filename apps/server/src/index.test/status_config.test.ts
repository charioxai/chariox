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

test("GET /health returns an ok status payload", async () => {
  const { app } = buildServer(baseConfig, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), {
      status: "ok",
      package: { materialized: true, package_root: null, missing_files: [] },
      provider_readiness: [],
    })
  } finally {
    await app.close()
  }
})

test("GET /health reports package materialization and provider readiness", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-health-"))
  await writeFile(join(root, "publication.json"), JSON.stringify({ schema_version: 1 }))
  await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
    schema_version: 1,
    source_session: { id: "source-session-1" },
    workflow: { id: "workflow-1", nodes: [] },
    endpoint: { id: "endpoint-1" },
    agents: [{ id: "agent-1", provider: "codex", model: "gpt-5.2" }],
  }))
  await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))
  const { app } = buildServer({
    ...baseConfig,
    package_root: root,
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
    getProviderReadiness: async () => [{
      provider: "codex",
      status: "provider_ready",
      ready: true,
      cli: { available: true, command: "codex", version: "codex 1.0.0" },
      auth: { status: "provider_ready", account_profile: "codex@example.test" },
    }],
  })

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), {
      status: "ok",
      package: { materialized: true, package_root: root, missing_files: [] },
      provider_readiness: [{
        provider: "codex",
        status: "provider_ready",
        ready: true,
        cli: { available: true, command: "codex", version: "codex 1.0.0" },
        auth: { status: "provider_ready", account_profile: "codex@example.test" },
      }],
    })
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("GET /health treats the explicitly enabled development provider stub as internally ready", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-dev-stub-health-"))
  const previousDevStub = process.env.ARROBA_PROVIDER_DEV_STUB
  await writeFile(join(root, "publication.json"), JSON.stringify({ schema_version: 1 }))
  await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
    schema_version: 1,
    source_session: { id: "source-session-1" },
    workflow: { id: "workflow-1", nodes: [] },
    endpoint: { id: "endpoint-1" },
    agents: [{ id: "agent-1", provider: "dev-stub", model: "default" }],
  }))
  await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))
  setOptionalEnv("ARROBA_PROVIDER_DEV_STUB", "1")
  const { app } = buildServer({
    ...baseConfig,
    package_root: root,
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json().provider_readiness, [{
      provider: "dev-stub",
      status: "provider_ready",
      ready: true,
      cli: { available: true, command: "internal:dev-stub", version: null },
      auth: { status: "provider_ready", account_profile: "development-stub" },
    }])
  } finally {
    await app.close()
    setOptionalEnv("ARROBA_PROVIDER_DEV_STUB", previousDevStub)
    await rm(root, { recursive: true, force: true })
  }
})

test("GET publication status reports runtime binding", async () => {
  const { app } = buildServer({
    ...baseConfig,
    source_session_id: "source-session-1",
    transport: "api_sse_json",
    methods: ["POST"],
    route: "/invoke",
    mode: "async",
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({
      method: "GET",
      url: "/.well-known/arroba/publication/status",
    })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), {
      status: "running",
      publication_id: "pub-test",
      runtime_session_id: "session-1",
      source_session_id: "source-session-1",
      workflow_ref: "workflow-1",
      endpoint_ref: "endpoint-1",
      hook_id: null,
      queue_ref: "default",
      transport: "api_sse_json",
      mode: "async",
      route: "/invoke",
      methods: ["POST"],
      package: { materialized: true, package_root: null, missing_files: [] },
      provider_readiness: [],
    })
  } finally {
    await app.close()
  }
})

test("GET publication status includes runtime watchdog and latest output details", async () => {
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
  }, {
    getPublicationStatusDetails: async () => ({
      runtime: { reachable: true },
      watchdog_count: 1,
      watchdogs: [{
        id: "watchdog-1",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        enabled: true,
        interval_seconds: 5,
        invocation_prompt: "scheduled prompt",
        policy: "queue",
        wakeups_executed: 1,
        next_run_at_ms: 1000,
        last_run_at_ms: 900,
        last_status: "started",
        last_error: null,
        last_workflow_run_id: "run-1",
        pending_run: false,
        created_at_ms: 0,
        updated_at_ms: 900,
      }],
      latest_run: {
        id: "run-1",
        status: "Completed",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        created_at_ms: 800,
        completed_at_ms: 950,
        publication_invocation: null,
        final_output: { message: "{\"value\":1842}" },
      },
      recent_runs: [{
        id: "run-1",
        status: "Completed",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        created_at_ms: 800,
        completed_at_ms: 950,
        publication_invocation: null,
        final_output: { message: "{\"value\":1842}" },
      }],
      latest_output: {
        kind: "final",
        message: "{\"value\":1842}",
        artifacts: [],
      },
    }),
  })

  try {
    const response = await app.inject({
      method: "GET",
      url: "/.well-known/arroba/publication/status",
    })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), {
      status: "running",
      publication_id: "pub-test",
      runtime_session_id: "session-1",
      source_session_id: null,
      workflow_ref: "workflow-1",
      endpoint_ref: "endpoint-1",
      hook_id: null,
      queue_ref: "default",
      transport: "human_http",
      mode: "sync",
      route: "/*",
      methods: ["GET", "POST"],
      package: { materialized: true, package_root: null, missing_files: [] },
      provider_readiness: [],
      runtime: { reachable: true },
      watchdog_count: 1,
      watchdogs: [{
        id: "watchdog-1",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        enabled: true,
        interval_seconds: 5,
        invocation_prompt: "scheduled prompt",
        policy: "queue",
        wakeups_executed: 1,
        next_run_at_ms: 1000,
        last_run_at_ms: 900,
        last_status: "started",
        last_error: null,
        last_workflow_run_id: "run-1",
        pending_run: false,
        created_at_ms: 0,
        updated_at_ms: 900,
      }],
      latest_run: {
        id: "run-1",
        status: "Completed",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        created_at_ms: 800,
        completed_at_ms: 950,
        publication_invocation: null,
        final_output: { message: "{\"value\":1842}" },
      },
      recent_runs: [{
        id: "run-1",
        status: "Completed",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        created_at_ms: 800,
        completed_at_ms: 950,
        publication_invocation: null,
        final_output: { message: "{\"value\":1842}" },
      }],
      latest_output: {
        kind: "final",
        message: "{\"value\":1842}",
        artifacts: [],
      },
    })
  } finally {
    await app.close()
  }
})

test("schedule-only publication exposes status without ingress routes", async () => {
  let invocations = 0
  const { app } = buildServer({
    publication_id: "pub-schedule-only",
    session_id: "session-1",
    workflow_ref: "workflow-1",
    endpoint_ref: "endpoint-1",
    queue_ref: "default",
    transport: "schedule_only",
  }, {
    invokeWorkflow: async () => {
      invocations += 1
      return { accepted: true, workflow_run: { id: "run-schedule", status: "Running" } }
    },
  })

  try {
    const status = await app.inject({
      method: "GET",
      url: "/.well-known/arroba/publication/status",
    })
    assert.equal(status.statusCode, 200)
    const statusPayload = status.json()
    assert.equal(statusPayload.transport, "schedule_only")
    assert.equal(statusPayload.route, undefined)
    assert.equal(statusPayload.methods, undefined)
    assert.equal(statusPayload.mode, undefined)

    const root = await app.inject({ method: "GET", url: "/" })
    assert.equal(root.statusCode, 404)
    const route = await app.inject({ method: "GET", url: "/prompt/hello" })
    assert.equal(route.statusCode, 404)
    const formInvoke = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/publication/human-http/invoke",
      payload: { input: { prompt: "hello" } },
    })
    assert.equal(formInvoke.statusCode, 404)
    assert.equal(invocations, 0)
  } finally {
    await app.close()
  }
})

test("gateway maps kernel-owned publication records to runtime config", async () => {
  const config = publicationConfigFromKernelRecord({
    id: "pub-1",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "public_qa",
    enabled: true,
    route: "/qa",
    methods: ["POST", "PUT"],
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<task>.+)$" },
    input_schema: { type: "object", required: ["task"] },
    mode: "async",
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
  }, "ws://kernel")

  assert.deepEqual(config, {
    publication_id: "pub-1",
    session_id: "session-1",
    workflow_ref: "workflow-1",
    endpoint_ref: "endpoint-1",
    queue_ref: "default",
    kernel_endpoint: "ws://kernel",
    route: "/qa",
    methods: ["POST"],
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<task>.+)$" },
    input_schema: { type: "object", required: ["task"] },
    mode: "async",
  })
})

test("gateway defaults publication runtime config by transport", async () => {
  const apiConfig = publicationConfigFromKernelRecord({
    id: "pub-api",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "api",
    enabled: true,
    transport: { kind: "api_sse_json" },
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
  }, "ws://kernel")
  assert.equal(apiConfig.route, "/invoke")
  assert.deepEqual(apiConfig.methods, ["POST"])
  assert.deepEqual(apiConfig.parser, { kind: "json" })
  assert.equal(apiConfig.mode, "async")

  const websocketConfig = publicationConfigFromKernelRecord({
    id: "pub-ws",
    session_id: "session-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    alias: "ws",
    enabled: true,
    transport: { kind: "websocket_json" },
    created_by_user_id: "local",
    created_at_ms: 0,
    updated_at_ms: 0,
  }, "ws://kernel")
  assert.equal(websocketConfig.route, "/.well-known/arroba/publication/ws")
  assert.equal(websocketConfig.methods, undefined)
  assert.equal(websocketConfig.parser, undefined)
  assert.equal(websocketConfig.mode, "async")
})

test("gateway can load publication config from kernel lookup", async () => {
  const requests: Record<string, unknown>[] = []
  const config = await loadPublicationConfigFromKernel("session-1", "pub-1", "ws://kernel", {
    send: async (request) => {
      requests.push(request)
      return {
        WorkflowPublication: {
          publication: {
            id: "pub-1",
            session_id: "session-1",
            workflow_id: "workflow-1",
            endpoint_id: "endpoint-1",
            enabled: true,
            route: "/qa",
            methods: ["GET"],
            parser: { kind: "json" },
            mode: "sync",
            created_by_user_id: "local",
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    },
  })

  assert.deepEqual(requests, [
    { GetWorkflowPublication: { session_id: "session-1", publication_ref: "pub-1" } },
    {
      AttachToSession: {
        session_id: "session-1",
        client_id: `arroba-publication-gateway-${process.pid}-pub-1-session-1`,
        capability_level: "FullTerminal",
      },
    },
  ])
  assert.equal(config.publication_id, "pub-1")
  assert.equal(config.workflow_ref, "workflow-1")
  assert.equal(config.endpoint_ref, "endpoint-1")
  assert.deepEqual(config.methods, ["GET"])
})

test("gateway maps exported publication packages to runtime config", async () => {
  const config = publicationConfigFromPackage({
    schema_version: 1,
    package_version: 1,
    publication_id: "pub-1",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    hooks: [{
      id: "hook-human",
      transport: "human_http",
      endpoint_id: "endpoint-1",
      route: "/*",
      methods: ["GET", "PATCH"],
      parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
      input_schema: { type: "object", required: ["prompt"] },
      mode: "async",
    }],
  }, {
    schema_version: 1,
    source_session: {
      id: "session-1",
      workspace_id: "/repo",
      worktree_id: "/repo",
    },
    workflow: {
      id: "workflow-1",
      alias: null,
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
      edges: [],
      endpoints: [{ id: "endpoint-1", alias: null, entry_node_id: "node-1" }],
    },
    endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
  }, "ws://kernel")

  assert.deepEqual(config, {
    publication_id: "pub-1",
    session_id: "session-1",
    source_session_id: "session-1",
    workflow_ref: "workflow-1",
    endpoint_ref: "endpoint-1",
    hook_id: "hook-human",
    queue_ref: "default",
    kernel_endpoint: "ws://kernel",
    transport: "human_http",
    route: "/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
    input_schema: { type: "object", required: ["prompt"] },
    mode: "async",
  })
})

test("gateway loads publication package directories", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-package-"))
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      default_bindings_path: "bindings.local.json",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
        parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
      }],
    }))
    await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
      schema_version: 1,
      source_session: {
        id: "session-1",
        workspace_id: "/repo",
        worktree_id: "/repo",
      },
      workflow: {
        id: "workflow-1",
        alias: null,
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
        edges: [],
        endpoints: [{ id: "endpoint-1", entry_node_id: "node-1" }],
      },
      endpoint: { id: "endpoint-1", entry_node_id: "node-1" },
      agents: [{
        id: "agent-1",
        agent_ref: "agent-ref-1",
        session_id: "session-1",
        alias: null,
        provider: "codex",
        model: null,
        worktree_id: "/repo",
        state: "Idle",
        is_processing: false,
        grid_row: 0,
        grid_col: 0,
        grid_row_span: 1,
        grid_col_span: 1,
        created_at_ms: 0,
        last_activity_at_ms: 0,
      }],
    }))

    const config = await loadPublicationPackageConfig(root, { kernelEndpoint: "ws://kernel" })

    assert.equal(config.publication_id, "pub-1")
    assert.equal(config.session_id, "session-1")
    assert.equal(config.workflow_ref, "workflow-1")
    assert.equal(config.endpoint_ref, "endpoint-1")
    assert.equal(config.kernel_endpoint, "ws://kernel")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway requires a valid deployment contract for package v3", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-contract-"))
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 3,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
      }],
    }))
    await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
      schema_version: 1,
      source_session: { id: "session-1", workspace_id: "/repo", worktree_id: "/repo" },
      workflow: {
        id: "workflow-1",
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
        edges: [],
        endpoints: [{ id: "endpoint-1", entry_node_id: "node-1" }],
      },
      endpoint: { id: "endpoint-1", entry_node_id: "node-1" },
      agents: [],
    }))

    await assert.rejects(
      loadPublicationPackageConfig(root, { kernelEndpoint: "ws://kernel" }),
      /deployment-contract\.json/,
    )
    const digest = `sha256:${"a".repeat(64)}`
    const deploymentContract = {
      schema_version: 1,
      package_id: digest,
      artifact: {
        content_digest: digest,
        digest_algorithm: "sha256",
        digest_scope: "package_files_excluding_deployment_contract",
      },
      source: {
        publication_id: "pub-1",
        session_id: "session-1",
        workflow_id: "workflow-1",
        endpoint_id: "endpoint-1",
        creator_user_id: "user-1",
        captured_at_ms: 1,
      },
      compatibility: {
        package_version: 3,
        minimum_kernel_version: "0.1.0",
        minimum_local_daemon_protocol_version: 240,
      },
      routes: [{ id: "hook-1" }],
      provider_requirements: [],
      credential_slots: [],
      configuration: [],
      capabilities: {},
      resources: {},
      presentation: {},
      signatures: [],
    }
    await writeFile(join(root, "deployment-contract.json"), JSON.stringify(deploymentContract))

    const config = await loadPublicationPackageConfig(root, { kernelEndpoint: "ws://kernel" })
    assert.equal(config.publication_id, "pub-1")

    deploymentContract.compatibility.minimum_local_daemon_protocol_version = 241
    await writeFile(join(root, "deployment-contract.json"), JSON.stringify(deploymentContract))
    await assert.rejects(
      loadPublicationPackageConfig(root, { kernelEndpoint: "ws://kernel" }),
      /requires local daemon protocol version 241, but target runtime supports 240/,
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
