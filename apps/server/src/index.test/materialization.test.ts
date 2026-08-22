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
  readFile,
  registerCloudPublicationDeploymentBackend,
  releaseAgentAppReplicaInvocation,
  rememberAgentAppInvocationRoute,
  rm,
  setOptionalEnv,
  test,
  tmpdir,
  visibleWorkflowRun,
  waitForCondition,
  writeDeploymentContractFixture,
  writeFile,
  type WorkflowPublicationConfig,
} from "../index.test-support.js"

test("gateway materializes exported publication packages through the kernel", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-materialize-"))
  const runtimeWorkspace = `${root}.runtime`
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      event_bindings_path: "event-bindings.local.json",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "codex",
      allowedProviders: ["codex", "opencode"],
    }])
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
    await writeFile(join(root, "event-bindings.local.json"), JSON.stringify({
      schema_version: 1,
      publication_id: "pub-1",
      destination_environment_id: "environment-1",
      secrets_included: false,
      bindings: [{
        source_binding_id: "source-binding-1",
        generator_id: "dev.chariox.github",
        generator_version: "1.0.0",
        manifest_digest: "sha256:manifest",
        event_type: "pull_request.synchronize",
        event_type_version: 1,
        filter: { repository: "charioxai/drill" },
        requested_scope: "repository:charioxai/drill",
        endpoint_id: "endpoint-1",
        queue_ref: "default",
        reply_mode: "disabled",
        action_ids: [],
        source_environment_id: "source-environment",
        source_revision: 1,
        activation: {
          connection_id: "connection-1",
          environment_id: "environment-1",
          mode: "authorized",
        },
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
      runtimeWorkspace,
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
          if ("CreateWorkflowEventBinding" in request) {
            return { WorkflowEventBindingCreated: { binding: { id: "runtime-binding-1" } } }
          }
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
      "CreateWorkflowEventBinding",
      "AttachToSession",
    ])
    assert.deepEqual(requests[1], { ListMcpServers: { workspace_id: runtimeWorkspace } })
    const materializeRequest = requests.find((request) => "MaterializeWorkflowPublication" in request) as {
      MaterializeWorkflowPublication: {
        snapshot: {
          source_session: { workspace_id: string; worktree_id: string }
          agents: Array<{
            provider: string
            model: string | null
            effort?: string | null
            workspace_id?: string | null
            worktree_id?: string | null
          }>
        }
      }
    }
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.workspace_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.worktree_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.workspace_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.worktree_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.provider, "opencode")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.model, "opencode/gpt-5")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.effort, "medium")
    assert.equal(config.source_session_id, "session-1")
    assert.equal(config.session_id, "runtime-session-1")
    assert.equal(config.workflow_ref, "workflow-1")
  } finally {
    await removeMaterializationFixture(root)
  }
})

test("gateway materializes Agent App replica sessions from package config", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-agent-app-replica-materialize-"))
  const requests: Record<string, unknown>[] = []
  let materializeCount = 0
  try {
    await mkdir(join(root, "app"), { recursive: true })
    await writeFile(join(root, "app", "index.html"), "<!doctype html><main>shop</main>")
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-agent-app",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
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
    await writeDeploymentContractFixture(root, "pub-agent-app")
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
    await removeMaterializationFixture(root)
  }
})

test("gateway remaps portable package workspace paths before local materialization", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-portable-workspace-materialize-"))
  const runtimeWorkspace = `${root}.runtime-${process.pid}`
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-portable",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-portable")
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
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.workspace_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.source_session.worktree_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.workspace_id, runtimeWorkspace)
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.worktree_id, runtimeWorkspace)
  } finally {
    await removeMaterializationFixture(root)
  }
})

test("gateway prompts for unavailable provider/model bindings and persists the replacement", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-bindings-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      default_bindings_path: "bindings.local.json",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "missing-provider",
      allowedProviders: ["missing-provider", "codex"],
    }])
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
    await removeMaterializationFixture(root)
  }
})

test("gateway accepts provider-prefixed captured models when the provider matches", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-prefixed-binding-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      default_bindings_path: "bindings.local.json",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "codex",
    }])
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
    await removeMaterializationFixture(root)
  }
})

test("gateway fails before materialization when provider/model bindings cannot be resolved", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-bindings-fail-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      default_bindings_path: "bindings.local.json",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "missing-provider",
    }])
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
    await removeMaterializationFixture(root)
  }
})

test("gateway materializes captured dev-stub bindings without exposing them in the provider catalog", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-dev-stub-bindings-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      default_bindings_path: "bindings.local.json",
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "dev-stub",
    }])
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
        provider: "dev-stub",
        model: "workflow-intermediate-node",
        effort: "low",
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
      promptProviderModelReplacement: false,
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
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.provider, "dev-stub")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.model, "workflow-intermediate-node")
    assert.equal(materializeRequest.MaterializeWorkflowPublication.snapshot.agents[0]?.effort, "low")
  } finally {
    await removeMaterializationFixture(root)
  }
})

test("gateway fails package materialization before runtime creation when requirements are missing", async () => {
  const root = await mkdtemp(join(tmpdir(), "chariox-server-publication-requirements-"))
  const requests: Record<string, unknown>[] = []
  try {
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 4,
      publication_id: "pub-1",
      source_session_id: "session-1",
      workflow_id: "workflow-1",
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
      hooks: [{
        id: "hook-1",
        transport: "human_http",
        endpoint_id: "endpoint-1",
        route: "/*",
        methods: ["GET"],
      }],
    }))
    await writeDeploymentContractFixture(root, "pub-1", "hook-1", [{
      agentId: "agent-1",
      capturedProvider: "codex",
    }])
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
    await removeMaterializationFixture(root)
  }
})

async function removeMaterializationFixture(root: string): Promise<void> {
  await rm(root, { recursive: true, force: true })
  await rm(`${root}.runtime`, { recursive: true, force: true })
  await rm(`${root}.runtime-${process.pid}`, { recursive: true, force: true })
}
