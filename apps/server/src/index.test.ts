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
import { releaseAgentAppReplicaInvocation } from "./publication-agent-app-replicas.js"
import { findWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
import {
  collectPublicationTraceEvents,
  createPublicationTraceStreamState,
} from "./publication-trace-events.js"
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

function firstSetCookieValue(value: string | string[] | number | undefined): string {
  const raw = Array.isArray(value) ? value[0] : value
  if (typeof raw !== "string") assert.fail("expected set-cookie header")
  return raw.split(";")[0] ?? raw
}

async function waitForCondition(
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

test("publication gateway registers local runtime backend with Cloud deployment", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const registered = await registerCloudPublicationDeploymentBackend({
    deploymentId: "deployment-1",
    publication: baseConfig,
    localUrl: "http://127.0.0.1:4567/",
    now: () => 1_700_000_000_000,
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ deployment: { id: "deployment-1" } }), { status: 200 })
    },
  })

  assert.equal(registered, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/publication-deployments/deployment-1/local-backend")
  assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer session-token")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    status: "ready",
    runtimeSessionId: "session-1",
    backendTarget: {
      kind: "local_runtime",
      url: "http://127.0.0.1:4567/",
      updated_at_ms: 1_700_000_000_000,
    },
  })
})

test("publication gateway can mark Cloud local runtime backend unavailable", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const registered = await registerCloudPublicationDeploymentBackend({
    deploymentId: "deployment-unavailable",
    publication: baseConfig,
    status: "unavailable",
    lastError: "relay display tunnel unavailable",
    now: () => 1_700_000_000_000,
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ deployment: { id: "deployment-unavailable" } }), { status: 200 })
    },
  })

  assert.equal(registered, true)
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    status: "unavailable",
    runtimeSessionId: "session-1",
    lastError: "relay display tunnel unavailable",
  })
})

test("publication gateway can register Cloud backend from env profile", async () => {
  const previous = {
    apiUrl: process.env.ARROBA_PUBLICATION_CLOUD_API_URL,
    accountId: process.env.ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID,
    token: process.env.ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN,
  }
  process.env.ARROBA_PUBLICATION_CLOUD_API_URL = "https://cloud-env.example.test/"
  process.env.ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID = "account-env"
  process.env.ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN = "token-env"
  try {
    const calls: Array<{ url: string; init: RequestInit }> = []
    const registered = await registerCloudPublicationDeploymentBackend({
      deploymentId: "deployment-env",
      publication: baseConfig,
      localUrl: "http://127.0.0.1:4568/",
      fetch: async (url, init) => {
        calls.push({ url: String(url), init: init ?? {} })
        return new Response(JSON.stringify({ deployment: { id: "deployment-env" } }), { status: 200 })
      },
    })

    assert.equal(registered, true)
    assert.equal(calls[0]?.url, "https://cloud-env.example.test/publication-deployments/deployment-env/local-backend")
    assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer token-env")
    assert.equal(JSON.parse(String(calls[0]?.init.body)).accountId, "account-env")
  } finally {
    setOptionalEnv("ARROBA_PUBLICATION_CLOUD_API_URL", previous.apiUrl)
    setOptionalEnv("ARROBA_PUBLICATION_CLOUD_ACCOUNT_ID", previous.accountId)
    setOptionalEnv("ARROBA_PUBLICATION_CLOUD_SESSION_TOKEN", previous.token)
  }
})

test("publication gateway appends account-scoped deployment logs", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const appended = await appendCloudPublicationDeploymentLogs({
    deploymentId: "deployment-log",
    profile: {
      apiUrl: "https://cloud.example.test/",
      accountId: "account-1",
      cloudSessionToken: "session-token",
    },
    entries: [{
      level: "info",
      message: "agent app action `cart.add` completed",
      metadata: { kind: "agent_app_action", action_id: "cart.add" },
    }],
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ logs: [] }), { status: 201 })
    },
  })

  assert.equal(appended, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/publication-deployments/deployment-log/logs")
  assert.equal((calls[0]?.init.headers as Record<string, string>).authorization, "Bearer session-token")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    accountId: "account-1",
    entries: [{
      level: "info",
      message: "agent app action `cart.add` completed",
      metadata: { kind: "agent_app_action", action_id: "cart.add" },
    }],
  })
})

test("publication gateway appends runner-scoped deployment logs", async () => {
  const calls: Array<{ url: string; init: RequestInit }> = []
  const appended = await appendCloudPublicationDeploymentLogs({
    deploymentId: "deployment-log",
    profile: { apiUrl: "https://cloud.example.test/", accountId: "account-1" },
    runnerKey: "runner-secret",
    entries: [{ level: "warn", message: "agent app action `cart.add` failed" }],
    fetch: async (url, init) => {
      calls.push({ url: String(url), init: init ?? {} })
      return new Response(JSON.stringify({ logs: [] }), { status: 201 })
    },
  })

  assert.equal(appended, true)
  assert.equal(calls[0]?.url, "https://cloud.example.test/runner/publication-deployments/deployment-log/logs")
  assert.deepEqual(JSON.parse(String(calls[0]?.init.body)), {
    runnerKey: "runner-secret",
    entries: [{ level: "warn", message: "agent app action `cart.add` failed" }],
  })
})

test("publication trace events honor per-node level policy", () => {
  const publication: WorkflowPublicationConfig = {
    ...baseConfig,
    trace_exposure: {
      nodes: {
        "node-a": ["output_summary"],
        "node-b": ["output_summary", "assistant_messages"],
        "node-c": ["output_summary", "assistant_messages", "thinking"],
        "node-d": ["output_summary", "assistant_messages", "thinking", "tool_use"],
      },
    },
    trace_context: {
      nodes: {
        "node-a": { node_id: "node-a", node_label: "Summarizer", agent_id: "agent-a", agent_alias: "summary" },
        "node-b": { node_id: "node-b", node_label: "Research", agent_id: "agent-b", agent_alias: "researcher" },
        "node-c": { node_id: "node-c", node_label: "Planner", agent_id: "agent-c", agent_alias: "planner" },
        "node-d": { node_id: "node-d", node_label: "Builder", agent_id: "agent-d", agent_alias: "builder" },
        "node-e": { node_id: "node-e", node_label: "Hidden", agent_id: "agent-e", agent_alias: "hidden" },
      },
    },
  }
  const workflowRun = {
    id: "run-1",
    status: "Completed",
    node_runs: [{
      id: "run-node-a",
      node_id: "node-a",
      agent_id: "agent-a-runtime",
      status: "Completed",
      summary: "A summary",
      completion: { summary: "A completion", output: { message: "A assistant output" } },
      thinking_traces: [{ id: "thinking-a", message: "A private reasoning", timestamp_ms: 11 }],
      completed_at_ms: 20,
    }, {
      id: "run-node-b",
      node_id: "node-b",
      agent_id: "agent-b-runtime",
      status: "Completed",
      summary: "B summary",
      completion: { summary: "B completion", output: { message: "B assistant output" } },
      thinking_traces: [{ id: "thinking-b", message: "B private reasoning", timestamp_ms: 21 }],
      completed_at_ms: 30,
    }, {
      id: "run-node-c",
      node_id: "node-c",
      agent_id: "agent-c-runtime",
      status: "Completed",
      completion: { summary: "C summary" },
      thinking_traces: [{ id: "thinking-c", message: "C thinking", timestamp_ms: 31 }],
      completed_at_ms: 40,
    }, {
      id: "run-node-d",
      node_id: "node-d",
      agent_id: "agent-d-runtime",
      status: "Completed",
      completion: { summary: "D summary", output: { message: "D assistant output" } },
      thinking_traces: [{ id: "thinking-d", message: "D thinking", timestamp_ms: 41 }],
      turn_envelope: {
        runtime_tool_calls: [{
          tool_name: "lookup",
          arguments_json: "{\"q\":\"d\"}",
          result_json: "{\"ok\":true}",
          ok: true,
          timestamp_ms: 42,
        }],
      },
      completed_at_ms: 50,
    }, {
      id: "run-node-e",
      node_id: "node-e",
      agent_id: "agent-e-runtime",
      status: "Completed",
      completion: { summary: "E summary", output: { message: "E assistant output" } },
      thinking_traces: [{ id: "thinking-e", message: "E thinking", timestamp_ms: 51 }],
      completed_at_ms: 60,
    }],
    messages: [
      {
        id: "message-b",
        source_node_run_id: "run-node-b",
        target_node_id: "node-c",
        message_type: "handoff",
        summary: "B handoff",
        handoff_payload: "{\"summary\":\"B handoff\"}",
        created_at_ms: 25,
      },
      {
        id: "message-c",
        source_node_run_id: "run-node-c",
        target_node_id: "node-d",
        message_type: "handoff",
        summary: "C handoff",
        handoff_payload: "{\"summary\":\"C handoff\"}",
        created_at_ms: 35,
      },
      {
        id: "message-d",
        source_node_run_id: "run-node-d",
        target_node_id: "node-e",
        message_type: "handoff",
        summary: "D handoff",
        handoff_payload: "{\"summary\":\"D handoff\"}",
        created_at_ms: 45,
      },
    ],
    final_output: { message: { kind: "html", html: "<main>C assistant output</main>" } },
    completed_by_node_run_id: "run-node-c",
  }

  const state = createPublicationTraceStreamState()
  const firstPass = collectPublicationTraceEvents(publication, workflowRun, state)
  const secondPass = collectPublicationTraceEvents(publication, workflowRun, state)

  assert.deepEqual(firstPass.map((event) => [event.node_id, event.agent_alias, event.level, event.message]), [
    ["node-a", "summary", "output_summary", "A completion"],
    ["node-b", "researcher", "output_summary", "B completion"],
    ["node-b", "researcher", "assistant_messages", "B handoff"],
    ["node-b", "researcher", "assistant_messages", "B assistant output"],
    ["node-c", "planner", "output_summary", "C summary"],
    ["node-c", "planner", "assistant_messages", "C handoff"],
    ["node-c", "planner", "assistant_messages", "{\"message\":{\"kind\":\"html\",\"html\":\"<main>C assistant output</main>\"}}"],
    ["node-c", "planner", "thinking", "C thinking"],
    ["node-d", "builder", "output_summary", "D summary"],
    ["node-d", "builder", "assistant_messages", "D handoff"],
    ["node-d", "builder", "assistant_messages", "D assistant output"],
    ["node-d", "builder", "thinking", "D thinking"],
    ["node-d", "builder", "tool_use", "lookup ok"],
  ])
  assert.equal(firstPass.some((event) => event.node_id === "node-e"), false)
  assert.deepEqual(firstPass.map((event) => event.sequence), [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13])
  assert.deepEqual(secondPass, [])
})

function setOptionalEnv(name: string, value: string | undefined) {
  if (value === undefined) delete process.env[name]
  else process.env[name] = value
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
        client_id: `arroba-publication-gateway-${process.pid}-pub-1`,
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

test("gateway materializes exported publication packages through the kernel", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-materialize-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
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
      queues: [{ id: "workflow-1:default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }],
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
    await writeFile(join(root, "bindings.local.json"), JSON.stringify({
      schema_version: 1,
      provider_model_overrides: [{
        agent_id: "agent-1",
        node_ids: ["node-1"],
        captured: { provider: "codex", model: null, effort: null },
        replacement: { provider: "opencode", model: "gpt-5", effort: "medium" },
      }],
    }))
    await writeFile(join(root, "requirements.json"), JSON.stringify({
      schema_version: 1,
      mcps: [{ name: "playwright" }],
      skills: [{ name: "qa" }],
      scripts: [{ name: "deploy" }],
      connectors: [{ name: "github" }],
      credentials: [{ name: "github-token", used_by: "github" }],
    }))

    const config = await loadPublicationPackageConfig(root, {
      kernelEndpoint: "ws://kernel",
      materialize: true,
      client: {
        send: async (request) => {
          requests.push(request)
          if ("GetProviderCatalog" in request) return providerCatalogResponse({
            opencode: ["gpt-5"],
          })
          if ("ListMcpServers" in request) return { McpServersListed: { mcps: [{ name: "playwright" }] } }
          if ("ListSkills" in request) return { SkillsListed: { skills: [{ name: "qa" }] } }
          if ("ListScripts" in request) return { ScriptsListed: { scripts: [{ name: "deploy" }] } }
          if ("ListConnectors" in request) return { ConnectorsListed: { connectors: [{ name: "github" }] } }
          if ("ListCredentials" in request) return { CredentialsListed: { credentials: [{ id: "github-token" }] } }
          return {
            WorkflowPublicationMaterialized: {
              publication_id: "pub-1",
              session: { id: "runtime-session-1" },
              agent_id_map: { "agent-1": "agent-2" },
            },
          }
        },
      },
    })

    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
      "GetProviderCatalog",
      "ListMcpServers",
      "ListSkills",
      "ListScripts",
      "ListConnectors",
      "ListCredentials",
      "MaterializeWorkflowPublication",
      "AttachToSession",
    ])
    assert.deepEqual(requests[1], { ListMcpServers: { workspace_id: "/repo" } })
    const materializeRequest = requests.find((request) => "MaterializeWorkflowPublication" in request) as {
      MaterializeWorkflowPublication: {
        snapshot: {
          agents: Array<{ provider: string; model: string | null; effort?: string | null }>
        }
      }
    }
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.provider, "opencode")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.model, "gpt-5")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.effort, "medium")
    assert.equal(config.source_session_id, "session-1")
    assert.equal(config.session_id, "runtime-session-1")
    assert.equal(config.workflow_ref, "workflow-1")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway materializes Agent App replica sessions from package config", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-replica-materialize-"))
  const requests: Record<string, unknown>[] = []
  let materializeCount = 0
  try {
    await mkdir(join(root, "app"), { recursive: true })
    await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 2,
      publication_id: "pub-agent-app",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        routes: [{ path: "/add/*", prompt_source: "path_tail" }],
        replicas: { count: 2, per_caller_ordering: true },
      },
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
      queues: [{ id: "workflow-1:default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }],
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))

    const config = await loadPublicationPackageConfig(root, {
      kernelEndpoint: "ws://kernel",
      materialize: true,
      validateProviderBindings: false,
      validateRequirements: false,
      client: {
        send: async (request) => {
          requests.push(request)
          if ("MaterializeWorkflowPublication" in request) {
            materializeCount += 1
            return {
              WorkflowPublicationMaterialized: {
                publication_id: "pub-agent-app",
                session: { id: `runtime-session-${materializeCount}` },
                agent_id_map: { "agent-1": `agent-${materializeCount + 1}` },
              },
            }
          }
          if ("AttachToSession" in request) {
            return { SessionAttached: { attachment: { id: `attachment-${requests.length}` } } }
          }
          throw new Error(`unexpected request ${JSON.stringify(request)}`)
        },
      },
    })

    assert.equal(config.session_id, "runtime-session-1")
    assert.deepEqual(config.replica_session_ids, ["runtime-session-1", "runtime-session-2"])
    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
      "MaterializeWorkflowPublication",
      "MaterializeWorkflowPublication",
      "AttachToSession",
      "AttachToSession",
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway remaps portable package workspace paths before local materialization", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-portable-workspace-materialize-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 2,
      publication_id: "pub-portable",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
      schema_version: 1,
      source_session: {
        id: "session-1",
        workspace_id: "/workspace",
        worktree_id: "/workspace",
      },
      workflow: {
        id: "workflow-1",
        alias: null,
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
        edges: [],
        endpoints: [{ id: "endpoint-1", entry_node_id: "node-1" }],
      },
      endpoint: { id: "endpoint-1", entry_node_id: "node-1" },
      queues: [{ id: "workflow-1:default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }],
      agents: [{
        id: "agent-1",
        agent_ref: "agent-ref-1",
        session_id: "session-1",
        alias: null,
        provider: "claude",
        model: "claude-sonnet-4-6",
        workspace_id: "/workspace",
        worktree_id: "/workspace",
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))

    await loadPublicationPackageConfig(root, {
      kernelEndpoint: "ws://kernel",
      materialize: true,
      validateProviderBindings: false,
      validateRequirements: false,
      client: {
        send: async (request) => {
          requests.push(request)
          if ("MaterializeWorkflowPublication" in request) {
            return {
              WorkflowPublicationMaterialized: {
                publication_id: "pub-portable",
                session: { id: "runtime-session-1" },
                agent_id_map: { "agent-1": "agent-2" },
              },
            }
          }
          if ("AttachToSession" in request) {
            return { SessionAttached: { attachment: { id: "attachment-1" } } }
          }
          throw new Error(`unexpected request ${JSON.stringify(request)}`)
        },
      },
    })

    const materializeRequest = requests.find((request) => "MaterializeWorkflowPublication" in request) as {
      MaterializeWorkflowPublication: {
        snapshot: {
          source_session: { workspace_id: string; worktree_id: string }
          agents: Array<{ workspace_id?: string | null; worktree_id?: string | null }>
        }
      }
    }
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.workspace_id, root)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.worktree_id, root)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.workspace_id, root)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.worktree_id, root)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway prompts for unavailable provider/model bindings and persists the replacement", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-bindings-"))
  const requests: Record<string, unknown>[] = []
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
      queues: [{ id: "workflow-1:default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }],
      agents: [{
        id: "agent-1",
        agent_ref: "agent-ref-1",
        session_id: "session-1",
        alias: null,
        provider: "missing-provider",
        model: "missing-model",
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))

    const config = await loadPublicationPackageConfig(root, {
      kernelEndpoint: "ws://kernel",
      materialize: true,
      promptProviderModelReplacement: async () => ({ provider: "codex", model: "gpt-5", effort: "high" }),
      client: {
        send: async (request) => {
          requests.push(request)
          if ("GetProviderCatalog" in request) return providerCatalogResponse({ codex: ["gpt-5"] })
          return {
            WorkflowPublicationMaterialized: {
              publication_id: "pub-1",
              session: { id: "runtime-session-1" },
              agent_id_map: { "agent-1": "agent-2" },
            },
          }
        },
      },
    })

    const materializeRequest = requests.at(-1) as {
      MaterializeWorkflowPublication: {
        snapshot: {
          agents: Array<{ provider: string; model: string | null; effort?: string | null }>
        }
      }
    }
    assert.equal(config.session_id, "runtime-session-1")
    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
      "GetProviderCatalog",
      "MaterializeWorkflowPublication",
    ])
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.provider, "codex")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.model, "gpt-5")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.effort, "high")

    const bindings = JSON.parse(await readFile(join(root, "bindings.local.json"), "utf8")) as {
      provider_model_overrides: Array<{ replacement?: { provider?: string; model?: string | null; effort?: string | null } | null }>
    }
    assert.deepEqual(bindings.provider_model_overrides[0]?.replacement, {
      provider: "codex",
      model: "gpt-5",
      effort: "high",
    })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway accepts provider-prefixed captured models when the provider matches", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-prefixed-binding-"))
  const requests: Record<string, unknown>[] = []
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
      queues: [{ id: "workflow-1:default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }],
      agents: [{
        id: "agent-1",
        agent_ref: "agent-ref-1",
        session_id: "session-1",
        alias: null,
        provider: "codex",
        model: "codex/gpt-5.5",
        effort: "high",
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))

    await loadPublicationPackageConfig(root, {
      kernelEndpoint: "ws://kernel",
      materialize: true,
      client: {
        send: async (request) => {
          requests.push(request)
          if ("GetProviderCatalog" in request) return providerCatalogResponse({ codex: ["gpt-5.5"] })
          return {
            WorkflowPublicationMaterialized: {
              publication_id: "pub-1",
              session: { id: "runtime-session-1" },
              agent_id_map: { "agent-1": "agent-2" },
            },
          }
        },
      },
    })

    const materializeRequest = requests.at(-1) as {
      MaterializeWorkflowPublication: {
        snapshot: {
          agents: Array<{ provider: string; model: string | null; effort?: string | null }>
        }
      }
    }
    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
      "GetProviderCatalog",
      "MaterializeWorkflowPublication",
    ])
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.provider, "codex")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.model, "gpt-5.5")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.effort, "high")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway fails before materialization when provider/model bindings cannot be resolved", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-bindings-fail-"))
  const requests: Record<string, unknown>[] = []
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
        provider: "missing-provider",
        model: "missing-model",
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({ schema_version: 1 }))

    await assert.rejects(
      () => loadPublicationPackageConfig(root, {
        kernelEndpoint: "ws://kernel",
        materialize: true,
        promptProviderModelReplacement: false,
        client: {
          send: async (request) => {
            requests.push(request)
            if ("GetProviderCatalog" in request) return providerCatalogResponse({ codex: ["gpt-5"] })
            throw new Error(`unexpected request: ${JSON.stringify(request)}`)
          },
        },
      }),
      /publication provider\/model is unavailable for agent agent-1: missing-provider\/missing-model/,
    )
    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["GetProviderCatalog"])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("gateway fails package materialization before runtime creation when requirements are missing", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-publication-requirements-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
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
    await writeFile(join(root, "requirements.json"), JSON.stringify({
      schema_version: 1,
      skills: [{ name: "qa" }],
      credentials: [{ name: "github-token" }],
    }))

    await assert.rejects(
      () => loadPublicationPackageConfig(root, {
        kernelEndpoint: "ws://kernel",
        materialize: true,
        client: {
          send: async (request) => {
            requests.push(request)
            if ("GetProviderCatalog" in request) return providerCatalogResponse({ codex: [] })
            if ("ListSkills" in request) return { SkillsListed: { skills: [] } }
            if ("ListCredentials" in request) return { CredentialsListed: { credentials: [] } }
            throw new Error(`unexpected request: ${JSON.stringify(request)}`)
          },
        },
      }),
      /publication requirements are missing: skill:qa, credential:github-token/,
    )
    assert.deepEqual(requests.map((request) => Object.keys(request)[0]), [
      "GetProviderCatalog",
      "ListSkills",
      "ListCredentials",
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
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
    route: "/ignored",
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
      url: "/invoke",
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

    const genericRoute = await app.inject({ method: "POST", url: "/ignored", payload: { prompt: "nope" } })
    assert.equal(genericRoute.statusCode, 404)
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
    route: "/ignored",
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
      url: "/mcp",
      payload: { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26" } },
    })
    assert.equal(initialize.statusCode, 200)
    assert.equal(initialize.json().result.serverInfo.name, "arroba-publication")

    const tools = await app.inject({
      method: "POST",
      url: "/mcp",
      payload: { jsonrpc: "2.0", id: 2, method: "tools/list" },
    })
    assert.equal(tools.statusCode, 200)
    assert.equal(tools.json().result.tools[0].name, "invoke_pub_mcp")

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
    assert.deepEqual(called.json().result.content, [{ type: "text", text: "mcp done" }])
    assert.equal(called.json().result.structuredContent.workflow_run_id, "run-mcp")
    assert.deepEqual(called.json().result.structuredContent.artifacts, [{ name: "artifact.txt", url: "artifact://mcp" }])
    assert.equal(called.json().result.isError, false)
    assert.deepEqual(seenInput, { prompt: "ship" })
    assert.deepEqual(seenCaller, { type: "anonymous", proof: { transport: "mcp", tool_name: "invoke_pub_mcp" } })

    const genericRoute = await app.inject({ method: "POST", url: "/ignored", payload: { prompt: "nope" } })
    assert.equal(genericRoute.statusCode, 404)
  } finally {
    await app.close()
  }
})

test("mcp accepts browser preflight for JSON-RPC tool calls", async () => {
  const { app } = buildServer({
    ...baseConfig,
    publication_id: "pub-mcp",
    transport: "mcp",
    route: "/mcp",
    methods: ["POST"],
  }, {
    invokeWorkflow: async () => ({ accepted: true }),
  })

  try {
    const response = await app.inject({
      method: "OPTIONS",
      url: "/mcp",
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
    { transport: "api_sse_json", methods: ["POST"], route: "/invoke", adapterMarker: /invokeApiSse/ },
    { transport: "websocket_json", methods: ["GET"], route: "/.well-known/arroba/publication/ws", adapterMarker: /invokeWebSocket/ },
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

test("agent app gateway serves packaged app assets", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-assets-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  await writeFile(join(root, "app", "styles.css"), "main { color: red; }")
  const { app } = buildServer({
    ...baseConfig,
    package_root: root,
    agent_app: {
      enabled: true,
      assets: { public_dir: "app", index: "index.html" },
      routes: [],
    },
  })

  try {
    const index = await app.inject({ method: "GET", url: "/" })
    assert.equal(index.statusCode, 200)
    assert.match(index.headers["content-type"] as string, /text\/html/)
    assert.equal(index.body, "<!doctype html><main>shop</main>")

    const styles = await app.inject({ method: "GET", url: "/styles.css" })
    assert.equal(styles.statusCode, 200)
    assert.match(styles.headers["content-type"] as string, /text\/css/)
    assert.equal(styles.body, "main { color: red; }")

    const traversal = await app.inject({ method: "GET", url: "/../publication.json" })
    assert.notEqual(traversal.statusCode, 200)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app config validation rejects invalid launch config before serving", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-invalid-config-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  try {
    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-route",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        routes: [{ path: "add/*" }],
      },
    }), /route path must start with \//)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-action",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        routes: [{
          path: "/add/*",
          manipulation: { allowed_actions: ["missing-action"] },
        }],
      },
    }), /unknown action missing-action/)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-invalid-replicas",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "app", index: "index.html" },
        replicas: { count: 0 },
        routes: [{ path: "/add/*" }],
      },
    }), /replicas\.count/)

    assert.throws(() => buildServer({
      ...baseConfig,
      publication_id: "pub-missing-assets",
      package_root: root,
      agent_app: {
        enabled: true,
        assets: { public_dir: "missing", index: "index.html" },
        routes: [{ path: "/add/*" }],
      },
    }), /assets\.public_dir does not exist/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app wrapped route invokes workflow with path-tail prompt and streams viewer shell", async () => {
  let seenInput: unknown = null
  let seenProof: Record<string, unknown> | null = null
  const previousPort = process.env.PORT
  process.env.PORT = "34567"
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-route-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    trace_exposure: { nodes: { "node-1": ["output_summary", "assistant_messages", "thinking", "tool_use"] } },
    trace_context: {
      nodes: {
        "node-1": {
          node_id: "node-1",
          node_label: "Checkout Agent",
          agent_id: "agent-1",
          agent_alias: "shopper",
        },
      },
    },
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
          allowed_actions: ["cart.add"],
        },
      }],
      actions: {
        "cart.add": {
          input_schema: {
            type: "object",
            required: ["sku"],
            properties: { sku: { type: "string" } },
          },
          transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/add" },
        },
        "cart.admin": {
          transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/admin" },
        },
      },
    },
  }, {
    invokeWorkflow: async (invocation) => {
      seenInput = invocation.input
      seenProof = invocation.caller.proof as Record<string, unknown>
      return {
        accepted: true,
        workflow_run: { id: "run-shopping", status: "Running" },
      }
    },
  })

  try {
    const response = await app.inject({
      method: "GET",
      url: "/add/1%20kg%20bananas",
      headers: { accept: "text/html" },
    })
    assert.equal(response.statusCode, 200)
    assert.match(response.headers["content-type"] as string, /text\/html/)
    assert.match(response.body, /class="split-viewer"/)
    assert.match(response.body, /run-shopping/)
    assert.deepEqual(seenInput, { prompt: "1 kg bananas" })
    const proof = seenProof as Record<string, unknown> | null
    assert.deepEqual(Object.keys((proof?.agent_app_actions as Record<string, unknown>) ?? {}), ["cart.add"])
    assert.deepEqual(
      (proof?.agent_app_audit as Record<string, unknown> | undefined)?.url,
      "http://127.0.0.1:34567/.well-known/arroba/agent-app/audit-log",
    )
    const auditToken = (proof?.agent_app_audit as Record<string, unknown> | undefined)?.token
    assert.equal(typeof auditToken, "string")
    const auditResponse = await app.inject({
      method: "POST",
      url: "/.well-known/arroba/agent-app/audit-log",
      payload: {
        token: auditToken,
        entries: [{
          level: "info",
          message: "agent app action `cart.add` completed",
          metadata: { kind: "agent_app_action", action_id: "cart.add" },
        }],
      },
    })
    assert.equal(auditResponse.statusCode, 200)
    assert.deepEqual(JSON.parse(auditResponse.body), { accepted: true, appended: false })
  } finally {
    setOptionalEnv("PORT", previousPort)
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app final response effects overlay generated files for serve mode", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-overlay-"))
  await mkdir(join(root, "app"), { recursive: true })
  await writeFile(join(root, "app", "index.html"), "<!doctype html><main>base shop</main>")
  const { app } = buildServer({
    ...baseConfig,
    transport: "human_http",
    package_root: root,
    trace_exposure: { nodes: { "node-1": ["output_summary", "assistant_messages", "thinking", "tool_use"] } },
    trace_context: {
      nodes: {
        "node-1": {
          node_id: "node-1",
          node_label: "Checkout Agent",
          agent_id: "agent-1",
          agent_alias: "shopper",
        },
      },
    },
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
          protected_paths: ["/payments/**"],
          allowed_actions: ["cart.search", "cart.add", "cart.checkout"],
        },
      }],
      actions: {
        "cart.search": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/search" } },
        "cart.add": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/add" } },
        "cart.checkout": { transport: { kind: "http", method: "POST", url: "http://127.0.0.1:1/cart/checkout" } },
      },
    },
  }, {
    invokeWorkflow: async () => ({
      accepted: true,
      workflow_run: {
        id: "run-shopping",
        status: "Completed",
        workflow_id: "workflow-1",
        completed_by_node_run_id: "node-run-1",
        completed_at_ms: 1_700_000_000_000,
        node_runs: [{
          id: "node-run-1",
          node_id: "node-1",
          agent_id: "agent-1",
          status: "Completed",
          summary: "Prepared checkout from the shopping list.",
          completion: { summary: "Checkout ready with three products." },
          thinking_traces: [{ id: "thinking-1", message: "Map quantities to catalog SKUs before checkout.", timestamp_ms: 1_700_000_000_001 }],
          turn_envelope: {
            runtime_tool_calls: [{
              tool_name: "cart.checkout",
              arguments_json: "{\"items\":3}",
              result_json: "{\"ok\":true}",
              ok: true,
              timestamp_ms: 1_700_000_000_002,
            }],
          },
        }],
        messages: [{
          id: "message-1",
          source_node_run_id: "node-run-1",
          target_node_id: "node-1",
          message_type: "assistant",
          summary: "Added bananas, Coca-Cola, and chips to the basket.",
          handoff_payload: "",
          created_at_ms: 1_700_000_000_003,
        }],
        final_output: {
          message: JSON.stringify({
            kind: "response",
            response: { mode: "serve", entry: "/generated/checkout.html" },
            effects: {
              overlay: [{
                path: "/generated/checkout.html",
                mime_type: "text/html; charset=utf-8",
                content: "<!doctype html><main>custom banana checkout</main>",
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
    assert.match(invoke.body, /run-shopping/)
    assert.match(invoke.body, /initialTraces/)
    assert.match(invoke.body, /Checkout ready with three products/)
    assert.match(invoke.body, /cart.checkout ok/)
    assert.match(invoke.body, /frame\.src = publicationAppAssetUrl\(renderable\.src\)/)
    const cookie = firstSetCookieValue(invoke.headers["set-cookie"])
    assert.match(cookie, /arroba_agent_app_session=/)

    const checkout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie } })
    assert.equal(checkout.statusCode, 200)
    assert.match(checkout.headers["content-type"] as string, /text\/html/)
    assert.equal(checkout.body, "<!doctype html><main>custom banana checkout</main>")

    const unrelatedSession = await app.inject({ method: "GET", url: "/generated/checkout.html" })
    assert.equal(unrelatedSession.statusCode, 404)
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app session overlays are isolated by browser session", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-server-agent-app-session-overlay-"))
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
          allowed_paths: ["/generated/**"],
        },
      }],
    },
  }, {
    invokeWorkflow: async (invocation) => {
      const prompt = (invocation.input as { prompt?: string }).prompt ?? "unknown"
      return {
        accepted: true,
        workflow_run: {
          id: `run-${prompt}`,
          status: "Completed",
          final_output: {
            message: JSON.stringify({
              kind: "response",
              response: { mode: "serve", entry: "/generated/checkout.html" },
              effects: {
                overlay: [{
                  path: "/generated/checkout.html",
                  mime_type: "text/html; charset=utf-8",
                  content: `<!doctype html><main>${prompt} checkout</main>`,
                }],
              },
            }),
          },
        },
      }
    },
  })

  try {
    const firstInvoke = await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html" } })
    assert.equal(firstInvoke.statusCode, 200)
    const firstCookie = firstSetCookieValue(firstInvoke.headers["set-cookie"])

    const secondInvoke = await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html" } })
    assert.equal(secondInvoke.statusCode, 200)
    const secondCookie = firstSetCookieValue(secondInvoke.headers["set-cookie"])
    assert.notEqual(firstCookie, secondCookie)

    const firstCheckout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie: firstCookie } })
    assert.equal(firstCheckout.statusCode, 200)
    assert.equal(firstCheckout.body, "<!doctype html><main>bananas checkout</main>")

    const secondCheckout = await app.inject({ method: "GET", url: "/generated/checkout.html", headers: { cookie: secondCookie } })
    assert.equal(secondCheckout.statusCode, 200)
    assert.equal(secondCheckout.body, "<!doctype html><main>chips checkout</main>")
  } finally {
    await app.close()
    await rm(root, { recursive: true, force: true })
  }
})

test("agent app replica selection preserves caller affinity across hidden sessions", async () => {
  const selectedReplicas: unknown[] = []
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
      return {
        accepted: true,
        workflow_run: { id: `run-${selectedReplicas.length}`, status: "Running" },
      }
    },
  })

  try {
    await app.inject({ method: "GET", url: "/add/apples", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-a" } })
    await app.inject({ method: "GET", url: "/add/bananas", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-b" } })
    await app.inject({ method: "GET", url: "/add/chips", headers: { accept: "text/html", "x-arroba-agent-app-caller": "caller-a" } })
    assert.deepEqual(selectedReplicas, ["replica-session-1", "replica-session-2", "replica-session-1"])
  } finally {
    await app.close()
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
    assert.match(response.body, /\/\.well-known\/arroba\/publication\/invocations\//)
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
    const socket = new WebSocket(`ws://127.0.0.1:${port}/.well-known/arroba/publication/ws`)
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

function sseEventNames(body: string) {
  return body
    .split("\n")
    .filter((line) => line.startsWith("event: "))
    .map((line) => line.slice("event: ".length))
}

function providerCatalogResponse(providers: Record<string, string[]>) {
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
