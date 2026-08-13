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
