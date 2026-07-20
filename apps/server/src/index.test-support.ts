import assert from "node:assert/strict"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createServer } from "node:http"
import { join } from "node:path"
import { tmpdir } from "node:os"
import test from "node:test"

import {
  buildServer,
  invokePublicationInput,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
  type WorkflowPublicationConfig,
} from "./index.js"
import { promptFromInvocationInput, publicationInvocationEnvelope } from "./kernel-publication-client.js"
import {
  appendCloudPublicationDeploymentLogs,
  registerCloudPublicationDeploymentBackend,
} from "./publication-cloud-deployment.js"
import {
  clearAgentAppEffectStoresForTests,
  publicationForAgentAppInvocation,
  rememberAgentAppInvocationRoute,
} from "./publication-agent-app-effects.js"
import {
  acquireAgentAppReplica,
  clearAgentAppReplicaPoolsForTests,
  enqueueAgentAppReplicaDispatch,
  releaseAgentAppReplicaInvocation,
} from "./publication-agent-app-replicas.js"
import { findWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import { ensurePublicationRuntimeAttached } from "./publication-runtime-pump.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
} from "./publication-trace-events.js"
import { visibleWorkflowRun } from "./publication-workflow-run-visibility.js"
import { WebSocket } from "ws"

export {
  assert,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
  createServer,
  join,
  tmpdir,
  test,
  buildServer,
  invokePublicationInput,
  loadPublicationConfigFromKernel,
  loadPublicationPackageConfig,
  publicationConfigFromKernelRecord,
  publicationConfigFromPackage,
  promptFromInvocationInput,
  publicationInvocationEnvelope,
  appendCloudPublicationDeploymentLogs,
  registerCloudPublicationDeploymentBackend,
  clearAgentAppEffectStoresForTests,
  publicationForAgentAppInvocation,
  rememberAgentAppInvocationRoute,
  acquireAgentAppReplica,
  clearAgentAppReplicaPoolsForTests,
  enqueueAgentAppReplicaDispatch,
  releaseAgentAppReplicaInvocation,
  findWorkflowRunByInvocationRequestId,
  ensurePublicationRuntimeAttached,
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
  visibleWorkflowRun,
  WebSocket,
}
export type { WorkflowPublicationConfig }

export const baseConfig: WorkflowPublicationConfig = {
  publication_id: "pub-test",
  session_id: "session-1",
  workflow_ref: "workflow-1",
  endpoint_ref: "endpoint-1",
  route: "/*",
  parser: { kind: "json" },
  mode: "sync",
}

export function firstSetCookieValue(value: string | string[] | number | undefined): string {
  const raw = Array.isArray(value) ? value[0] : value
  if (typeof raw !== "string") assert.fail("expected set-cookie header")
  return raw.split(";")[0] ?? raw
}

export async function waitForCondition(
  condition: () => boolean,
  message: string,
  timeoutMs = 1_000,
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (condition()) return
    await new Promise((resolve) => setTimeout(resolve, 10))
  }
  assert.fail(message)
}

export function setOptionalEnv(name: string, value: string | undefined) {
  if (value === undefined) delete process.env[name]
  else process.env[name] = value
}

export function publishedHttpConfig(
  id: string,
  route: string,
  methods: Array<"GET" | "POST">,
  parser: NonNullable<WorkflowPublicationConfig["parser"]>,
): WorkflowPublicationConfig {
  return publicationConfigFromPackage({
    schema_version: 1,
    package_version: 1,
    publication_id: `pub-${id}`,
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    hooks: [{
      id: `hook-${id}`,
      transport: "human_http",
      endpoint_id: "endpoint-1",
      route,
      methods,
      parser,
      mode: "sync",
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
}

export function publishedTransportConfig(input: {
  readonly id: string
  readonly transport: string
  readonly route?: string
  readonly methods?: string[]
  readonly parser?: NonNullable<WorkflowPublicationConfig["parser"]>
  readonly inputSchema?: WorkflowPublicationConfig["input_schema"]
  readonly mode?: "sync" | "async"
}): WorkflowPublicationConfig {
  return publicationConfigFromPackage({
    schema_version: 1,
    package_version: 1,
    publication_id: `pub-${input.id}`,
    source_session_id: "session-1",
    workflow_id: "workflow-1",
    hooks: [{
      id: `hook-${input.id}`,
      transport: input.transport,
      endpoint_id: "endpoint-1",
      ...(input.route ? { route: input.route } : {}),
      ...(input.methods ? { methods: input.methods } : {}),
      ...(input.parser ? { parser: input.parser } : {}),
      ...(input.inputSchema ? { input_schema: input.inputSchema } : {}),
      ...(input.mode ? { mode: input.mode } : {}),
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
}

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
    const socket = new WebSocket(`ws://127.0.0.1:${port}/`)
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

export function createWebSocketReader(socket: WebSocket) {
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

export function sseEventNames(body: string) {
  return body
    .split("\n")
    .filter((line) => line.startsWith("event: "))
    .map((line) => line.slice("event: ".length))
}

export function providerCatalogResponse(providers: Record<string, string[]>) {
  return {
    ProviderCatalog: {
      catalog: {
        all: Object.entries(providers).map(([id, models]) => ({
          id,
          name: id,
          models: Object.fromEntries(models.map((model) => [model, { id: model, name: model }])),
        })),
        default: {},
        connected: Object.keys(providers),
      },
    },
  }
}
