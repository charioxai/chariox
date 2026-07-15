import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
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

test("executeShellCommand deletes workflows through the shared design-op path", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [] })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("ResolveWorkflow" in request) {
          return { WorkflowResolved: { workflow } }
        }
        return { WorkflowDesignOpAccepted: { event: { op: { kind: "workflow_remove", workflow_id: workflow.id } }, session } }
      },
    },
  }
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    sessionId: "session-1",
    workflowId: workflow.id,
  })
  const result = await executeShellCommand(parseShellCommand("workflow delete"), context, {
    client: fake.client,
    clientId: "tui-client-1",
  })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /deleted workflow workflow-1 \(qa\)/)
  assert.deepEqual(result.contextUpdates, { workflowId: undefined, sessionId: "session-1", agentId: "agent-1" })
  assert.deepEqual(requests[0], { ResolveWorkflow: { session_id: "session-1", workflow_ref: "workflow-1" } })
  assert.deepEqual(requests[1], {
    ApplyWorkflowDesignOp: {
      session_id: "session-1",
      origin_client_id: "tui-client-1",
      op_id: (requests[1] as { ApplyWorkflowDesignOp: { op_id: string } }).ApplyWorkflowDesignOp.op_id,
      op: { kind: "workflow_remove", workflow_id: "workflow-1" },
    },
  })
  assert.match((requests[1] as { ApplyWorkflowDesignOp: { op_id: string } }).ApplyWorkflowDesignOp.op_id, /^shell-/)
})

test("executeShellCommand manages workflow registry entries", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-workflow-registry-shell-"))
  try {
    const source = "workflow.define({ alias: \"registered\" })\n"
    await writeFile(join(root, "registered.workflow.js"), source, "utf8")
    await mkdir(join(root, "registered-source", "schemas"), { recursive: true })
    await writeFile(join(root, "registered-source", "workflow.js"), "workflow.define({ alias: \"dir\" })\n", "utf8")
    await writeFile(join(root, "registered-source", "schemas", "final.json"), "{\n  \"type\": \"object\"\n}\n", "utf8")
    await writeFile(join(root, "registered-source", "manifest.json"), "{\n  \"manifest_version\": 1\n}\n", "utf8")

    const entry = {
      name: "dev-team-small",
      source_scope: "workspace",
      source_kind: "single_file",
      source_path: "workflow.js",
      source_sha256: "source-sha256",
      source_bytes: source.length,
      definition_sha256: "definition-sha256",
      created_at_ms: 1_000,
      updated_at_ms: 1_000,
      validation: { ok: true, diagnostics: [] },
      parameters_schema: {
        type: "object",
        properties: {
          bracket_size: { type: "integer", default: 2 },
          dry_run: { type: "boolean", default: false },
        },
        additionalProperties: false,
      },
    }
    const session = makeSession()
    const requests: Record<string, unknown>[] = []
    const fake = {
      client: {
        send: async (request: Record<string, unknown>) => {
          requests.push(request)
          if ("ListWorkflowRegistry" in request) {
            return { WorkflowRegistryListed: { entries: [entry] } }
          }
          if ("GetWorkflowRegistryEntry" in request) {
            return { WorkflowRegistryEntry: { entry } }
          }
          if ("LoadWorkflowRegistryEntry" in request) {
            return { WorkflowRegistryEntryLoaded: { entry, result: { compile: { definition: { workflow: {} }, validation: { ok: true, diagnostics: [] }, logs: "" }, apply: { workflow_id: "workflow-loaded", canvas_layout_applied: true } }, session } }
          }
          if ("RunWorkflowRegistryEntry" in request) {
            return {
              WorkflowRegistryEntryRun: {
                entry,
                result: {
                  apply: { compile: { definition: { workflow: {} }, validation: { ok: true, diagnostics: [] }, logs: "" }, apply: { workflow_id: "workflow-run", canvas_layout_applied: true } },
                  invocation: { kind: "started", workflow_run: makeWorkflowRun(), workflow: makeWorkflow(), endpoint: makeWorkflow().endpoints![0] },
                },
                session,
              },
            }
          }
          if ("DeleteWorkflowRegistryEntry" in request) {
            return { WorkflowRegistryEntryDeleted: { name: "dev-team-small", path: "/repo/.arroba/workflows/dev-team-small" } }
          }
          return { WorkflowRegistryEntryAdded: { entry } }
        },
      },
    }
    const context = createDefaultShellContext({ workspace: root, worktree: root, sessionId: "session-1" })
    const list = await executeShellCommand(parseShellCommand("workflow registry list"), context, { client: fake.client })
    const get = await executeShellCommand(parseShellCommand("workflow registry get dev-team-small"), context, { client: fake.client })
    const addFile = await executeShellCommand(parseShellCommand("workflow registry add dev-team-small registered.workflow.js --workspace"), context, { client: fake.client })
    const addDir = await executeShellCommand(parseShellCommand("workflow registry add dev-team-dir registered-source --user"), context, { client: fake.client })
    const addFromWorkflow = await executeShellCommand(parseShellCommand("workflow registry add-from-workflow copied-team workflow-1 --existing-agents --user"), context, { client: fake.client })
    const load = await executeShellCommand(parseShellCommand("workflow load dev-team-small --inputs-json '{\"mode\":\"fast\"}' --input bracket_size=4 --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const run = await executeShellCommand(parseShellCommand("workflow run dev-team-small --endpoint entry --queue urgent --prompt \"Run it\" --input dry_run=true --input label=smoke --provider-rebinding planner=dev-stub/default"), context, { client: fake.client })
    const deleted = await executeShellCommand(parseShellCommand("workflow registry delete dev-team-small --workspace"), context, { client: fake.client })

    assert.equal(list.ok, true)
    assert.match(list.message ?? "", /dev-team-small scope=workspace/)
    assert.equal(get.ok, true)
    assert.match(get.message ?? "", /workflow registry entry dev-team-small/)
    assert.match(get.message ?? "", /parameters_schema=/)
    assert.equal(addFile.ok, true)
    assert.equal(addDir.ok, true)
    assert.equal(addFromWorkflow.ok, true)
    assert.equal(load.ok, true)
    assert.match(load.message ?? "", /workflow-loaded/)
    assert.equal(run.ok, true)
    assert.match(run.message ?? "", /workflow-run/)
    assert.equal(deleted.ok, true)
    assert.deepEqual(requests, [
      { ListWorkflowRegistry: { session_id: "session-1" } },
      { GetWorkflowRegistryEntry: { session_id: "session-1", name: "dev-team-small" } },
      {
        AddWorkflowRegistryEntry: {
          session_id: "session-1",
          name: "dev-team-small",
          scope: "workspace",
          source: {
            kind: "single_file",
            source,
            source_path: "registered.workflow.js",
          },
          node_path: process.execPath,
        },
      },
      {
        AddWorkflowRegistryEntry: {
          session_id: "session-1",
          name: "dev-team-dir",
          scope: "user",
          source: {
            kind: "source_directory",
            files: [
              { path: "manifest.json", contents: "{\n  \"manifest_version\": 1\n}\n", sha256: sha256("{\n  \"manifest_version\": 1\n}\n") },
              { path: "schemas/final.json", contents: "{\n  \"type\": \"object\"\n}\n", sha256: sha256("{\n  \"type\": \"object\"\n}\n") },
              { path: "workflow.js", contents: "workflow.define({ alias: \"dir\" })\n", sha256: sha256("workflow.define({ alias: \"dir\" })\n") },
            ],
          },
          node_path: process.execPath,
        },
      },
      {
        AddWorkflowRegistryEntryFromWorkflow: {
          session_id: "session-1",
          name: "copied-team",
          workflow_ref: "workflow-1",
          scope: "user",
          agent_mode: "existing_agents",
        },
      },
      {
        LoadWorkflowRegistryEntry: {
          session_id: "session-1",
          name: "dev-team-small",
          parameters: { mode: "fast", bracket_size: 4 },
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
        },
      },
      {
        RunWorkflowRegistryEntry: {
          session_id: "session-1",
          name: "dev-team-small",
          parameters: { dry_run: true, label: "smoke" },
          provider_rebindings: [{ node: "planner", provider: "dev-stub", model: "default" }],
          endpoint: "entry",
          queue_ref: "urgent",
          prompt: "Run it",
        },
      },
      { DeleteWorkflowRegistryEntry: { session_id: "session-1", name: "dev-team-small", scope: "workspace" } },
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

function sha256(contents: string): string {
  return createHash("sha256").update(contents, "utf8").digest("hex")
}

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
        kind: null,
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
    const deploymentContract = {
      schema_version: 1,
      package_id: `sha256:${"a".repeat(64)}`,
    }
    const publicationPackage = {
      schema_version: 1,
      package_version: 3,
      publication_id: publication.id,
      workflow_id: publication.workflow_id,
      deployment_contract: { path: "deployment-contract.json", schema_version: 1 },
    }
    const fake = fakeClient((request) => {
      if ("ExportWorkflowPublicationPackage" in request) {
        return {
          WorkflowPublicationPackageExported: {
            publication,
            package_version: 3,
            package_digest: `sha256:${"b".repeat(64)}`,
            package_archive_base64: Buffer.from("archive").toString("base64"),
            package_files: [
              {
                path: "publication.json",
                content_base64: Buffer.from(JSON.stringify(publicationPackage)).toString("base64"),
                executable: false,
              },
              {
                path: "deployment-contract.json",
                content_base64: Buffer.from(JSON.stringify(deploymentContract)).toString("base64"),
                executable: false,
              },
              {
                path: "run.sh",
                content_base64: Buffer.from("#!/bin/sh\nexit 0\n").toString("base64"),
                executable: true,
              },
            ],
          },
        }
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
      parseShellCommand("workflow publication export publication-1 exported --kernel-url ws://kernel.example --agent-app-json '{\"enabled\":true,\"replicas\":{\"count\":2,\"per_caller_ordering\":true}}' --agent-app-assets-dir agent-app-dist"),
      context,
      { client: fake.client },
    )

    assert.equal(result.ok, true)
    assert.match(result.message ?? "", /exported workflow publication publication-1 .*package v3 sha256:b+/)
    const packageJson = JSON.parse(await readFile(join(root, "exported", "publication.json"), "utf8"))
    assert.equal(packageJson.schema_version, 1)
    assert.equal(packageJson.package_version, 3)
    assert.deepEqual(
      JSON.parse(await readFile(join(root, "exported", "deployment-contract.json"), "utf8")),
      deploymentContract,
    )
    assert.notEqual((await stat(join(root, "exported", "run.sh"))).mode & 0o111, 0)
    assert.deepEqual(fake.requests, [
      {
        ExportWorkflowPublicationPackage: {
          session_id: "session-1",
          publication_ref: "publication-1",
          kernel_url: "ws://kernel.example",
          agent_app: {
            enabled: true,
            replicas: { count: 2, per_caller_ordering: true },
          },
          agent_app_assets_dir: join(root, "agent-app-dist"),
        },
      },
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
      schedules: [],
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

test("executeShellCommand manages advanced workflow settings, schedules, and queue", async () => {
  const workflow = makeWorkflow({ flush_agent_context_before_run: false, run_output_schema_ref: "final" })
  const session = makeSession({ attachment_ids: ["attachment-1"], workflows: [workflow] })
  const node = { id: "node-1", agent_id: "agent-1", can_complete_workflow_run: true, max_turns: 3 }
  const schedule = makeWorkflowWatchdog({ id: "schedule-1" })
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
        if ("CreateWorkflowSchedule" in request) {
          return { WorkflowScheduleCreated: { schedule, workflow, endpoint: workflow.endpoints![0], session } }
        }
        if ("ListWorkflowSchedules" in request) {
          return { WorkflowSchedulesListed: { schedules: [schedule] } }
        }
        if ("SetWorkflowScheduleEnabled" in request) {
          return { WorkflowScheduleUpdated: { schedule: { ...schedule, enabled: false }, session } }
        }
        if ("RemoveWorkflowSchedule" in request) {
          return { WorkflowScheduleRemoved: { schedule, session } }
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
  const scheduleAdd = await executeShellCommand(parseShellCommand("workflow schedule add endpoint-1 --every 1m --overlap queue --prompt Run it"), context, { client: fake.client })
  const scheduleList = await executeShellCommand(parseShellCommand("workflow schedule list workflow-1"), context, { client: fake.client })
  const scheduleDisable = await executeShellCommand(parseShellCommand("workflow schedule disable schedule-1"), context, { client: fake.client })
  const scheduleRemove = await executeShellCommand(parseShellCommand("workflow schedule remove schedule-1"), context, { client: fake.client })
  const queueList = await executeShellCommand(parseShellCommand("workflow queue list"), context, { client: fake.client })
  const queueRemove = await executeShellCommand(parseShellCommand("workflow queue remove prompt-1"), context, { client: fake.client })
  const queueFlush = await executeShellCommand(parseShellCommand("workflow queue flush"), context, { client: fake.client })
  assert.equal(flush.ok, true)
  assert.equal(schema.ok, true)
  assert.equal(maxTurns.ok, true)
  assert.equal(nodeConfig.ok, true)
  assert.equal(scheduleAdd.ok, true)
  assert.match(scheduleAdd.message ?? "", /created workflow schedule schedule-1/)
  assert.equal(scheduleList.ok, true)
  assert.match(scheduleList.message ?? "", /schedule-1 workflow=workflow-1/)
  assert.equal(scheduleDisable.ok, true)
  assert.equal(scheduleRemove.ok, true)
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
    { CreateWorkflowSchedule: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", queue_ref: null, trigger: { kind: "interval", every_seconds: 60 }, invocation_prompt: "Run it", overlap_policy: "queue", max_runs_configured: false, max_runs: null } },
    { ListWorkflowSchedules: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { SetWorkflowScheduleEnabled: { session_id: "session-1", schedule_ref: "schedule-1", enabled: false } },
    { RemoveWorkflowSchedule: { session_id: "session-1", schedule_ref: "schedule-1" } },
    { ListWorkflowPromptQueues: { session_id: "session-1", workflow_ref: "workflow-1" } },
    { ListQueuedWorkflowPrompts: { session_id: "session-1" } },
    { RemoveQueuedWorkflowPrompt: { session_id: "session-1", queue_item_ref: "prompt-1" } },
    { ClearWorkflowPromptQueue: { session_id: "session-1", workflow_ref: "workflow-1", queue_ref: "default" } },
  ])
})

test("executeShellCommand creates workflow schedules with explicit workflow ref", async () => {
  const workflow = makeWorkflow()
  const session = makeSession({ workflows: [workflow] })
  const schedule = makeWorkflowWatchdog({ id: "schedule-1" })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        return { WorkflowScheduleCreated: { schedule, workflow, endpoint: workflow.endpoints![0], session } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const result = await executeShellCommand(parseShellCommand("workflow schedule add workflow-1 endpoint-1 --cron \"15 30 14 * * *\" --tz Europe/Berlin --overlap skip --prompt Run it"), context, { client: fake.client })
  assert.equal(result.ok, true)
  assert.deepEqual(requests, [
    { CreateWorkflowSchedule: { session_id: "session-1", workflow_ref: "workflow-1", endpoint_ref: "endpoint-1", queue_ref: null, trigger: { kind: "cron", expression: "15 30 14 * * *", timezone: "Europe/Berlin" }, invocation_prompt: "Run it", overlap_policy: "skip", max_runs_configured: false, max_runs: null } },
  ])
})
