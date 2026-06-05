import assert from "node:assert/strict"
import test from "node:test"

import {
  buildServer,
  invokePublicationInput,
  loadPublicationConfigFromKernel,
  publicationConfigFromKernelRecord,
  type WorkflowPublicationConfig,
} from "./index.js"
import { WebSocket } from "ws"

const baseConfig: WorkflowPublicationConfig = {
  publication_id: "pub-test",
  session_id: "session-1",
  workflow_ref: "workflow-1",
  endpoint_ref: "endpoint-1",
  route: "/*",
  parser: { kind: "json" },
  mode: "sync",
}

test("GET /health returns an ok status payload", async () => {
  const { app } = buildServer(baseConfig, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({ method: "GET", url: "/health" })

    assert.equal(response.statusCode, 200)
    assert.deepEqual(response.json(), { status: "ok" })
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
    kernel_endpoint: "ws://kernel",
    route: "/qa",
    methods: ["POST"],
    parser: { kind: "regex", source: "path", pattern: "^/qa/(?<task>.+)$" },
    input_schema: { type: "object", required: ["task"] },
    mode: "async",
  })
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
  ])
  assert.equal(config.publication_id, "pub-1")
  assert.equal(config.workflow_ref, "workflow-1")
  assert.equal(config.endpoint_ref, "endpoint-1")
  assert.deepEqual(config.methods, ["GET"])
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

test("gateway supports regex and path-template parsers", async () => {
  const regexInputs: unknown[] = []
  const regexServer = buildServer({
    ...baseConfig,
    parser: {
      kind: "regex",
      source: "path",
      pattern: "^/page/(?<source_path>[^/]+)/(?<instruction>.+)$",
    },
  }, {
    invokeWorkflow: async (invocation) => {
      regexInputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const response = await regexServer.app.inject({ method: "GET", url: "/page/about/make-it-green" })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(regexInputs[0], { source_path: "about", instruction: "make-it-green" })
  } finally {
    await regexServer.app.close()
  }

  const templateInputs: unknown[] = []
  const templateServer = buildServer({
    ...baseConfig,
    parser: { kind: "path_template", template: "/store/:list" },
  }, {
    invokeWorkflow: async (invocation) => {
      templateInputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-2", status: "Running" } }
    },
  })

  try {
    const response = await templateServer.app.inject({ method: "GET", url: "/store/apples%20milk" })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(templateInputs[0], { list: "apples milk" })
  } finally {
    await templateServer.app.close()
  }
})

test("gateway returns HTTP 400 for parser and schema failures", async () => {
  let invoked = false
  const parserServer = buildServer({
    ...baseConfig,
    parser: { kind: "regex", source: "path", pattern: "^/ok/(?<value>.+)$" },
  }, {
    invokeWorkflow: async () => {
      invoked = true
      return { accepted: true }
    },
  })
  try {
    const response = await parserServer.app.inject({ method: "GET", url: "/bad/value" })
    assert.equal(response.statusCode, 400)
    assert.match(response.json().error, /did not match/)
    assert.equal(invoked, false)
  } finally {
    await parserServer.app.close()
  }

  const schemaServer = buildServer({
    ...baseConfig,
    input_schema: { type: "object", required: ["name"], properties: { name: { type: "string" } } },
  }, {
    invokeWorkflow: async () => {
      invoked = true
      return { accepted: true }
    },
  })
  try {
    const response = await schemaServer.app.inject({ method: "POST", url: "/schema", payload: { name: 42 } })
    assert.equal(response.statusCode, 400)
    assert.match(response.json().error, /field name expected string/)
    assert.equal(invoked, false)
  } finally {
    await schemaServer.app.close()
  }
})

test("gateway supports custom command parsers", async () => {
  const inputs: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    parser: {
      kind: "custom_command",
      command: process.execPath,
      args: [
        "-e",
        "let body=''; process.stdin.on('data', c => body += c); process.stdin.on('end', () => { const req = JSON.parse(body); process.stdout.write(JSON.stringify({ url: req.url, ok: true })); });",
      ],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      inputs.push(invocation.input)
      return { accepted: true, workflow_run: { id: "run-1", status: "Running" } }
    },
  })

  try {
    const response = await app.inject({ method: "POST", url: "/custom", payload: { ignored: true } })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(inputs[0], { url: "/custom", ok: true })
  } finally {
    await app.close()
  }
})

test("gateway accepts WebSocket publication invocations", async () => {
  const inputs: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    input_schema: { type: "object", required: ["task"], properties: { task: { type: "string" } } },
  }, {
    invokeWorkflow: async (invocation) => {
      inputs.push(invocation.input)
      return {
        accepted: true,
        workflow_run: {
          id: "run-ws",
          status: "Completed",
          final_output: { message: "done" },
        },
      }
    },
  })

  try {
    await app.listen({ host: "127.0.0.1", port: 0 })
    const address = app.server.address()
    const port = typeof address === "object" && address ? address.port : 0
    const socket = new WebSocket(`ws://127.0.0.1:${port}/.well-known/arroba/publication/ws`)
    const reader = createWebSocketReader(socket)
    try {
      assert.deepEqual(await reader.read(), { type: "ready", publication_id: "pub-test" })
      socket.send(JSON.stringify({ type: "invoke", input: { task: "ship" } }))
      const accepted = await reader.read() as { type?: string; workflow_run?: { id?: string } }
      assert.equal(accepted.type, "accepted")
      assert.equal(accepted.workflow_run?.id, "run-ws")
      const final = await reader.read() as { type?: string; workflow_run?: { status?: string } }
      assert.equal(final.type, "final")
      assert.equal(final.workflow_run?.status, "Completed")
      assert.deepEqual(inputs, [{ task: "ship" }])
    } finally {
      socket.close()
    }
  } finally {
    await app.close()
  }
})

test("gateway reports WebSocket input validation errors", async () => {
  const { app } = buildServer({
    ...baseConfig,
    input_schema: { type: "object", required: ["task"], properties: { task: { type: "string" } } },
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    await app.listen({ host: "127.0.0.1", port: 0 })
    const address = app.server.address()
    const port = typeof address === "object" && address ? address.port : 0
    const socket = new WebSocket(`ws://127.0.0.1:${port}/.well-known/arroba/publication/ws`)
    const reader = createWebSocketReader(socket)
    try {
      await reader.read()
      socket.send(JSON.stringify({ type: "invoke", input: { task: 3 } }))
      const error = await reader.read() as { type?: string; error?: string }
      assert.equal(error.type, "error")
      assert.match(error.error ?? "", /field task expected string/)
    } finally {
      socket.close()
    }
  } finally {
    await app.close()
  }
})

test("invokePublicationInput validates and invokes through IPC-shaped caller metadata", async () => {
  const inputs: unknown[] = []
  const callers: unknown[] = []
  const result = await invokePublicationInput({
    ...baseConfig,
    input_schema: { type: "object", required: ["task"], properties: { task: { type: "string" } } },
  }, {
    input: { task: "ship" },
    mode: "async",
    deps: {
      invokeWorkflow: async (invocation) => {
        inputs.push(invocation.input)
        callers.push(invocation.caller)
        return { accepted: true, workflow_run: { id: "run-ipc", status: "Running" } }
      },
    },
  })

  assert.equal(result.workflow_run?.id, "run-ipc")
  assert.deepEqual(inputs, [{ task: "ship" }])
  assert.deepEqual(callers, [{ type: "ipc", proof: { transport: "ipc" } }])

  await assert.rejects(
    () => invokePublicationInput({
      ...baseConfig,
      input_schema: { type: "object", required: ["task"], properties: { task: { type: "string" } } },
    }, {
      input: { task: 7 },
      deps: { invokeWorkflow: async () => ({ accepted: true }) },
    }),
    /field task expected string/,
  )
})

function createWebSocketReader(socket: WebSocket) {
  const queue: unknown[] = []
  const waiters: Array<(value: unknown) => void> = []
  let socketError: Error | null = null
  socket.on("message", (data) => {
    let parsed: unknown
    try {
      parsed = JSON.parse(data.toString())
    } catch (error) {
      socketError = error as Error
      return
    }
    const waiter = waiters.shift()
    if (waiter) waiter(parsed)
    else queue.push(parsed)
  })
  socket.on("error", (error) => {
    socketError = error
  })
  return {
    async read() {
      if (socketError) throw socketError
      const queued = queue.shift()
      if (queued !== undefined) return queued
      return await new Promise<unknown>((resolve, reject) => {
        const timeout = setTimeout(() => reject(new Error("timed out waiting for websocket message")), 5_000)
        waiters.push((value) => {
          clearTimeout(timeout)
          resolve(value)
        })
      })
    },
  }
}
