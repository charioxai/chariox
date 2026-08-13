import { createHmac } from "node:crypto"

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
import {
  configurePublicationCallerClaimsRuntimeForTests,
} from "../publication-caller-claims.js"
import {
  publicationViewerPage,
  publicationViewerResultPage,
  viewerComposerEnabled,
  viewerTraceNodes,
} from "../publication-viewer.js"

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
  assert.equal(
    resolvePrefix({ location: { pathname: "/.well-known/chariox/publication/viewer/invocations/request-1" } }, viewerConfig),
    "",
  )
  assert.match(html, /"showComposer":false/)

  const namedRouteHtml = publicationViewerPage({
    ...baseConfig,
    transport: "human_http",
    route: "/viewer/:prompt/result",
    methods: ["GET"],
  })
  const namedRouteConfig = JSON.parse(
    namedRouteHtml.match(/window\.__charioxPublicationViewerConfig = ([^\n]+);/)?.[1] ?? "{}",
  )
  assert.deepEqual(namedRouteConfig.humanPromptTarget, {
    prefix: "/viewer/",
    suffix: "/result",
  })

  const invocationHtml = publicationViewerPage({
    ...baseConfig,
    transport: "human_http",
  }, {
    result: { accepted: true, queued: true },
    invocationRequestId: "request-1",
  })
  const invocationConfig = JSON.parse(
    invocationHtml.match(/window\.__charioxPublicationViewerConfig = ([^\n]+);/)?.[1] ?? "{}",
  )
  assert.equal(
    invocationConfig.permalink,
    "/.well-known/chariox/publication/viewer/invocations/request-1",
  )
  assert.match(invocationHtml, /window\.history\.replaceState/)

  const directGetHtml = publicationViewerResultPage({
    ...baseConfig,
    transport: "human_http",
    route: "/final/*",
    methods: ["GET"],
  }, { accepted: true, queued: true }, "request-2", true, { prompt: "Visible immediately" })
  const directGetConfig = JSON.parse(
    directGetHtml.match(/window\.__charioxPublicationViewerConfig = ([^\n]+);/)?.[1] ?? "{}",
  )
  assert.equal(directGetConfig.permalink, null)
  assert.equal(directGetConfig.optimisticPrompt, "Visible immediately")
  assert.equal(
    directGetHtml.indexOf("renderOptimisticPrompt(viewerConfig.optimisticPrompt);")
      < directGetHtml.indexOf("for (const trace of viewerConfig.initialTraces || []) renderTrace(trace);"),
    true,
  )

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
  const serializedConfig = agentAppHtml.match(/window\.__charioxPublicationViewerConfig = ([^\n]+);/)?.[1]
  assert.ok(serializedConfig)
  const agentAppViewerConfig = JSON.parse(serializedConfig)
  assert.deepEqual(agentAppViewerConfig.directRouteRoots, ["prompt", "agent"])
  assert.equal(
    resolvePrefix({ location: { pathname: "/agent/demo" } }, agentAppViewerConfig),
    "",
  )
})

test("publication viewer derives composer capability and one pane per exposed node", () => {
  assert.equal(viewerComposerEnabled({ ...baseConfig, transport: "human_http", methods: ["GET"] }), false)
  assert.equal(viewerComposerEnabled({ ...baseConfig, transport: "human_http", methods: ["POST"] }), true)
  assert.equal(viewerComposerEnabled({ ...baseConfig, transport: "api_sse_json", methods: ["POST"] }), true)
  assert.equal(viewerComposerEnabled({ ...baseConfig, transport: "websocket_json" }), true)
  assert.equal(viewerComposerEnabled({ ...baseConfig, transport: "mcp", methods: ["POST"] }), false)

  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    transport: "api_sse_json",
    trace_exposure: {
      nodes: {
        "node-a": ["thinking", "tool_use"],
        "node-b": ["assistant_messages"],
      },
    },
    trace_context: {
      nodes: {
        "node-a": { node_id: "node-a", node_label: "Research", agent_id: "agent-1", agent_alias: "Scout" },
        "node-b": { node_id: "node-b", node_label: "Compose", agent_id: "agent-2", agent_alias: "Writer" },
      },
    },
  }
  assert.deepEqual(viewerTraceNodes(publication), [
    { nodeId: "node-a", nodeLabel: "Research", agentAlias: "Scout", levels: ["thinking", "tool_use"] },
    { nodeId: "node-b", nodeLabel: "Compose", agentAlias: "Writer", levels: ["assistant_messages"] },
  ])

  const html = publicationViewerPage(publication)
  assert.match(html, /class="publication-viewer has-traces has-composer"/)
  assert.match(html, /new URLSearchParams\(window\.location\.search\)\.get\('chariox_embed'\) === 'output'/)
  assert.match(html, /rootEl\?\.classList\.add\('is-output-only'\)/)
  assert.match(html, /\.publication-viewer\.is-output-only \.trace-rail/)
  assert.match(html, /event\.data\?\.type !== 'chariox:publication:invoke'/)
  assert.match(html, /chariox:publication:snapshot/)
  assert.doesNotMatch(html, /initialWorkflowRun/)
  assert.match(html, /void invokePublication\(prompt, artifacts\)/)
  assert.match(html, /type: 'chariox:publication:settled'/)
  assert.match(html, /publicationId: viewerConfig\.publicationId/)
  assert.match(html, /workflowRun,/)
  assert.match(html, /id="rail-resizer"/)
  assert.match(html, /data-trace-node="node-a"/)
  assert.match(html, /data-trace-node="node-b"/)
  assert.match(html, /<footer>Scout<\/footer>/)
  assert.match(html, /class="invoke-form composer-under-traces"/)
  assert.match(html, /ResizeObserver/)
  assert.match(html, /traceKeys\.has\(key\)/)
  assert.match(html, /trace\.level, trace\.timestamp_ms, trace\.message/)
  assert.doesNotMatch(html, /trace\.level, trace\.sequence, trace\.timestamp_ms/)
  assert.match(html, /resetForInvocation\(prompt\)/)
  assert.match(html, /renderOptimisticPrompt\(prompt\)/)
  assert.match(html, /if \(outputOnlyEmbed\) rootUrl\.searchParams\.set\('chariox_embed', 'output'\)/)
  assert.match(html, /reorderTraceFeed\(feed\)/)
  assert.match(html, /leftLevel === 'output_summary'/)
  assert.match(html, /\['user_prompt', nodeId, workflowRunId, trace\.message\]/)
  assert.match(html, /item\.dataset\.traceKey === pendingKey/)
  assert.match(html, /pendingItem\.dataset\.traceRun = String\(trace\.workflow_node_run_id \|\| workflowRunId\)/)
  assert.match(html, /window\.addEventListener\('pointermove', move\)/)
  assert.match(html, /window\.addEventListener\('pointerup', done\)/)
  assert.match(html, /startWidth \+ startX - moveEvent\.clientX/)
  assert.match(html, /is-resizing-rail/)
  assert.match(html, /\.trace-agent-pane:only-child \{ grid-column: 1 \/ -1; \}/)
})

test("publication viewer replaces progress output and hydrates only the latest update", () => {
  const html = publicationViewerPage({
    ...baseConfig,
    transport: "api_sse_json",
  }, {
    result: {
      accepted: true,
      workflow_run: {
        id: "run-progress",
        status: "Running",
        intermediate_outputs: [
          { id: "first", output: { message: "first update" }, valid: true },
          { id: "latest", output: { message: "latest update" }, valid: true },
        ],
      },
    },
  })

  assert.match(html, /run\.intermediate_outputs\.at\(-1\)/)
  assert.match(html, /htmlOutputEl\.replaceChildren\(\)/)
  assert.match(html, /outputEl\.textContent = ''/)
  assert.match(html, /function normalizeViewerMessage\(message\)/)
  assert.match(html, /parsed\.output && typeof parsed\.output === 'object'/)
  assert.match(html, /const finalMessage = outputMessage\(payload\.workflow_run\?\.final_output\)/)
  assert.match(html, /else if \(payload\.message !== undefined\) renderOutput\(payload\.message, 'final'\)/)
  assert.match(html, /status\.latest_run && !isTerminalStatus\(status\.latest_run\.status\)/)
  assert.match(html, /setTimeout\(\(\) => void hydrateLatestRun\(\), 1_000\)/)
  assert.doesNotMatch(html, /partialOutputs\.push/)
  assert.doesNotMatch(html, /partialOutputs\.join/)
  assert.match(html, /allow-downloads/)
  assert.doesNotMatch(html, /allow-same-origin/)
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

test("api_sse_json defaults to / when no route is configured", async () => {
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
      url: "/",
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
    assert.equal(initialize.json().result.serverInfo.name, "chariox-publication")

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

test("human HTTP GET viewer omits the user composer", async () => {
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
    assert.doesNotMatch(response.body, /id="invoke-form"/)
    assert.doesNotMatch(response.body, /type="file" name="artifact" multiple/)
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
    { transport: "human_http", methods: ["POST"], route: "/qa", adapterMarker: /invokeHumanHttp/ },
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
      assert.match(response.body, /publication-viewer/)
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
      url: "/.well-known/chariox/publication/human-http/invoke",
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
    assert.match(response.body, /"permalink":null/)
    assert.match(response.body, /window\.history\.replaceState\(null, '', rootUrl\.pathname \+ rootUrl\.search\)/)
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
      url: "/.well-known/chariox/publication/human-http/invoke",
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
    assert.match(response.body, /type === 'started' && !isTerminalStatus\(payload\.workflow_run\?\.status\)/)
    assert.match(response.body, /setStatus\('Running', false, payload\.workflow_run\)/)
    assert.match(response.body, /let eventStreamSettled = false/)
    assert.match(response.body, /events\.addEventListener\('timeout', \(event\) => \{ applyPublicationEvent\('timeout', parseEventData\(event\)\); reconnect\(\); \}\);/)
    assert.match(response.body, /setTimeout\(\(\) => subscribeHumanHttpEvents\(path\), 1_000\)/)
    assert.match(response.body, /if \(!eventStreamSettled\) setStatus\('Still running · reconnecting'\)/)
    assert.match(response.body, /"permalink":null/)
    assert.match(response.body, /\/display\\\/\[\^\/\]\+/)
    assert.match(response.body, /parts\[0\] === 'publication-ingress'/)
    assert.match(response.body, /directRouteRoots\.includes\('\*'\)/)
    assert.match(response.body, /run-1/)
    assert.deepEqual(seenInput, { prompt: "make tea" })
  } finally {
    await app.close()
  }
})

test("human HTTP status page renders resizable per-node traces and sandboxed app output", async () => {
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
    assert.match(response.body, /class="publication-viewer"/)
    assert.doesNotMatch(response.body, /id="trace-rail"/)
    assert.match(response.body, /events\.addEventListener\('trace'/)
    assert.match(response.body, /frame\.setAttribute\('sandbox', 'allow-scripts allow-forms allow-popups allow-modals allow-downloads'\)/)
    assert.match(response.body, /frame\.srcdoc = renderable\.html/)
    assert.match(response.body, /frame\.src = publicationAppAssetUrl\(renderable\.src\)/)
    assert.match(response.body, /parsed\.kind === 'response'/)
    assert.match(response.body, /renderTraceContent\(item, trace\)/)
    assert.match(response.body, /trace\.level === 'tool_use'/)
    assert.match(response.body, /Generated an interactive HTML workflow update\./)
    assert.match(response.body, /className = 'trace-prose'/)
    assert.match(response.body, /className = 'trace-code'/)
    assert.doesNotMatch(response.body, /item\.innerHTML = .*<pre><\/pre>/)
    assert.match(response.body, /const files = data\.getAll\('artifact'\)/)
    assert.match(response.body, /form\.reset\(\);/)
    assert.ok(response.body.indexOf("form.reset();") < response.body.indexOf("setStatus('Submitting');"))
  } finally {
    await app.close()
  }
})

test("managed publication transports enforce signed caller identity and route roles", async () => {
  const root = await createManagedCallerClaimsPackage()
  configurePublicationCallerClaimsRuntimeForTests({
    deploymentId: "deployment-1",
    environmentId: "environment-1",
    secret: MANAGED_CALLER_CLAIMS_SECRET,
    now: () => new Date(MANAGED_CALLER_CLAIMS_NOW_SECONDS * 1_000),
  })
  try {
    const humanCallers: unknown[] = []
    const human = buildServer(managedTransportConfig(root, "human", "human_http", "/managed/http", ["POST"]), {
      invokeWorkflow: async (invocation) => {
        humanCallers.push(invocation.caller)
        return completedManagedInvocation("run-managed-human")
      },
    })
    try {
      const request = { method: "POST" as const, url: "/managed/http", payload: { prompt: "ship" } }
      assert.equal((await human.app.inject({
        ...request,
        headers: { "x-chariox-agent-app-caller": "forged-human" },
      })).statusCode, 401)
      assert.equal((await human.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("human-forged", { secret: MANAGED_FORGED_SECRET }),
      })).statusCode, 401)
      assert.equal((await human.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("human-role", { roles: ["viewer"] }),
      })).statusCode, 403)
      const accepted = await human.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("human-valid", {
          legacyCaller: "forged-human",
          projectedRoles: "admin",
          projectedSubject: "forged-subject",
        }),
      })
      assert.equal(accepted.statusCode, 200)
      assert.deepEqual(humanCallers, [expectedManagedCaller("human_http", "human-valid")])
    } finally {
      await human.app.close()
    }

    const apiCallers: unknown[] = []
    const api = buildServer(managedTransportConfig(root, "api", "api_sse_json", "/managed/api", ["POST"]), {
      invokeWorkflow: async (invocation) => {
        apiCallers.push(invocation.caller)
        return completedManagedInvocation("run-managed-api")
      },
    })
    try {
      const request = { method: "POST" as const, url: "/managed/api", payload: { prompt: "ship" } }
      assert.equal((await api.app.inject({ ...request })).statusCode, 401)
      assert.equal((await api.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("api-forged", { secret: MANAGED_FORGED_SECRET }),
      })).statusCode, 401)
      assert.equal((await api.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("api-role", { roles: ["viewer"] }),
      })).statusCode, 403)
      const accepted = await api.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("api-valid", { legacyCaller: "forged-api" }),
      })
      assert.equal(accepted.statusCode, 200)
      assert.deepEqual(apiCallers, [expectedManagedCaller("api_sse_json", "api-valid")])
    } finally {
      await api.app.close()
    }

    const mcpCallers: unknown[] = []
    const mcp = buildServer(managedTransportConfig(root, "mcp", "mcp", "/managed/mcp", ["POST"]), {
      invokeWorkflow: async (invocation) => {
        mcpCallers.push(invocation.caller)
        return completedManagedInvocation("run-managed-mcp")
      },
    })
    const mcpPayload = {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "invoke_pub_managed_boundary", arguments: { prompt: "ship" } },
    }
    try {
      const request = { method: "POST" as const, url: "/managed/mcp", payload: mcpPayload }
      assert.equal((await mcp.app.inject({ ...request })).statusCode, 401)
      assert.equal((await mcp.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("mcp-forged", { secret: MANAGED_FORGED_SECRET }),
      })).statusCode, 401)
      assert.equal((await mcp.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("mcp-role", { roles: ["viewer"] }),
      })).statusCode, 403)
      const accepted = await mcp.app.inject({
        ...request,
        headers: managedCallerClaimsHeaders("mcp-valid", { projectedSubject: "forged-subject" }),
      })
      assert.equal(accepted.statusCode, 200)
      assert.deepEqual(mcpCallers, [expectedManagedCaller("mcp", "mcp-valid", {
        tool_name: "invoke_pub_managed_boundary",
      })])
    } finally {
      await mcp.app.close()
    }

    const webSocketCallers: unknown[] = []
    const webSocket = buildServer(
      managedTransportConfig(root, "websocket", "websocket_json", "/managed/ws", ["GET"]),
      {
        invokeWorkflow: async (invocation) => {
          webSocketCallers.push(invocation.caller)
          return completedManagedInvocation("run-managed-websocket")
        },
      },
    )
    try {
      await webSocket.app.listen({ host: "127.0.0.1", port: 0 })
      const address = webSocket.app.server.address()
      const port = typeof address === "object" && address ? address.port : 0
      const url = `ws://127.0.0.1:${port}/managed/ws`
      assert.deepEqual(await rejectedManagedWebSocket(url, {
        "x-chariox-agent-app-caller": "forged-websocket",
      }), { statusCode: 401, code: "caller_authentication_required" })
      assert.deepEqual(await rejectedManagedWebSocket(
        url,
        managedCallerClaimsHeaders("websocket-forged", { secret: MANAGED_FORGED_SECRET }),
      ), { statusCode: 401, code: "caller_claims_invalid" })
      assert.deepEqual(await rejectedManagedWebSocket(
        url,
        managedCallerClaimsHeaders("websocket-role", { roles: ["viewer"] }),
      ), { statusCode: 403, code: "caller_role_denied" })

      const socket = new WebSocket(url, {
        headers: managedCallerClaimsHeaders("websocket-valid", {
          legacyCaller: "forged-websocket",
          projectedRoles: "admin",
        }),
      })
      const reader = createWebSocketReader(socket)
      try {
        assert.deepEqual(await reader.read(), { type: "ready", publication_id: "pub-managed-boundary" })
        socket.send(JSON.stringify({ type: "invoke", input: { prompt: "ship" } }))
        assert.equal((await reader.read() as { type?: string }).type, "accepted")
        assert.deepEqual(webSocketCallers, [expectedManagedCaller("websocket_json", "websocket-valid")])
      } finally {
        socket.close()
      }
    } finally {
      await webSocket.app.close()
    }
  } finally {
    configurePublicationCallerClaimsRuntimeForTests(undefined)
    await rm(root, { recursive: true, force: true })
  }
})

const MANAGED_CALLER_CLAIMS_SECRET = "managed-caller-claims-runtime-secret-0123456789"
const MANAGED_FORGED_SECRET = "forged-caller-claims-runtime-secret-0123456789"
const MANAGED_CALLER_CLAIMS_NOW_SECONDS = Math.floor(Date.parse("2026-07-15T12:00:00.000Z") / 1_000)

async function createManagedCallerClaimsPackage(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-managed-caller-claims-"))
  await writeFile(join(root, "publication.json"), JSON.stringify({
    schema_version: 1,
    package_version: 3,
    publication_id: "pub-managed-boundary",
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
  }))
  await writeFile(join(root, "deployment-contract.json"), JSON.stringify({
    schema_version: 1,
    package_id: `sha256:${"1".repeat(64)}`,
    artifact: {
      content_digest: `sha256:${"2".repeat(64)}`,
      digest_algorithm: "sha256",
      digest_scope: "package_files_excluding_deployment_contract",
    },
    source: {
      publication_id: "pub-managed-boundary",
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
    routes: [
      { id: "human-hook", path: "/managed/http", transport: "human_http", required_roles: ["member"] },
      { id: "api-hook", path: "/managed/api", transport: "api_sse_json", required_roles: ["member"] },
      { id: "mcp-hook", path: "/managed/mcp", transport: "mcp", required_roles: ["member"] },
      { id: "websocket-hook", path: "/managed/ws", transport: "websocket_json", required_roles: ["member"] },
    ],
    provider_requirements: [],
    credential_slots: [],
    configuration: [],
    capabilities: {},
    resources: {},
    presentation: {},
    signatures: [],
  }))
  return root
}

function managedTransportConfig(
  packageRoot: string,
  hook: string,
  transport: string,
  route: string,
  methods: Array<"GET" | "POST">,
): WorkflowPublicationConfig {
  return {
    ...baseConfig,
    publication_id: "pub-managed-boundary",
    hook_id: `${hook}-hook`,
    package_root: packageRoot,
    transport,
    route,
    methods,
  }
}

function managedCallerClaimsHeaders(
  invocationId: string,
  options: {
    readonly legacyCaller?: string
    readonly projectedRoles?: string
    readonly projectedSubject?: string
    readonly roles?: readonly string[]
    readonly secret?: string
  } = {},
): Record<string, string> {
  const encodedHeader = Buffer.from(JSON.stringify({ alg: "HS256", typ: "JWT" })).toString("base64url")
  const encodedPayload = Buffer.from(JSON.stringify({
    iss: "chariox-cloud",
    aud: "deployment-1",
    sub: "user:trusted-user",
    org: "account-1",
    roles: options.roles ?? ["member"],
    deployment_id: "deployment-1",
    environment_id: "environment-1",
    invocation_id: invocationId,
    nonce: `nonce-${invocationId}`,
    iat: MANAGED_CALLER_CLAIMS_NOW_SECONDS,
    exp: MANAGED_CALLER_CLAIMS_NOW_SECONDS + 60,
  })).toString("base64url")
  const unsigned = `${encodedHeader}.${encodedPayload}`
  const signature = createHmac("sha256", options.secret ?? MANAGED_CALLER_CLAIMS_SECRET)
    .update(unsigned)
    .digest("base64url")
  return {
    "x-chariox-caller-claims": `${unsigned}.${signature}`,
    "x-chariox-invocation-id": invocationId,
    ...(options.legacyCaller ? { "x-chariox-agent-app-caller": options.legacyCaller } : {}),
    ...(options.projectedRoles ? { "x-chariox-caller-roles": options.projectedRoles } : {}),
    ...(options.projectedSubject ? { "x-chariox-caller-subject": options.projectedSubject } : {}),
  }
}

function expectedManagedCaller(
  transport: string,
  invocationId: string,
  proof: Record<string, unknown> = {},
): unknown {
  return {
    type: "authenticated",
    proof: {
      transport,
      ...proof,
      publication_caller: {
        account_id: "account-1",
        deployment_id: "deployment-1",
        environment_id: "environment-1",
        invocation_id: invocationId,
        roles: ["member"],
        subject: "user:trusted-user",
      },
    },
  }
}

function completedManagedInvocation(id: string) {
  return {
    accepted: true,
    workflow_run: {
      id,
      status: "Completed",
      final_output: { message: "done" },
    },
  }
}

async function rejectedManagedWebSocket(
  url: string,
  headers: Record<string, string>,
): Promise<{ readonly statusCode: number; readonly code: string | null }> {
  return await new Promise((resolve, reject) => {
    const socket = new WebSocket(url, { headers })
    const timeout = setTimeout(() => {
      socket.terminate()
      reject(new Error("timed out waiting for managed WebSocket rejection"))
    }, 5_000)
    let responseReceived = false
    socket.once("unexpected-response", (_request, response) => {
      responseReceived = true
      const chunks: Buffer[] = []
      response.on("data", (chunk) => chunks.push(Buffer.from(chunk)))
      response.on("end", () => {
        clearTimeout(timeout)
        let code: string | null = null
        try {
          code = (JSON.parse(Buffer.concat(chunks).toString("utf8")) as { error?: { code?: string } }).error?.code ?? null
        } catch {
          code = null
        }
        resolve({ statusCode: response.statusCode ?? 0, code })
      })
      response.resume()
    })
    socket.once("open", () => {
      clearTimeout(timeout)
      socket.close()
      reject(new Error("managed WebSocket unexpectedly opened"))
    })
    socket.once("error", (error) => {
      if (responseReceived) return
      clearTimeout(timeout)
      reject(error)
    })
  })
}
