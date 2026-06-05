import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
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
import { findWorkflowRunByInvocationRequestId } from "./publication-run-correlation.js"
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
    ])
    assert.deepEqual(requests[1], { ListMcpServers: { workspace_id: "/repo" } })
    const materializeRequest = requests.at(-1) as {
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
    assert.match(response.body, /run-1/)
    assert.deepEqual(seenInput, { prompt: "make tea" })
  } finally {
    await app.close()
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
          invocation_prompt: JSON.stringify({
            publication_id: "pub-test",
            request_id: "api_123",
            input: { prompt: "ship" },
          }),
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
