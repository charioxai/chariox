import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand manages workflow list, create, show, and alias", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListWorkflows" in request) {
          return { WorkflowsListed: { workflows: [workflow] } }
        }
        if ("CreateWorkflow" in request) {
          return { WorkflowCreated: { workflow, session } }
        }
        if ("ResolveWorkflow" in request) {
          return { WorkflowResolved: { workflow } }
        }
        return { WorkflowAliased: { workflow: { ...workflow, alias: "review" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const listResult = await executeShellCommand(parseShellCommand("workflow list"), context, { client: fake.client })
  const newResult = await executeShellCommand(parseShellCommand("workflow new qa as wf"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow show workflow-1"), context, { client: fake.client })
  const aliasResult = await executeShellCommand(parseShellCommand("workflow alias workflow-1 review"), context, { client: fake.client })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /workflow-1 \(qa\) nodes=1/)
  assert.equal(newResult.ok, true)
  assert.deepEqual(newResult.bindings, { wf: "workflow-1" })
  assert.deepEqual(newResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(showResult.ok, true)
  assert.match(showResult.message ?? "", /workflow workflow-1 \(qa\)/)
  assert.deepEqual(showResult.contextUpdates, { workflowId: "workflow-1" })
  assert.equal(aliasResult.ok, true)
  assert.match(aliasResult.message ?? "", /aliased as review/)
  assert.deepEqual(requests, [
    { ListWorkflows: { session_id: "session-1" } },
    { CreateWorkflow: { session_id: "session-1", alias: "qa" } },
    { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { AliasWorkflow: { session_id: "session-1", workflow_ref: "workflow-1", alias: "review" } },
  ])
})

test("executeShellCommand exports and imports workflow-code packages and source", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-workflow-code-shell-"))
  try {
    const workflowCodePackage = {
      package_version: 2,
      name: "toy-flow",
      language: "JavaScript",
      source: "workflow.define({ alias: \"toy\" })\n",
      source_sha256: "source-sha256",
      source_bytes: 34,
      definition_sha256: "definition-sha256",
      definition: {
        schema_version: 1,
        workflow: { alias: "toy" },
      },
      validation: { ok: true },
      exported_at_ms: 1_000,
    }
    const artifact = {
      metadata: {
        name: "imported-toy",
        language: "JavaScript",
        path: "/repo/.arroba/workflow-code/imported-toy.json",
        source_sha256: "source-sha256",
        source_bytes: 34,
        validation: { ok: true },
        provenance: { created_by: { user_id: "user-1" }, updated_by: { user_id: "user-1" } },
        created_at_ms: 1_000,
        updated_at_ms: 1_000,
      },
      source: workflowCodePackage.source,
      definition: workflowCodePackage.definition,
    }
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ExportWorkflowCodePackage" in request) {
            return { WorkflowCodePackageExported: { package: workflowCodePackage } }
          }
          if ("ImportWorkflowCodePackage" in request) {
            return { WorkflowCodePackageImported: { artifact } }
          }
          const payload = request.ExportWorkflowCodeSource as { format?: string }
          if (payload.format === "directory") {
            return {
              WorkflowCodeSourceExported: {
                export: {
                  name: "toy-flow",
                  language: "JavaScript",
                  format: "directory",
                  source_path: "workflow.js",
                  source: "async function defineWorkflow(workflow) {}\n",
                  source_sha256: "dir-source-sha256",
                  source_bytes: 43,
                  definition_sha256: "definition-sha256",
                  files: [
                    { path: "workflow.js", contents: "async function defineWorkflow(workflow) {}\n", sha256: "dir-source-sha256" },
                    { path: "schemas/final.json", contents: "{\n  \"type\": \"object\"\n}\n", sha256: "schema-sha256" },
                    { path: "manifest.json", contents: "{\n  \"manifest_version\": 1\n}\n", sha256: "manifest-sha256" },
                  ],
                },
              },
            }
          }
          return {
            WorkflowCodeSourceExported: {
              export: {
                name: "toy-flow",
                language: "JavaScript",
                format: "inline",
                source_path: "workflow.js",
                source: workflowCodePackage.source,
                source_sha256: "source-sha256",
                source_bytes: 34,
                definition_sha256: "definition-sha256",
                files: [],
              },
            },
          }
        },
      },
    }
    const context = createDefaultShellContext({ workspace: root, worktree: root, sessionId: "session-1" })
    const packageExport = await executeShellCommand(parseShellCommand("workflow code package export toy-flow exports/toy.workflow-code.json"), context, { client: fake.client })
    const packageImport = await executeShellCommand(parseShellCommand("workflow code package import exports/toy.workflow-code.json imported-toy --overwrite"), context, { client: fake.client })
    const sourceInline = await executeShellCommand(parseShellCommand("workflow code source export toy-flow exports/toy.js"), context, { client: fake.client })
    const sourceDirectory = await executeShellCommand(parseShellCommand("workflow code source export toy-flow exports/toy-source --format directory"), context, { client: fake.client })
    const sourceDirectoryAlias = await executeShellCommand(parseShellCommand("workflow code source export-dir toy-flow exports/toy-source-alias"), context, { client: fake.client })
    const workflowSource = await executeShellCommand(parseShellCommand("workflow code source export workflow-1 exports/workflow.js --workflow"), context, { client: fake.client })

    assert.equal(packageExport.ok, true)
    assert.equal(packageImport.ok, true)
    assert.equal(sourceInline.ok, true)
    assert.equal(sourceDirectory.ok, true)
    assert.equal(sourceDirectoryAlias.ok, true)
    assert.equal(workflowSource.ok, true)
    assert.equal(JSON.parse(await readFile(join(root, "exports/toy.workflow-code.json"), "utf8")).name, "toy-flow")
    assert.equal(await readFile(join(root, "exports/toy.js"), "utf8"), workflowCodePackage.source)
    assert.equal(await readFile(join(root, "exports/toy-source/workflow.js"), "utf8"), "async function defineWorkflow(workflow) {}\n")
    assert.equal(await readFile(join(root, "exports/toy-source/schemas/final.json"), "utf8"), "{\n  \"type\": \"object\"\n}\n")
    assert.equal(await readFile(join(root, "exports/toy-source-alias/manifest.json"), "utf8"), "{\n  \"manifest_version\": 1\n}\n")
    assert.deepEqual(requests, [
      { ExportWorkflowCodePackage: { session_id: "session-1", name: "toy-flow" } },
      {
        ImportWorkflowCodePackage: {
          session_id: "session-1",
          package: workflowCodePackage,
          name: "imported-toy",
          overwrite: true,
          node_path: "node",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "inline",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "directory",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "artifact", name: "toy-flow" },
          format: "directory",
        },
      },
      {
        ExportWorkflowCodeSource: {
          session_id: "session-1",
          target: { kind: "workflow", workflow_ref: "workflow-1" },
          format: "inline",
        },
      },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand runs and controls workflow runs", async () => {
  const workflow = makeWorkflow()
  const workflowRun = makeWorkflowRun()
  const session = makeSession({ workflows: [workflow], workflow_runs: [workflowRun] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("InvokeWorkflowEndpoint" in request) {
          return { WorkflowRunInvoked: { workflow_run: workflowRun, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowRuns" in request) {
          return { WorkflowRunsListed: { workflow_runs: [workflowRun] } }
        }
        if ("GetWorkflowRun" in request) {
          return { WorkflowRun: { workflow_run: workflowRun } }
        }
        if ("CancelWorkflowRun" in request) {
          return { WorkflowRunCancelled: { workflow_run: { ...workflowRun, status: "Cancelled" }, session } }
        }
        return { WorkflowRunResumed: { workflow_run: { ...workflowRun, status: "Running" }, session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const runResult = await executeShellCommand(parseShellCommand("workflow run workflow-1 endpoint-1 Run QA --queue priority"), context, { client: fake.client })
  const runsResult = await executeShellCommand(parseShellCommand("workflow runs workflow-1"), context, { client: fake.client })
  const showRunResult = await executeShellCommand(parseShellCommand("workflow run-show run-1"), context, { client: fake.client })
  const cancelResult = await executeShellCommand(parseShellCommand("workflow cancel run-1"), context, { client: fake.client })
  const resumeResult = await executeShellCommand(parseShellCommand("workflow resume run-1"), context, { client: fake.client })
  assert.equal(runResult.ok, true)
  assert.match(runResult.message ?? "", /started workflow run run-1/)
  assert.deepEqual(runResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
  assert.equal(runsResult.ok, true)
  assert.match(runsResult.message ?? "", /run-1 workflow=workflow-1/)
  assert.equal(showRunResult.ok, true)
  assert.equal(showRunResult.format, "json")
  assert.equal(cancelResult.ok, true)
  assert.match(cancelResult.message ?? "", /cancelled workflow run run-1 \[cancelled\]/)
  assert.equal(resumeResult.ok, true)
  assert.match(resumeResult.message ?? "", /resumed workflow run run-1 \[running\]/)
  assert.deepEqual(requests, [
    { InvokeWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", queue_ref: "priority", prompt: "Run QA" } },
    { ListWorkflowRuns: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { GetWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { CancelWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
    { ResumeWorkflowRun: { session_id: "session-1", workflow_run_ref: "run-1" } },
  ])
})

test("executeShellCommand manages workflow graph and endpoints", async () => {
  const workflow = makeWorkflow({
    nodes: [
      { id: "node-1", agent_id: "agent-1" },
      { id: "node-2", agent_id: "agent-2" },
    ],
    edges: [{ id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }],
  })
  const session = makeSession({ workflows: [workflow] })
  const node = { id: "node-2", agent_id: "agent-2" }
  const edge = { id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }
  const endpoint = { id: "endpoint-1", alias: "default", entry_node_id: "node-1" }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ListAgents" in request) {
          return { AgentsListed: { agents: [makeAgent(), makeAgent({ id: "agent-2", agent_ref: "agent-2", alias: "reviewer" })] } }
        }
        if ("AddWorkflowNode" in request) {
          return { WorkflowNodeAdded: { node, workflow, session } }
        }
        if ("RemoveWorkflowNode" in request) {
          return { WorkflowNodeRemoved: { node, workflow, session } }
        }
        if ("AddWorkflowEdge" in request) {
          return { WorkflowEdgeAdded: { edge, workflow, session } }
        }
        if ("RemoveWorkflowEdge" in request) {
          return { WorkflowEdgeRemoved: { edge, workflow, session } }
        }
        if ("CreateWorkflowEndpoint" in request) {
          return { WorkflowEndpointCreated: { endpoint, workflow, session } }
        }
        if ("AliasWorkflowEndpoint" in request) {
          return { WorkflowEndpointAliased: { endpoint: { ...endpoint, alias: "smoke" }, workflow, session } }
        }
        return { WorkflowEndpointBound: { endpoint, workflow, session } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: "workflow-1",
  })
  const nodeAdd = await executeShellCommand(parseShellCommand("workflow node add reviewer as node"), context, { client: fake.client })
  const nodeRemove = await executeShellCommand(parseShellCommand("workflow node remove node-2"), context, { client: fake.client })
  const edgeAdd = await executeShellCommand(parseShellCommand("workflow edge add node-1 node-2"), context, { client: fake.client })
  const edgeRemove = await executeShellCommand(parseShellCommand("workflow edge remove edge-1"), context, { client: fake.client })
  const endpointNew = await executeShellCommand(parseShellCommand("workflow endpoint new workflow-1 node-1 default"), context, { client: fake.client })
  const endpointAlias = await executeShellCommand(parseShellCommand("workflow endpoint alias endpoint-1 smoke"), context, { client: fake.client })
  const endpointBind = await executeShellCommand(parseShellCommand("workflow endpoint bind endpoint-1 node-1"), context, { client: fake.client })
  assert.equal(nodeAdd.ok, true)
  assert.deepEqual(nodeAdd.bindings, { node: "node-2" })
  assert.equal(nodeRemove.ok, true)
  assert.equal(edgeAdd.ok, true)
  assert.equal(edgeRemove.ok, true)
  assert.equal(endpointNew.ok, true)
  assert.equal(endpointAlias.ok, true)
  assert.equal(endpointBind.ok, true)
  assert.deepEqual(requests, [
    { ListAgents: { session_id: "session-1" } },
    { AddWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", agent_id: "agent-2" } },
    { RemoveWorkflowNode: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-2" } },
    { AddWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", from_node_id: "node-1", to_node_id: "node-2" } },
    { RemoveWorkflowEdge: { session_id: "session-1", workflow_ref: "workflow-1", edge_id: "edge-1" } },
    { CreateWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", entry_node_id: "node-1", alias: "default" } },
    { AliasWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", alias: "smoke" } },
    { BindWorkflowEndpoint: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", entry_node_id: "node-1" } },
  ])
})

test("executeShellCommand manages workflow node instructions from shell", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-workflow-instructions-"))
  try {
    await writeFile(join(root, "instructions.md"), "Review the handoff and return JSON.", "utf8")
    const workflow = makeWorkflow({
      nodes: [
        { id: "node-1", agent_id: "agent-1", instructions: "Old instructions" },
      ],
    })
    const updatedWorkflow = makeWorkflow({
      nodes: [
        { id: "node-1", agent_id: "agent-1", instructions: "Review the handoff and return JSON." },
      ],
    })
    const session = makeSession({ workflows: [updatedWorkflow] })
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ResolveWorkflow" in request) {
            return { WorkflowResolved: { workflow } }
          }
          return { WorkflowNodeInstructionsUpdated: { node: updatedWorkflow.nodes![0], workflow: updatedWorkflow, session } }
        },
      },
    }
    const context = createDefaultShellContext({
      workspace: root,
      worktree: root,
      sessionId: "session-1",
      workflowId: "workflow-1",
    })

    const showResult = await executeShellCommand(parseShellCommand("workflow node instructions show node-1"), context, { client: fake.client })
    const setResult = await executeShellCommand(parseShellCommand("workflow node instructions set workflow-1 node-1 instructions.md"), context, { client: fake.client })

    assert.equal(showResult.ok, true)
    assert.equal(showResult.message, "Old instructions")
    assert.deepEqual(showResult.contextUpdates, { workflowId: "workflow-1" })
    assert.equal(setResult.ok, true)
    assert.match(setResult.message ?? "", /updated workflow node node-1 instructions/)
    assert.deepEqual(setResult.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
    assert.deepEqual(requests, [
      { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } },
      { UpdateWorkflowNodeInstructions: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-1", instructions: "Review the handoff and return JSON." } },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand forwards workflow node instruction edits as design ops for TUI clients", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-workflow-design-instructions-"))
  try {
    await writeFile(join(root, "instructions.md"), "Collaborative node prompt.", "utf8")
    const workflow = makeWorkflow({
      nodes: [
        { id: "node-1", agent_id: "agent-1", instructions: null },
      ],
    })
    const session = makeSession({ workflows: [workflow] })
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ResolveWorkflow" in request) {
            return { WorkflowResolved: { workflow } }
          }
          return {
            WorkflowDesignOpAccepted: {
              event: {
                session_id: "session-1",
                origin_client_id: "cli-1",
                op_id: "shell-test",
                kernel_sequence: 1,
                op: { kind: "node_update", workflow_id: "workflow-1", node_id: "node-1", patch: { instructions: "Collaborative node prompt." } },
              },
              session,
            },
          }
        },
      },
    }
    const context = createDefaultShellContext({
      workspace: root,
      worktree: root,
      sessionId: "session-1",
      workflowId: "workflow-1",
    })

    const result = await executeShellCommand(parseShellCommand("workflow node instructions set node-1 instructions.md"), context, { client: fake.client, clientId: "cli-1" })

    assert.equal(result.ok, true)
    assert.match(result.message ?? "", /updated workflow node node-1 instructions/)
    assert.deepEqual(result.contextUpdates, { workflowId: "workflow-1", sessionId: "session-1", agentId: "agent-1" })
    assert.equal(requests.length, 2)
    assert.deepEqual(requests[0], { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } })
    const designRequest = requests[1] as {
      ApplyWorkflowDesignOp?: {
        session_id?: string
        origin_client_id?: string
        op_id?: string
        op?: unknown
      }
    }
    assert.equal(designRequest.ApplyWorkflowDesignOp?.session_id, "session-1")
    assert.equal(designRequest.ApplyWorkflowDesignOp?.origin_client_id, "cli-1")
    assert.match(designRequest.ApplyWorkflowDesignOp?.op_id ?? "", /^shell-/)
    assert.deepEqual(designRequest.ApplyWorkflowDesignOp?.op, {
      kind: "node_update",
      workflow_id: "workflow-1",
      node_id: "node-1",
      patch: { instructions: "Collaborative node prompt." },
    })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand manages workflow publications", async () => {
  const publication = makeWorkflowPublication({ queue_ref: "priority" })
  const session = makeSession({ workflows: [makeWorkflow()], workflow_publications: [publication] })
  const fake = fakeClient((request) => {
    if ("CreateWorkflowPublication" in request) {
      return { WorkflowPublicationCreated: { publication, session } }
    }
    if ("ListWorkflowPublications" in request) {
      return { WorkflowPublicationsListed: { publications: [publication] } }
    }
    if ("GetWorkflowPublication" in request) {
      return { WorkflowPublication: { publication } }
    }
    return { WorkflowPublicationDisabled: { publication: { ...publication, enabled: false }, session } }
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: "workflow-1",
  })

  const createResult = await executeShellCommand(
    parseShellCommand("workflow publication create endpoint-1 public_qa --queue priority --route /qa --method POST"),
    context,
    { client: fake.client },
  )
  const listResult = await executeShellCommand(parseShellCommand("workflow publication list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow publication show publication-1"), context, { client: fake.client })
  const disableResult = await executeShellCommand(parseShellCommand("workflow publication disable publication-1"), context, { client: fake.client })

  assert.equal(createResult.ok, true)
  assert.match(createResult.message ?? "", /created workflow publication publication-1/)
  assert.deepEqual(createResult.contextUpdates, { sessionId: "session-1", agentId: "agent-1", workflowId: "workflow-1" })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /publication-1 \(public_qa\) workflow=workflow-1 endpoint=endpoint-1 queue=priority enabled=true route=\/qa methods=POST/)
  assert.equal(showResult.ok, true)
  assert.equal(showResult.format, "json")
  assert.equal(disableResult.ok, true)
  assert.match(disableResult.message ?? "", /disabled workflow publication publication-1/)
  assert.deepEqual(fake.requests, [
    {
      CreateWorkflowPublication: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        endpoint_ref: "endpoint-1",
        queue_ref: "priority",
        alias: "public_qa",
        route: "/qa",
        methods: ["POST"],
        transport: null,
        parser: null,
        input_schema: null,
        trace_exposure: null,
        mode: null,
        sync_timeout_ms: null,
        poll_ms: null,
      },
    },
    { ListWorkflowPublications: { session_id: "session-1" } },
    { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    { DisableWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
  ])
})

test("executeShellCommand exports a workflow publication package", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-export-test-"))
  try {
    const publication = makeWorkflowPublication({
      mode: "async",
      queue_ref: "priority",
    })
    const queue = { id: "default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }
    const watchdog = makeWorkflowWatchdog({ policy: "queue", invocation_prompt: "published schedule" })
    const session = makeSession({
      workflows: [makeWorkflow()],
      workflow_publications: [publication],
      workflow_prompt_queues: [queue],
      workflow_watchdogs: [watchdog],
    })
    const fake = fakeClient((request) => {
      if ("GetWorkflowPublication" in request) {
        return { WorkflowPublication: { publication } }
      }
      if ("GetSessionState" in request) {
        return { SessionState: { session } }
      }
      throw new Error(`unexpected request ${JSON.stringify(request)}`)
    })
    const context = createDefaultShellContext({
      workspace: root,
      worktree: root,
      sessionId: "session-1",
      workflowId: "workflow-1",
    })

    const result = await executeShellCommand(
      parseShellCommand("workflow publication export publication-1 exported --kernel-url ws://kernel.example"),
      context,
      { client: fake.client },
    )

    assert.equal(result.ok, true)
    assert.match(result.message ?? "", /exported workflow publication publication-1/)
    const config = JSON.parse(await readFile(join(root, "exported", "publication.config.json"), "utf8"))
    assert.equal(config.publication_id, "publication-1")
    assert.equal(config.kernel_endpoint, "ws://kernel.example")
    assert.equal("auth" in config, false)
    const packageJson = JSON.parse(await readFile(join(root, "exported", "publication.json"), "utf8"))
    assert.equal(packageJson.schema_version, 1)
    assert.equal(packageJson.hooks[0].transport, "human_http")
    assert.equal(packageJson.hooks[0].queue_ref, "priority")
    const snapshot = JSON.parse(await readFile(join(root, "exported", "workflow.snapshot.json"), "utf8"))
    assert.equal(snapshot.workflow.id, "workflow-1")
    assert.equal(snapshot.endpoint.id, "endpoint-1")
    assert.equal(snapshot.queues[0].id, "default")
    assert.equal(snapshot.watchdogs[0].id, "watchdog-1")
    assert.equal(snapshot.watchdogs[0].invocation_prompt, "published schedule")
    assert.equal(snapshot.agents[0].id, "agent-1")
    const requirements = JSON.parse(await readFile(join(root, "exported", "requirements.json"), "utf8"))
    assert.deepEqual(requirements.mcps, [])
    const bindings = JSON.parse(await readFile(join(root, "exported", "bindings.example.json"), "utf8"))
    assert.equal(bindings.provider_model_overrides[0].agent_id, "agent-1")
    const html = await readFile(join(root, "exported", "public", "index.html"), "utf8")
    assert.match(html, /public_qa/)
    const launcher = await readFile(join(root, "exported", "run.sh"), "utf8")
    assert.match(launcher, /arroba-workflow-gateway/)
    assert.match(launcher, /ARROBA_PUBLICATION_PACKAGE/)
    const readme = await readFile(join(root, "exported", "README.md"), "utf8")
    assert.match(readme, /arroba-workflow-call --package/)
    assert.doesNotMatch(readme, /paired sender auth/)
    assert.doesNotMatch(readme, /well-known\/arroba\/publication\/pair/)
    assert.deepEqual(fake.requests, [
      { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
      { GetSessionState: { session_id: "session-1" } },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand configures workflow publication package bindings", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-bindings-test-"))
  try {
    const workflow = makeWorkflow({
      nodes: [
        { id: "node-1", agent_id: "agent-1" },
        { id: "node-2", agent_id: "agent-2" },
      ],
    })
    await writeFile(join(root, "publication.json"), JSON.stringify({
      schema_version: 1,
      package_version: 1,
      publication_id: "publication-1",
      workflow_id: "workflow-1",
      default_bindings_path: "bindings.local.json",
      hooks: [],
    }, null, 2), "utf8")
    await writeFile(join(root, "workflow.snapshot.json"), JSON.stringify({
      schema_version: 1,
      workflow,
      endpoint: workflow.endpoints![0],
      queues: [],
      watchdogs: [],
      agents: [
        makeAgent({ id: "agent-1", provider: "opencode", model: "gpt-5.2", effort: "high" }),
        makeAgent({ id: "agent-2", agent_ref: "agent-2", provider: "codex", model: "gpt-5", effort: null }),
      ],
    }, null, 2), "utf8")
    const fake = fakeClient((request) => {
      throw new Error(`unexpected request ${JSON.stringify(request)}`)
    })
    const context = createDefaultShellContext({
      workspace: root,
      worktree: root,
      sessionId: "session-1",
      workflowId: "workflow-1",
    })

    const showResult = await executeShellCommand(parseShellCommand("workflow publication config show ."), context, { client: fake.client })
    const setResult = await executeShellCommand(parseShellCommand("workflow publication config set . agent-1 claude sonnet-4 medium"), context, { client: fake.client })
    const localBindingsAfterSet = JSON.parse(await readFile(join(root, "bindings.local.json"), "utf8"))
    const clearResult = await executeShellCommand(parseShellCommand("workflow publication config clear . agent-1"), context, { client: fake.client })
    const localBindingsAfterClear = JSON.parse(await readFile(join(root, "bindings.local.json"), "utf8"))

    assert.equal(showResult.ok, true)
    assert.match(showResult.message ?? "", /agent-1 nodes=node-1 captured=opencode\/gpt-5\.2 effort=high replacement=default/)
    assert.match(showResult.message ?? "", /local bindings file has not been created yet/)
    assert.equal(setResult.ok, true)
    assert.match(setResult.message ?? "", /updated workflow publication binding for agent-1/)
    assert.deepEqual(localBindingsAfterSet.provider_model_overrides[0].replacement, {
      provider: "claude",
      model: "sonnet-4",
      effort: "medium",
    })
    assert.equal(clearResult.ok, true)
    assert.match(clearResult.message ?? "", /cleared workflow publication binding for agent-1/)
    assert.equal(localBindingsAfterClear.provider_model_overrides[0].replacement, null)
    assert.deepEqual(fake.requests, [])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand manages advanced workflow settings, watchdogs, and queue", async () => {
  const workflow = makeWorkflow({ flush_agent_context_before_run: false, run_output_schema_ref: "final", intermediate_output_schema_ref: "progress" })
  const session = makeSession({ attachment_ids: ["attachment-1"], workflows: [workflow] })
  const node = { id: "node-1", agent_id: "agent-1", can_complete_workflow_run: true, max_turns: 3 }
  const watchdog = makeWorkflowWatchdog()
  const queue = { id: "default", workflow_id: "workflow-1", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }
  const queued = { id: "prompt-1", queue_id: "default", workflow_id: "workflow-1", endpoint_id: "endpoint-1", source: "manual" as const, status: "queued" as const, created_at_ms: 0, updated_at_ms: 0 }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("GetSessionState" in request) {
          return { SessionState: { session } }
        }
        if ("SetWorkflowFlushContext" in request) {
          return { WorkflowFlushContextUpdated: { workflow, session } }
        }
        if ("SetWorkflowRunOutputSchema" in request) {
          return { WorkflowRunOutputSchemaUpdated: { workflow, session } }
        }
        if ("SetWorkflowNodeCanCompleteRun" in request) {
          return { WorkflowNodeCanCompleteRunUpdated: { node, workflow, session } }
        }
        if ("CreateWorkflowWatchdog" in request) {
          return { WorkflowWatchdogCreated: { watchdog, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowWatchdogs" in request) {
          return { WorkflowWatchdogsListed: { watchdogs: [watchdog] } }
        }
        if ("SetWorkflowWatchdogEnabled" in request) {
          return { WorkflowWatchdogUpdated: { watchdog: { ...watchdog, enabled: false }, session } }
        }
        if ("RemoveWorkflowWatchdog" in request) {
          return { WorkflowWatchdogRemoved: { watchdog, session } }
        }
        if ("ListWorkflowPromptQueues" in request) {
          return { WorkflowPromptQueuesListed: { queues: [queue] } }
        }
        if ("ListQueuedWorkflowPrompts" in request) {
          return { QueuedWorkflowPromptsListed: { queued_prompts: [queued] } }
        }
        if ("RemoveQueuedWorkflowPrompt" in request) {
          return { QueuedWorkflowPromptRemoved: { queued_prompt: queued, session } }
        }
        if ("ClearWorkflowPromptQueue" in request) {
          return { WorkflowPromptQueueCleared: { queued_prompts: [queued], session } }
        }
        return { SessionConfigUpdated: { session, config: { version: 1, values: { "workflow.max_turns": "4" } } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1", workflowId: "workflow-1" })
  const flush = await executeShellCommand(parseShellCommand("workflow flush-context false"), context, { client: fake.client })
  const schema = await executeShellCommand(parseShellCommand("workflow run-output-schema final"), context, { client: fake.client })
  const maxTurns = await executeShellCommand(parseShellCommand("workflow max-turns 4"), context, { client: fake.client })
  const nodeConfig = await executeShellCommand(parseShellCommand("workflow node can-complete-run node-1 true"), context, { client: fake.client })
  const watchdogAdd = await executeShellCommand(parseShellCommand("workflow watchdog add endpoint-1 every 1m queue Run it"), context, { client: fake.client })
  const watchdogList = await executeShellCommand(parseShellCommand("workflow watchdog list workflow-1"), context, { client: fake.client })
  const watchdogDisable = await executeShellCommand(parseShellCommand("workflow watchdog disable watchdog-1"), context, { client: fake.client })
  const watchdogRemove = await executeShellCommand(parseShellCommand("workflow watchdog remove watchdog-1"), context, { client: fake.client })
  const queueList = await executeShellCommand(parseShellCommand("workflow queue list"), context, { client: fake.client })
  const queueRemove = await executeShellCommand(parseShellCommand("workflow queue remove prompt-1"), context, { client: fake.client })
  const queueFlush = await executeShellCommand(parseShellCommand("workflow queue flush"), context, { client: fake.client })
  assert.equal(flush.ok, true)
  assert.equal(schema.ok, true)
  assert.equal(maxTurns.ok, true)
  assert.equal(nodeConfig.ok, true)
  assert.equal(watchdogAdd.ok, true)
  assert.match(watchdogAdd.message ?? "", /created workflow watchdog watchdog-1/)
  assert.equal(watchdogList.ok, true)
  assert.match(watchdogList.message ?? "", /watchdog-1 workflow=workflow-1/)
  assert.equal(watchdogDisable.ok, true)
  assert.equal(watchdogRemove.ok, true)
  assert.equal(queueList.ok, true)
  assert.match(queueList.message ?? "", /prompt-1 .*queue=default/)
  assert.equal(queueRemove.ok, true)
  assert.equal(queueFlush.ok, true)
  assert.deepEqual(requests, [
    { SetWorkflowFlushContext: { session_id: "session-1", workflow_ref: "workflow-1", flush_agent_context_before_run: false } },
    { SetWorkflowRunOutputSchema: { session_id: "session-1", workflow_ref: "workflow-1", run_output_schema_ref: "final" } },
    { GetSessionState: { session_id: "session-1" } },
    { UpdateSessionConfig: { session_id: "session-1", attachment_id: "attachment-1", values: { "workflow.max_turns": "4" }, requires_idle: false } },
    { SetWorkflowNodeCanCompleteRun: { session_id: "session-1", workflow_ref: "workflow-1", node_id: "node-1", can_complete_workflow_run: true } },
    { CreateWorkflowWatchdog: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", interval_seconds: 60, invocation_prompt: "Run it", policy: "queue", max_wakeups_configured: false, max_wakeups: null } },
    { ListWorkflowWatchdogs: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { SetWorkflowWatchdogEnabled: { session_id: "session-1", watchdog_ref: "watchdog-1", enabled: false } },
    { RemoveWorkflowWatchdog: { session_id: "session-1", watchdog_ref: "watchdog-1" } },
    { ListWorkflowPromptQueues: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { ListQueuedWorkflowPrompts: { session_id: "session-1" } },
    { RemoveQueuedWorkflowPrompt: { session_id: "session-1", queue_item_ref: "prompt-1" } },
    { ClearWorkflowPromptQueue: { session_id: "session-1", workflow_ref: "workflow-1", queue_ref: "default" } },
  ])
})

test("executeShellCommand creates workflow watchdogs with explicit workflow ref", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const watchdog = makeWorkflowWatchdog()
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { WorkflowWatchdogCreated: { watchdog, workflow, endpoint: workflow.endpoints![0], session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("workflow watchdog add workflow-1 endpoint-1 every 1m skip Run it"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { CreateWorkflowWatchdog: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", interval_seconds: 60, invocation_prompt: "Run it", policy: "skip", max_wakeups_configured: false, max_wakeups: null } },
  ])
})
