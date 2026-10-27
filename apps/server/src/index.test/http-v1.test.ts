import {
  assert,
  baseConfig,
  buildServer,
  promptFromInvocationInput,
  publicationConfigFromKernelRecord,
  test,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"
import { publicationViewerPage } from "../publication-viewer.js"

test("HTTP V1 keeps GET address-bar and POST form/API invocation on one trigger", async () => {
  const inputs: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    route: "/",
    methods: ["GET", "POST"],
    parser: { kind: "query_params" },
    mode: "async",
  }, {
    invokeWorkflow: async (invocation) => {
      inputs.push(invocation.input)
      return {
        accepted: true,
        workflow_run: {
          id: `run-${inputs.length}`,
          status: "Completed",
          final_output: { message: `done-${inputs.length}` },
        },
      }
    },
  })
  try {
    const viewer = await app.inject({ method: "GET", url: "/", headers: { accept: "text/html" } })
    assert.equal(viewer.statusCode, 200)
    assert.match(viewer.body, /Workflow public view/)
    assert.equal(inputs.length, 0)

    const addressBar = await app.inject({
      method: "GET",
      url: "/?prompt=review%20this",
      headers: { accept: "text/html" },
    })
    assert.equal(addressBar.statusCode, 200)
    assert.equal(promptFromInvocationInput(inputs[0]), "review this")

    const form = await app.inject({
      method: "POST",
      url: "/.well-known/chariox/publication/human-http/invoke",
      headers: { "content-type": "application/json" },
      payload: { prompt: "ship it", artifacts: [] },
    })
    assert.equal(form.statusCode, 200)
    assert.equal(promptFromInvocationInput(inputs[1]), "ship it")
  } finally {
    await app.close()
  }
})

test("HTTP V1 retains supported request parsers", async () => {
  const cases: Array<{
    config: WorkflowPublicationConfig
    request: { method: "GET" | "POST"; url: string; payload?: Record<string, unknown>; headers?: Record<string, string> }
    prompt: string
  }> = [{
    config: { ...baseConfig, transport: "human_http", route: "/prompt/:prompt", methods: ["GET"], parser: { kind: "path_template", template: "/prompt/:prompt" } },
    request: { method: "GET", url: "/prompt/path-value" },
    prompt: "path-value",
  }, {
    config: { ...baseConfig, transport: "human_http", route: "/invoke", methods: ["POST"], parser: { kind: "json" } },
    request: { method: "POST", url: "/invoke", headers: { "content-type": "application/json" }, payload: { prompt: "json-value" } },
    prompt: "json-value",
  }]
  for (const candidate of cases) {
    let prompt: string | null = null
    const { app } = buildServer(candidate.config, {
      invokeWorkflow: async (invocation) => {
        prompt = promptFromInvocationInput(invocation.input)
        return { accepted: true, workflow_run: { id: "run-parser", status: "Completed" } }
      },
    })
    try {
      const response = await app.inject({
        method: candidate.request.method,
        url: candidate.request.url,
        ...(candidate.request.payload ? { payload: candidate.request.payload } : {}),
        ...(candidate.request.headers ? { headers: candidate.request.headers } : {}),
      })
      assert.equal(response.statusCode, 200)
      assert.equal(prompt, candidate.prompt)
    } finally {
      await app.close()
    }
  }
})

test("the viewer contains only the HTTP adapter and internal SSE progress", () => {
  const html = publicationViewerPage({
    ...baseConfig,
    transport: "human_http",
    methods: ["GET", "POST"],
  })
  assert.match(html, /invokeHumanHttp/)
  assert.match(html, /EventSource/)
})
