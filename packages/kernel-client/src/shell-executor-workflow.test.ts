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
  WorkflowPublicationTrustedSender,
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
  const publication = makeWorkflowPublication()
  const sender: WorkflowPublicationTrustedSender = {
    sender_id: "sender-1",
    publication_id: publication.id,
    display_name: "partner",
    credential_hash: "hash",
    allowed_transports: ["http"],
    created_at_ms: 0,
  }
  const session = makeSession({ workflows: [makeWorkflow()], workflow_publications: [publication] })
  const fake = fakeClient((request) => {
    if ("CreateWorkflowPublication" in request) {
      return { WorkflowPublicationCreated: { publication, session } }
    }
    if ("CreateWorkflowPublicationPairCode" in request) {
      return {
        WorkflowPublicationPairCodeCreated: {
          pair_code: {
            code: {
              code_id: "pair-1",
              publication_id: publication.id,
              pair_code_hash: "hash",
              created_by_user_id: "local",
              created_at_ms: 0,
              used_count: 0,
            },
            pair_code: "pair-code",
          },
          session,
        },
      }
    }
    if ("RedeemWorkflowPublicationPairCode" in request) {
      return { WorkflowPublicationSenderPaired: { sender_credential: { sender, credential: "sender-secret" }, session } }
    }
    if ("ListWorkflowPublicationSenders" in request) {
      return { WorkflowPublicationSendersListed: { senders: [sender] } }
    }
    if ("RevokeWorkflowPublicationSender" in request) {
      return { WorkflowPublicationSenderRevoked: { sender: { ...sender, revoked_at_ms: 42 }, session } }
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
    parseShellCommand("workflow publication create endpoint-1 public_qa --route /qa --method POST --auth-json '{\"mode\":\"anonymous\"}'"),
    context,
    { client: fake.client },
  )
  const listResult = await executeShellCommand(parseShellCommand("workflow publication list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workflow publication show publication-1"), context, { client: fake.client })
  const disableResult = await executeShellCommand(parseShellCommand("workflow publication disable publication-1"), context, { client: fake.client })
  const pairCodeResult = await executeShellCommand(parseShellCommand("workflow publication pair-code publication-1 --max-uses 1"), context, { client: fake.client })
  const redeemResult = await executeShellCommand(parseShellCommand("workflow publication redeem-code publication-1 pair-code partner"), context, { client: fake.client })
  const sendersResult = await executeShellCommand(parseShellCommand("workflow publication senders publication-1"), context, { client: fake.client })
  const revokeSenderResult = await executeShellCommand(parseShellCommand("workflow publication revoke-sender publication-1 sender-1"), context, { client: fake.client })

  assert.equal(createResult.ok, true)
  assert.match(createResult.message ?? "", /created workflow publication publication-1/)
  assert.deepEqual(createResult.contextUpdates, { sessionId: "session-1", agentId: "agent-1", workflowId: "workflow-1" })
  assert.equal(listResult.ok, true)
  assert.match(listResult.message ?? "", /publication-1 \(public_qa\) workflow=workflow-1 endpoint=endpoint-1 enabled=true route=\/qa methods=POST/)
  assert.equal(showResult.ok, true)
  assert.equal(showResult.format, "json")
  assert.equal(disableResult.ok, true)
  assert.match(disableResult.message ?? "", /disabled workflow publication publication-1/)
  assert.equal(pairCodeResult.ok, true)
  assert.match(pairCodeResult.message ?? "", /pair-code/)
  assert.equal(redeemResult.ok, true)
  assert.match(redeemResult.message ?? "", /sender-secret/)
  assert.equal(sendersResult.ok, true)
  assert.match(sendersResult.message ?? "", /sender-1 \(partner\)/)
  assert.equal(revokeSenderResult.ok, true)
  assert.match(revokeSenderResult.message ?? "", /revoked workflow publication sender sender-1/)
  assert.deepEqual(fake.requests, [
    {
      CreateWorkflowPublication: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        endpoint_ref: "endpoint-1",
        alias: "public_qa",
        route: "/qa",
        methods: ["POST"],
        transport: null,
        auth: { mode: "anonymous" },
        parser: null,
        input_schema: null,
        mode: null,
      },
    },
    { ListWorkflowPublications: { session_id: "session-1" } },
    { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    { DisableWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    { CreateWorkflowPublicationPairCode: { session_id: "session-1", publication_ref: "publication-1", expires_in_ms: null, max_uses: 1 } },
    {
      RedeemWorkflowPublicationPairCode: {
        session_id: "session-1",
        publication_ref: "publication-1",
        pair_code: "pair-code",
        display_name: "partner",
        allowed_transports: ["http"],
        expires_in_ms: null,
      },
    },
    { ListWorkflowPublicationSenders: { session_id: "session-1", publication_ref: "publication-1" } },
    { RevokeWorkflowPublicationSender: { session_id: "session-1", publication_ref: "publication-1", sender_ref: "sender-1" } },
  ])
})

test("executeShellCommand exports a workflow publication gateway package", async () => {
  const root = await mkdtemp(join(tmpdir(), "arroba-publication-export-test-"))
  try {
    const publication = makeWorkflowPublication({
      auth: { mode: "arroba", paired_senders: { enabled: true } },
      mode: "async",
    })
    const fake = fakeClient((request) => {
      if ("GetWorkflowPublication" in request) {
        return { WorkflowPublication: { publication } }
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
    assert.deepEqual(config.auth, { mode: "arroba", paired_senders: { enabled: true } })
    const launcher = await readFile(join(root, "exported", "run.sh"), "utf8")
    assert.match(launcher, /arroba-workflow-gateway/)
    const readme = await readFile(join(root, "exported", "README.md"), "utf8")
    assert.match(readme, /paired sender auth/)
    assert.match(readme, /well-known\/arroba\/publication\/pair/)
    assert.deepEqual(fake.requests, [
      { GetWorkflowPublication: { session_id: "session-1", publication_ref: "publication-1" } },
    ])
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("executeShellCommand manages advanced workflow settings, watchdogs, and queue", async () => {
  const workflow = makeWorkflow({ flush_agent_context_before_run: false, run_output_schema_ref: "final", intermediate_output_schema_ref: "progress" })
  const session = makeSession({ attachment_ids: ["attachment-1"], workflows: [workflow] })
  const node = { id: "node-1", agent_id: "agent-1", can_complete_workflow_run: true, max_turns: 3 }
  const watchdog = makeWorkflowWatchdog()
  const queue = { id: "default", alias: "default", priority: 0, enabled: true, created_at_ms: 0, updated_at_ms: 0 }
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
  assert.match(queueList.message ?? "", /prompt-1 queue=default/)
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
    { ListWorkflowPromptQueues: { session_id: "session-1" } },
    { ListQueuedWorkflowPrompts: { session_id: "session-1" } },
    { RemoveQueuedWorkflowPrompt: { session_id: "session-1", queue_item_ref: "prompt-1" } },
    { ClearWorkflowPromptQueue: { session_id: "session-1", queue_ref: "default" } },
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
