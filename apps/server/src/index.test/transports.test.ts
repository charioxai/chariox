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
import { publicationViewerPage } from "../publication-viewer.js"

test("publication viewer preserves canonical and legacy Cloud ingress prefixes", () => {
  const html = publicationViewerPage({
    ...baseConfig,
    transport: "human_http",
    route: "/final/*",
    methods: ["GET"],
  })
  assert.match(html, /<link rel="icon" href="data:,">/)
  const functionSource = html.match(/function publicationIngressPrefix\(\) \{[\s\S]*?\n\}/)?.[0]
  assert.ok(functionSource)
  const resolvePrefix = new Function(
    "window",
    "viewerConfig",
    `${functionSource}; return publicationIngressPrefix();`,
  ) as (window: { location: { pathname: string } }, viewerConfig: unknown) => string
  const viewerConfig = { humanPromptTarget: { prefix: "/final/" }, transport: "human_http" }

  assert.equal(
    resolvePrefix({ location: { pathname: "/publication-ingress/~d/deployment-1/demo/final/hello" } }, viewerConfig),
    "/publication-ingress/~d/deployment-1/demo",
  )
  assert.equal(
    resolvePrefix({ location: { pathname: "/~d/deployment-1/demo/final/hello" } }, viewerConfig),
    "/~d/deployment-1/demo",
  )
  assert.equal(
    resolvePrefix({ location: { pathname: "/publication-ingress/demo/final/hello" } }, viewerConfig),
    "/publication-ingress/demo",
  )
  assert.equal(resolvePrefix({ location: { pathname: "/final/hello" } }, viewerConfig), "")
  assert.match(html, /window\.location\.href = publicationUrl\(viewerConfig\.humanPromptTarget\.prefix/)

  const agentAppHtml = publicationViewerPage({
    ...baseConfig,
    transport: "human_http",
    route: "/prompt/*",
    methods: ["GET"],
    agent_app: {
      enabled: true,
      routes: [{
        path: "/agent/*",
        hook_id: "agent-app-hook",
        prompt_source: "path_tail",
        response: "streaming_shell",
        required_role: "public",
      }],
    },
  })
  const serializedConfig = agentAppHtml.match(/window\.__arrobaPublicationViewerConfig = ([^\n]+);/)?.[1]
  assert.ok(serializedConfig)
  const agentAppViewerConfig = JSON.parse(serializedConfig)
  assert.deepEqual(agentAppViewerConfig.directRouteRoots, ["prompt", "agent"])
  assert.equal(
    resolvePrefix({ location: { pathname: "/agent/demo" } }, agentAppViewerConfig),
    "",
  )
})

test("gateway parses JSON and forwards transport-shaped workflow output", async () => {
  let seenInput: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    input_schema: {
      type: "object",
      required: ["name"],
      properties: { name: { type: "string" } },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      return {
        accepted: true,
        workflow_run: {
          id: "run-1",
          status: "Completed",
          final_output: {
            message: JSON.stringify({
              kind: "http_response",
              status: 201,
              headers: { "content-type": "text/plain" },
              body: `hello ${(invocation.input as { name: string }).name}`,
            }),
          },
        },
      }
    },
  })

  try {
    const accepted = await app.inject({
      method: "POST",
      url: "/anything",
      payload: { name: "miguel" },
    })
    assert.equal(accepted.statusCode, 201)
    assert.equal(accepted.headers["content-type"], "text/plain")
    assert.equal(accepted.body, "hello miguel")
    assert.deepEqual(seenInput, { name: "miguel" })
  } finally {
    await app.close()
  }
})

test("api_sse_json streams queued, started, partial, and final events", async () => {
  let seenInput: unknown = null
  let seenCaller: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    transport: "api_sse_json",
    route: "/custom/invoke",
    methods: ["POST"],
    input_schema: {
      type: "object",
      required: ["prompt"],
      properties: { prompt: { type: "string" } },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      seenCaller = invocation.caller
      return {
        accepted: true,
        workflow_run: {
          id: "run-api",
          status: "Completed",
          intermediate_outputs: [{
            id: "partial-1",
            output: { message: "working" },
            valid: true,
          }],
          final_output: {
            message: "done",
            artifacts: [{ name: "result.txt", url: "artifact://result" }],
          },
        },
      }
    },
  })

  try {
    const response = await app.inject({
      method: "POST",
      url: "/custom/invoke",
      headers: { accept: "text/event-stream" },
      payload: {
        prompt: "ship",
        artifacts: [{
          name: "input.txt",
          type: "text/plain",
          base64: "aGVsbG8=",
        }],
      },
    })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/event-stream/)
    assert.deepEqual(sseEventNames(response.body), ["queued", "started", "partial", "final"])
    assert.match(response.body, /"workflow_run_id":"run-api"/)
    assert.match(response.body, /"message":"working"/)
    assert.match(response.body, /"message":"done"/)
    assert.deepEqual(seenInput, {
      prompt: "ship",
      artifacts: [{
        name: "input.txt",
        type: "text/plain",
        base64: "aGVsbG8=",
      }],
    })
    assert.deepEqual(seenCaller, { type: "anonymous", proof: { transport: "api_sse_json" } })

    const defaultRoute = await app.inject({ method: "POST", url: "/invoke", payload: { prompt: "nope" } })
    assert.equal(defaultRoute.statusCode, 404)
  } finally {
    await app.close()
  }
})

test("api_sse_json defaults to /invoke when no route is configured", async () => {
  const { route: _route, ...configWithoutRoute } = baseConfig
  const { app } = buildServer({
    ...configWithoutRoute,
    transport: "api_sse_json",
    methods: ["POST"],
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-api-default",
        status: "Completed",
        final_output: { message: "done" },
      },
    }),
  })

  try {
    const response = await app.inject({
      method: "POST",
      url: "/invoke",
      headers: { accept: "text/event-stream" },
      payload: { prompt: "ship" },
    })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(sseEventNames(response.body), ["queued", "started", "final"])
  } finally {
    await app.close()
  }
})

test("api_sse_json accepts browser preflight for publication invocation", async () => {
  const { app } = buildServer({
    ...baseConfig,
    transport: "api_sse_json",
    route: "/invoke",
    methods: ["POST"],
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({
      method: "OPTIONS",
      url: "/invoke",
      headers: {
        origin: "https://cloud.example.test",
        "access-control-request-method": "POST",
        "access-control-request-headers": "content-type",
      },
    })

    assert.equal(response.statusCode, 204)
    assert.equal(response.headers["access-control-allow-origin"], "*")
    assert.equal(response.headers["access-control-allow-methods"], "POST, OPTIONS")
    assert.equal(response.headers["access-control-allow-headers"], "content-type, accept")
  } finally {
    await app.close()
  }
})

test("mcp exposes a published workflow as a tool and returns final output", async () => {
  let seenInput: unknown = null
  let seenCaller: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-mcp",
    transport: "mcp",
    route: "/integrations/mcp",
    methods: ["POST"],
    input_schema: {
      type: "object",
      required: ["prompt"],
      properties: { prompt: { type: "string" } },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      seenCaller = invocation.caller
      return {
        accepted: true,
        workflow_run: {
          id: "run-mcp",
          status: "Completed",
          final_output: {
            message: "mcp done",
            artifacts: [{ name: "artifact.txt", url: "artifact://mcp" }],
          },
        },
      }
    },
  })

  try {
    const initialize = await app.inject({
      method: "POST",
      url: "/integrations/mcp",
      payload: { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26" } },
    })
    assert.equal(initialize.statusCode, 200)
    assert.equal(initialize.json().result.serverInfo.name, "arroba-publication")

    const tools = await app.inject({
      method: "POST",
      url: "/integrations/mcp",
      payload: { jsonrpc: "2.0", id: 2, method: "tools/list" },
    })
    assert.equal(tools.statusCode, 200)
    assert.equal(tools.json().result.tools[0].name, "invoke_pub_mcp")

    const called = await app.inject({
      method: "POST",
      url: "/integrations/mcp",
      payload: {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "invoke_pub_mcp", arguments: { prompt: "ship" } },
      },
    })
    assert.equal(called.statusCode, 200)
    assert.deepEqual(called.json().result.content, [{ type: "text", text: "mcp done" }])
    assert.equal(called.json().result.structuredContent.workflow_run_id, "run-mcp")
    assert.deepEqual(called.json().result.structuredContent.artifacts, [{ name: "artifact.txt", url: "artifact://mcp" }])
    assert.equal(called.json().result.isError, false)
    assert.deepEqual(seenInput, { prompt: "ship" })
    assert.deepEqual(seenCaller, { type: "anonymous", proof: { transport: "mcp", tool_name: "invoke_pub_mcp" } })

    const genericRoute = await app.inject({ method: "POST", url: "/not-mcp", payload: { prompt: "nope" } })
    assert.equal(genericRoute.statusCode, 404)
  } finally {
    await app.close()
  }
})

test("mcp tools/call streams JSON whitespace keepalives before slow final output", async () => {
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-mcp",
    transport: "mcp",
    mcp_keepalive_ms: 5,
  } as WorkflowPublicationConfig, {
    invokeWorkflow: async () => {
      await new Promise((resolve) => setTimeout(resolve, 25))
      return {
        accepted: true,
        workflow_run: {
          id: "run-mcp",
          status: "Completed",
          final_output: { message: "mcp done" },
        },
      }
    },
  })

  try {
    const called = await app.inject({
      method: "POST",
      url: "/mcp",
      payload: {
        jsonrpc: "2.0",
        id: 3,
        method: "tools/call",
        params: { name: "invoke_pub_mcp", arguments: { prompt: "ship" } },
      },
    })

    assert.equal(called.statusCode, 200)
    assert.match(called.body, /^\s+\{/)
    assert.deepEqual(JSON.parse(called.body).result.content, [{ type: "text", text: "mcp done" }])
  } finally {
    await app.close()
  }
})

test("mcp accepts browser preflight for JSON-RPC tool calls", async () => {
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-mcp",
    transport: "mcp",
    route: "/integrations/mcp",
    methods: ["POST"],
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({
      method: "OPTIONS",
      url: "/integrations/mcp",
      headers: {
        origin: "https://cloud.example.test",
        "access-control-request-method": "POST",
        "access-control-request-headers": "content-type",
      },
    })

    assert.equal(response.statusCode, 204)
    assert.equal(response.headers["access-control-allow-origin"], "*")
    assert.equal(response.headers["access-control-allow-methods"], "POST, OPTIONS")
    assert.equal(response.headers["access-control-allow-headers"], "content-type, accept")
  } finally {
    await app.close()
  }
})

test("mcp rejects invalid tool input", async () => {
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-mcp",
    transport: "mcp",
    input_schema: {
      type: "object",
      required: ["prompt"],
      properties: { prompt: { type: "string" } },
    },
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({
      method: "POST",
      url: "/mcp",
      payload: {
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name: "invoke_pub_mcp", arguments: { prompt: 7 } },
      },
    })
    assert.equal(response.statusCode, 200)
    assert.equal(response.json().error.code, -32000)
    assert.match(response.json().error.message, /field prompt expected string/)
  } finally {
    await app.close()
  }
})

test("human HTTP root returns a browser invocation form", async () => {
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/qa/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<prompt>.+)$" },
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/", headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/html/)
    assert.match(response.body, /invoke-form/)
    assert.match(response.body, /type="file" name="artifact" multiple/)
    assert.match(response.body, /\/qa\//)
  } finally {
    await app.close()
  }
})

test("browser viewer shell is shared across human HTTP, API SSE, and WebSocket transports", async () => {
  const cases: Array<{
    transport: "human_http" | "api_sse_json" | "websocket_json"
    methods: Array<"GET" | "POST">
    route: string
    adapterMarker: RegExp
  }> = [
    { transport: "human_http", methods: ["GET"], route: "/qa/*", adapterMarker: /invokeHumanHttp/ },
    { transport: "api_sse_json", methods: ["POST"], route: "/custom/api", adapterMarker: /invokeApiSse/ },
    { transport: "websocket_json", methods: ["GET"], route: "/custom/ws", adapterMarker: /invokeWebSocket/ },
  ]

  for (const item of cases) {
    const { app } = buildServer({
      ...baseConfig,
      transport: item.transport,
      route: item.route,
      methods: item.methods,
      parser: { kind: "json" },
    }, {
      invokeWorkflow: async () => ({ accepted: true }),
    })

    try {
      const response = await app.inject({ method: "GET", url: "/", headers: { accept: "text/html" } })
      assert.equal(response.statusCode, 200)
      assert.match(response.headers["content-type"] as string, /text\/html/)
      assert.match(response.body, /split-viewer/)
      assert.match(response.body, /invoke-form/)
      assert.match(response.body, /type="file" name="artifact" multiple/)
      assert.match(response.body, new RegExp(`"transport":"${item.transport}"`))
      assert.match(response.body, item.adapterMarker)
      if (item.transport === "api_sse_json") {
        assert.match(response.body, /"apiSseInvokePath":"\/custom\/api"/)
      }
    } finally {
      await app.close()
    }
  }
})

test("human HTTP root form can submit prompt and uploaded artifacts", async () => {
  let seenInput: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      return {
        accepted: true,
        workflow_run: { id: "run-upload", status: "Running" },
      }
    },
  })

  try {
    const response = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/publication/human-http/invoke",
      headers: { accept: "text/html" },
      payload: {
        prompt: "read image",
        artifacts: [{
          name: "image.png",
          type: "image/png",
          size_bytes: 11,
          data_url: "data:image/png;base64,aGVsbG8=",
        }],
      },
    })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/html/)
    assert.match(response.body, /EventSource/)
    assert.match(response.body, /events\.addEventListener\('partial'/)
    assert.deepEqual(seenInput, {
      prompt: "read image",
      artifacts: [{
        name: "image.png",
        type: "image/png",
        size_bytes: 11,
        data_url: "data:image/png;base64,aGVsbG8=",
      }],
    })

    const empty = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/publication/human-http/invoke",
      payload: { prompt: "", artifacts: [] },
    })
    assert.equal(empty.statusCode, 400)
    assert.match(empty.json().error, /prompt or artifact is required/)
  } finally {
    await app.close()
  }
})

test("human HTTP browser GET returns an HTML status page with SSE subscription", async () => {
  let seenInput: unknown = null
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      return {
        accepted: true,
        workflow_run: { id: "run-1", status: "Running" },
      }
    },
  })

  try {
    const response = await app.inject({ method: "GET", url: "/make%20tea", headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/html/)
    assert.match(response.body, /EventSource/)
    assert.match(response.body, /events\.addEventListener\('partial'/)
    assert.match(response.body, /subscribeHumanHttpEvents\(viewerConfig\.eventsUrl\)/)
    assert.match(response.body, /\/display\\\/\[\^\/\]\+/)
    assert.match(response.body, /parts\[0\] === 'publication-ingress'/)
    assert.match(response.body, /run-1/)
    assert.deepEqual(seenInput, { prompt: "make tea" })
  } finally {
    await app.close()
  }
})

test("human HTTP status page renders split trace viewer and sandboxed HTML output support", async () => {
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/*",
    methods: ["GET"],
    parser: { kind: "regex", source: "path", pattern: "^/(?<prompt>.+)$" },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-html",
        status: "Completed",
        final_output: {
          message: JSON.stringify({
            kind: "html",
            html: "<!doctype html><html><body><main>dashboard</main></body></html>",
          }),
        },
      },
    }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/dashboard", headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 200)
    assert.match(response.body, /class="split-viewer"/)
    assert.match(response.body, /id="trace-feed"/)
    assert.match(response.body, /events\.addEventListener\('trace'/)
    assert.match(response.body, /frame\.setAttribute\('sandbox', 'allow-scripts allow-forms allow-popups allow-modals'\)/)
    assert.match(response.body, /frame\.srcdoc = renderable\.html/)
    assert.match(response.body, /frame\.src = publicationAppAssetUrl\(renderable\.src\)/)
    assert.match(response.body, /parsed\.kind === 'response'/)
  } finally {
    await app.close()
  }
})
