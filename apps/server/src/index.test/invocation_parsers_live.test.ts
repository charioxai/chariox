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

test("human HTTP queued browser GET opens an invocation SSE subscription", async () => {
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      queued: true,
      response: { queued_prompt: { id: "queue-1" } },
    }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/queued", headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 200)
    assert.match(response.body, /EventSource/)
    assert.match(response.body, /\/\.well-known\/chariox\/publication\/invocations\//)
    assert.match(response.body, /id="queue-status"/)
    assert.match(response.body, /renderQueueStatus/)
  } finally {
    await app.close()
  }
})

test("workflow run correlation resolves queued publication invocations by request id", async () => {
  const client = {
    send: async () => ({
      WorkflowRunsListed: {
        workflow_runs: [{
          id: "run-1",
          status: "Completed",
          invocation_prompt: "ship",
          publication_invocation: {
            publication_id: "pub-test",
            invocation_id: "api_123",
            transport: "api_sse_json",
            endpoint_id: "endpoint-1",
            input: { prompt: "ship" },
            artifacts: [],
            mode: "async",
            caller: {},
          },
        }, {
          id: "run-2",
          status: "Completed",
          invocation_prompt: JSON.stringify({ request_id: "api_456" }),
        }],
      },
    }),
  }

  const workflowRun = await findWorkflowRunByInvocationRequestId(client, baseConfig, "api_123")

  assert.equal(workflowRun?.id, "run-1")
})

test("kernel publication client splits prompt from publication invocation envelope", () => {
  const invocation = {
    publication_id: "pub-test",
    request_id: "req-1",
    caller: { type: "anonymous" },
    input: { prompt: "describe this", artifacts: [{ id: "artifact-1" }] },
    mode: "sync" as const,
  }

  assert.equal(promptFromInvocationInput(invocation.input), "describe this")
  assert.deepEqual(publicationInvocationEnvelope({
    ...baseConfig,
    hook_id: "hook-1",
    queue_ref: "priority",
    transport: "human_http",
  }, invocation), {
    publication_id: "pub-test",
    hook_id: "hook-1",
    invocation_id: "req-1",
    transport: "human_http",
    endpoint_id: "endpoint-1",
    queue_ref: "priority",
    input: invocation.input,
    artifacts: [{ id: "artifact-1" }],
    mode: "sync",
    caller: { type: "anonymous" },
  })
})

test("human HTTP publication parsers inject prompt consistently", async () => {
  const prompt = "build a bright checkout page"
  const cases: Array<{
    readonly name: string
    readonly config: () => WorkflowPublicationConfig
    readonly request: {
      readonly method: "GET" | "POST"
      readonly url: string
      readonly headers?: Record<string, string>
      readonly payload?: Record<string, unknown>
    }
    readonly expectedInput: unknown
  }> = [{
    name: "path_template",
    config: () => publishedHttpConfig("path-template", "/prompt/:prompt", ["GET"], {
      kind: "path_template",
      template: "/prompt/:prompt",
    }),
    request: { method: "GET", url: `/prompt/${encodeURIComponent(prompt)}` },
    expectedInput: { prompt },
  }, {
    name: "json",
    config: () => publishedHttpConfig("json", "/prompt", ["POST"], { kind: "json" }),
    request: {
      method: "POST",
      url: "/prompt",
      headers: { "content-type": "application/json" },
      payload: { prompt, style: "minimal" },
    },
    expectedInput: { prompt, style: "minimal" },
  }, {
    name: "query_params",
    config: () => publishedHttpConfig("query-params", "/prompt", ["GET"], { kind: "query_params" }),
    request: { method: "GET", url: `/prompt?prompt=${encodeURIComponent(prompt)}&style=minimal` },
    expectedInput: { prompt, style: "minimal" },
  }, {
    name: "webhook",
    config: () => publishedHttpConfig("webhook", "/webhook", ["POST"], { kind: "webhook" }),
    request: {
      method: "POST",
      url: "/webhook?source=stripe",
      headers: {
        "content-type": "application/json",
        "stripe-signature": "test-signature",
      },
      payload: { prompt, event: "checkout.session.completed" },
    },
    expectedInput: {
      body: { prompt, event: "checkout.session.completed" },
      query: { source: "stripe" },
    },
  }]

  for (const candidate of cases) {
    const prompts: Array<string | null> = []
    const inputs: unknown[] = []
    const { app } = buildServer(candidate.config(), {
      invokeWorkflow: async (invocation) => {
        prompts.push(promptFromInvocationInput(invocation.input))
        inputs.push(invocation.input)
        return { accepted: true, workflow_run: { id: `run-${candidate.name}`, status: "Completed" } }
      },
    })

    try {
      const address = await app.listen({ host: "127.0.0.1", port: 0 })
      const fetchOptions: RequestInit = {
        method: candidate.request.method,
      }
      if (candidate.request.headers) fetchOptions.headers = candidate.request.headers
      if (candidate.request.payload !== undefined) fetchOptions.body = JSON.stringify(candidate.request.payload)
      const response = await fetch(`${address}${candidate.request.url}`, fetchOptions)
      assert.equal(response.status, 200, candidate.name)
      assert.equal(prompts[0], prompt, candidate.name)
      if (candidate.name === "webhook") {
        assert.deepEqual((inputs[0] as Record<string, unknown>).body, (candidate.expectedInput as Record<string, unknown>).body)
        assert.deepEqual((inputs[0] as Record<string, unknown>).query, (candidate.expectedInput as Record<string, unknown>).query)
        assert.equal((inputs[0] as { readonly headers?: Record<string, unknown> }).headers?.["stripe-signature"], "test-signature")
      } else {
        assert.deepEqual(inputs[0], candidate.expectedInput, candidate.name)
      }
    } finally {
      await app.close()
    }
  }
})

test("published transports invoke at their root defaults without viewer route collisions", async () => {
  const prompt = "ship the transport surface"

  {
    let invocations = 0
    const { app } = buildServer(publishedTransportConfig({
      id: "human-http",
      transport: "human_http",
      methods: ["GET", "POST"],
      parser: { kind: "query_params" },
    }), {
      invokeWorkflow: async (invocation) => {
        invocations += 1
        assert.deepEqual(invocation.input, { prompt })
        return { accepted: true, workflow_run: { id: "run-human", status: "Completed", final_output: { message: "human done" } } }
      },
    })
    try {
      const address = await app.listen({ host: "127.0.0.1", port: 0 })
      const viewer = await fetch(address, { headers: { accept: "text/html" } })
      assert.equal(viewer.status, 200)
      assert.match(viewer.headers.get("content-type") ?? "", /text\/html/)
      assert.match(await viewer.text(), /Published workflow/)
      assert.equal(invocations, 0)

      const invoked = await fetch(`${address}/?prompt=${encodeURIComponent(prompt)}`, { headers: { accept: "text/html" } })
      assert.equal(invoked.status, 200)
      assert.match(invoked.headers.get("content-type") ?? "", /text\/html/)
      assert.match(await invoked.text(), /human done/)
      assert.equal(invocations, 1)
    } finally {
      await app.close()
    }
  }

  {
    let seenInput: unknown = null
    let seenCaller: unknown = null
    let seenMode: unknown = null
    const { app } = buildServer(publishedTransportConfig({
      id: "api-sse",
      transport: "api_sse_json",
      methods: ["POST"],
      inputSchema: { type: "object", required: ["prompt"], properties: { prompt: { type: "string" } } },
      mode: "async",
    }), {
      invokeWorkflow: async (invocation) => {
        seenInput = invocation.input
        seenCaller = invocation.caller
        seenMode = invocation.mode
        return {
          accepted: true,
          workflow_run: {
            id: "run-api-sse",
            status: "Completed",
            final_output: { message: "api done" },
          },
        }
      },
    })
    try {
      const address = await app.listen({ host: "127.0.0.1", port: 0 })
      const response = await fetch(address, {
        method: "POST",
        headers: { accept: "text/event-stream", "content-type": "application/json" },
        body: JSON.stringify({ prompt, format: "html" }),
      })
      assert.equal(response.status, 200)
      assert.match(response.headers.get("content-type") ?? "", /text\/event-stream/)
      const body = await response.text()
      assert.deepEqual(sseEventNames(body), ["queued", "started", "final"])
      assert.match(body, /"workflow_run_id":"run-api-sse"/)
      assert.match(body, /"message":"api done"/)
      assert.deepEqual(seenInput, { prompt, format: "html" })
      assert.deepEqual(seenCaller, { type: "anonymous", proof: { transport: "api_sse_json" } })
      assert.equal(seenMode, "async")
    } finally {
      await app.close()
    }
  }

  {
    const inputs: unknown[] = []
    const callers: unknown[] = []
    const modes: unknown[] = []
    const { app } = buildServer(publishedTransportConfig({
      id: "websocket",
      transport: "websocket_json",
      inputSchema: { type: "object", required: ["prompt"], properties: { prompt: { type: "string" } } },
      mode: "async",
    }), {
      invokeWorkflow: async (invocation) => {
        inputs.push(invocation.input)
        callers.push(invocation.caller)
        modes.push(invocation.mode)
        return {
          accepted: true,
          workflow_run: {
            id: "run-websocket",
            status: "Completed",
            final_output: { message: "ws done" },
          },
        }
      },
    })
    try {
      await app.listen({ host: "127.0.0.1", port: 0 })
      const address = app.server.address()
      const port = typeof address === "object" && address ? address.port : 0
      const socket = new WebSocket(`ws://127.0.0.1:${port}/`)
      const reader = createWebSocketReader(socket)
      try {
        assert.deepEqual(await reader.read(), { type: "ready", publication_id: "pub-websocket" })
        socket.send(JSON.stringify({ type: "invoke", input: { prompt, format: "html" } }))
        const accepted = await reader.read() as { type?: string; workflow_run?: { id?: string } }
        assert.equal(accepted.type, "accepted")
        assert.equal(accepted.workflow_run?.id, "run-websocket")
        const queued = await reader.read() as { type?: string; invocation_id?: string }
        assert.equal(queued.type, "queued")
        assert.match(queued.invocation_id ?? "", /^ws_/)
        const started = await reader.read() as { type?: string; workflow_run_id?: string }
        assert.equal(started.type, "started")
        assert.equal(started.workflow_run_id, "run-websocket")
        const final = await reader.read() as { type?: string; workflow_run?: { final_output?: { message?: string } } }
        assert.equal(final.type, "final")
        assert.equal(final.workflow_run?.final_output?.message, "ws done")
        assert.deepEqual(inputs, [{ prompt, format: "html" }])
        assert.deepEqual(callers, [{ type: "anonymous" }])
        assert.deepEqual(modes, ["async"])
      } finally {
        socket.close()
      }
    } finally {
      await app.close()
    }
  }

  {
    let seenInput: unknown = null
    let seenCaller: unknown = null
    let seenMode: unknown = null
    const { app } = buildServer(publishedTransportConfig({
      id: "mcp",
      transport: "mcp",
      methods: ["POST"],
      inputSchema: { type: "object", required: ["prompt"], properties: { prompt: { type: "string" } } },
      mode: "sync",
    }), {
      invokeWorkflow: async (invocation) => {
        seenInput = invocation.input
        seenCaller = invocation.caller
        seenMode = invocation.mode
        return {
          accepted: true,
          workflow_run: {
            id: "run-mcp-live",
            status: "Completed",
            final_output: { message: "mcp done" },
          },
        }
      },
    })
    try {
      const address = await app.listen({ host: "127.0.0.1", port: 0 })
      const initialize = await fetch(address, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26" } }),
      })
      assert.equal(initialize.status, 200)
      assert.equal((await initialize.json() as { result?: { serverInfo?: { name?: string } } }).result?.serverInfo?.name, "chariox-publication")

      const called = await fetch(address, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          jsonrpc: "2.0",
          id: 2,
          method: "tools/call",
          params: { name: "invoke_pub_mcp", arguments: { prompt, format: "html" } },
        }),
      })
      assert.equal(called.status, 200)
      const body = await called.json() as {
        readonly result?: {
          readonly content?: unknown
          readonly structuredContent?: { readonly workflow_run_id?: string; readonly message?: unknown }
          readonly isError?: boolean
        }
      }
      assert.deepEqual(body.result?.content, [{ type: "text", text: "mcp done" }])
      assert.equal(body.result?.structuredContent?.workflow_run_id, "run-mcp-live")
      assert.equal(body.result?.isError, false)
      assert.deepEqual(seenInput, { prompt, format: "html" })
      assert.deepEqual(seenCaller, { type: "anonymous", proof: { transport: "mcp", tool_name: "invoke_pub_mcp" } })
      assert.equal(seenMode, "sync")
    } finally {
      await app.close()
    }
  }
})
