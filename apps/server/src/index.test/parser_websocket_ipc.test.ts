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
    const response = await regexServer.app.inject({ method: "GET", url: "/page/about/make-it-green%20now" })
    assert.equal(response.statusCode, 202)
    assert.deepEqual(regexInputs[0], { source_path: "about", instruction: "make-it-green now" })
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
  const modes: unknown[] = []
  const { app } = buildServer({
    ...baseConfig,
    transport: "websocket_json",
    route: "/custom/ws",
    input_schema: { type: "object", required: ["task"], properties: { task: { type: "string" } } },
  }, {
    invokeWorkflow: async (invocation) => {
      inputs.push(invocation.input)
      modes.push(invocation.mode)
      return {
        accepted: true,
        workflow_run: {
          id: "run-ws",
          status: "Completed",
          node_runs: [{
            id: "node-run-ws",
            node_id: "node-ws",
            agent_id: "agent-ws",
            status: "Completed",
            summary: "TRACE_SUMMARY",
            thinking_traces: [{ id: "thought-ws", message: "TRACE_THINKING", timestamp_ms: 1 }],
            turn_envelope: {
              runtime_tool_calls: [{
                tool_name: "tool",
                arguments_json: "{\"marker\":\"TRACE_TOOL\"}",
                result_json: null,
                ok: true,
                timestamp_ms: 1,
              }],
            },
          }],
          intermediate_outputs: [{
            id: "partial-ws",
            output: { message: "working" },
            valid: true,
          }],
          final_output: { message: "done" },
        },
      }
    },
  })

  try {
    await app.listen({ host: "127.0.0.1", port: 0 })
    const address = app.server.address()
    const port = typeof address === "object" && address ? address.port : 0
    const socket = new WebSocket(`ws://127.0.0.1:${port}/custom/ws`)
    const reader = createWebSocketReader(socket)
    try {
      assert.deepEqual(await reader.read(), { type: "ready", publication_id: "pub-test" })
      socket.send(JSON.stringify({
        type: "artifact_begin",
        artifact_id: "artifact-1",
        name: "input.txt",
        mime_type: "text/plain",
        size_bytes: 5,
      }))
      assert.deepEqual(await reader.read(), { type: "artifact_ack", status: "begun", artifact_id: "artifact-1" })
      socket.send(JSON.stringify({ type: "artifact_chunk", artifact_id: "artifact-1", data: "aGVs" }))
      assert.deepEqual(await reader.read(), { type: "artifact_ack", status: "chunk", artifact_id: "artifact-1" })
      socket.send(JSON.stringify({ type: "artifact_chunk", artifact_id: "artifact-1", data: "bG8=" }))
      assert.deepEqual(await reader.read(), { type: "artifact_ack", status: "chunk", artifact_id: "artifact-1" })
      socket.send(JSON.stringify({ type: "artifact_end", artifact_id: "artifact-1" }))
      assert.deepEqual(await reader.read(), {
        type: "artifact",
        status: "ready",
        artifact: {
          artifact_id: "artifact-1",
          name: "input.txt",
          type: "text/plain",
          size_bytes: 5,
          base64: "aGVsbG8=",
        },
      })
      socket.send(JSON.stringify({ type: "invoke", input: { task: "ship" } }))
      const accepted = await reader.read() as { type?: string; workflow_run?: { id?: string } }
      assert.equal(accepted.type, "accepted")
      assert.equal(accepted.workflow_run?.id, "run-ws")
      assert.doesNotMatch(JSON.stringify(accepted), /TRACE_SUMMARY|TRACE_THINKING|TRACE_TOOL|thinking_traces|runtime_tool_calls/)
      const queued = await reader.read() as { type?: string; invocation_id?: string }
      assert.equal(queued.type, "queued")
      assert.match(queued.invocation_id ?? "", /^ws_/)
      const started = await reader.read() as { type?: string; workflow_run_id?: string }
      assert.equal(started.type, "started")
      assert.equal(started.workflow_run_id, "run-ws")
      const partial = await reader.read() as { type?: string; message?: string; workflow_run_id?: string }
      assert.equal(partial.type, "partial")
      assert.equal(partial.workflow_run_id, "run-ws")
      assert.equal(partial.message, "working")
      const final = await reader.read() as { type?: string; workflow_run?: { status?: string } }
      assert.equal(final.type, "final")
      assert.equal(final.workflow_run?.status, "Completed")
      assert.deepEqual(inputs, [{
        task: "ship",
        artifacts: [{
          artifact_id: "artifact-1",
          name: "input.txt",
          type: "text/plain",
          size_bytes: 5,
          base64: "aGVsbG8=",
        }],
      }])
      assert.deepEqual(modes, ["async"])
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
