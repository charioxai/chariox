import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeSession,
  WorkflowRun,
} from "./cli-types.js"
import { buildWorkflowInspectorProjection } from "./workflow-inspector-projection.js"

test("workflow inspector projects node instruction editor metadata and callbacks", () => {
  const calls: string[] = []
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession(),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    inspectorMode: "logs",
    nodeInstructionsEditor: {
      workflowId: "workflow-1",
      nodeId: "node-1",
      draft: "draft instructions",
    },
    agentPaneEntries: {},
    updateNodeInstructionsDraft: (draft) => {
      calls.push(`draft:${draft}`)
    },
    setNodeInstructionsInputRef: (editor) => {
      calls.push(editor ? "ref:set" : "ref:clear")
    },
  })

  assert.equal(inspector?.title, "Node Instructions")
  assert.deepEqual(inspector?.meta, [
    "Workflow: workflow-1 (Release)",
    "Node: node-1",
    "Agent: agent-a (Builder)",
  ])
  assert.equal(inspector?.draft, "draft instructions")

  inspector?.onDraftChange?.("updated")
  inspector?.onEditorRef?.(null)
  assert.deepEqual(calls, ["draft:updated", "ref:clear"])
})

test("workflow inspector projects runtime state for the selected workflow node", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      workflow_runs: [
        workflowRun({
          id: "run-old",
          status: "Completed",
          created_at_ms: 100,
        }),
        workflowRun({
          id: "run-new",
          status: "Failed",
          created_at_ms: 200,
          node_runs: [
            {
              id: "node-run-1",
              node_id: "node-1",
              agent_id: "agent-1",
              status: "Failed",
              summary: "Could not build",
              created_at_ms: 210,
              started_at_ms: 211,
              completed_at_ms: 220,
            },
          ],
          failure_events: [
            {
              kind: "Validation",
              source_node_run_id: "node-run-1",
              edge_ids: ["edge-1"],
              message: "schema mismatch",
              timestamp_ms: Date.UTC(2026, 0, 1),
            },
          ],
        }),
      ],
      workflow_watchdogs: [
        {
          id: "watchdog-1",
          workflow_id: "workflow-1",
          endpoint_id: "endpoint-1",
          enabled: true,
          trigger: { kind: "interval", every_seconds: 60 },
          interval_seconds: 60,
          invocation_prompt: "Run checks",
          overlap_policy: "queue",
          policy: "queue",
          runs_started: 0,
          wakeups_executed: 0,
          next_run_at_ms: Date.UTC(2026, 0, 1, 1),
          pending_run: true,
          created_at_ms: 1,
          updated_at_ms: 2,
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    inspectorMode: "logs",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Logs")
  assert.deepEqual(inspector?.meta.slice(0, 5), [
    "Workflow: workflow-1 (Release)",
    "Selected: node node-1",
    "Agent: agent-a (Builder)",
    "Run: run-new",
    "Run status: failed",
  ])
  assert.match(inspector?.body ?? "", /Watchdogs: 1/)
  assert.match(inspector?.body ?? "", /pending: true/)
  assert.match(inspector?.body ?? "", /Selected node failure events/)
  assert.match(inspector?.body ?? "", /schema mismatch/)
})

test("workflow inspector redacts collaborator-owned node agent ids", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      agents: [],
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          nodes: [{ id: "node-1", agent_id: "agent-hidden" }],
          edges: [],
          endpoints: [],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    inspectorMode: "logs",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.deepEqual(inspector?.meta.slice(0, 3), [
    "Workflow: workflow-1 (Release)",
    "Selected: node node-1",
    "Agent: another collaborator's agent",
  ])
  assert.doesNotMatch(inspector?.meta.join("\n") ?? "", /agent-hidden/)
})

test("workflow inspector projects workflow log output", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      workflow_consoles: [
        {
          workflow_id: "workflow-1",
          entries: [
            { timestamp_ms: 1, text: "first\n" },
            { timestamp_ms: 2, text: "second\n" },
          ],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    inspectorMode: "logs",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Logs")
  assert.equal(inspector?.meta[0], "Workflow: workflow-1 (Release)")
  assert.equal(inspector?.meta.at(-1), "Entries: 2")
  assert.match(inspector?.body ?? "", /first\nsecond\n/)
})

test("workflow inspector projects selected node agent trace entries", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession(),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    inspectorMode: "trace",
    nodeInstructionsEditor: null,
    agentPaneEntries: {
      "agent-1": [{ id: 1, role: "assistant", text: "trace output" }],
    },
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Trace")
  assert.equal(inspector?.transcriptAgentId, "agent-1")
  assert.equal(inspector?.transcriptEntries?.[0]?.text, "trace output")
})

test("workflow inspector edit mode describes workflow fields", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          flush_agent_context_before_run: false,
          run_output_schema_ref: "run.schema.json",
          intermediate_output_schema_ref: "handoff.schema.json",
          nodes: [{ id: "node-1", agent_id: "agent-1" }],
          edges: [],
          endpoints: [],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    selectedWorkflowComponent: { kind: "workflow" },
    inspectorMode: "edit",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Edit")
  assert.match(inspector?.body ?? "", /Editable workflow fields/)
  assert.match(inspector?.body ?? "", /run-output-schema: run\.schema\.json/)
  assert.match(inspector?.body ?? "", /intermediate-output-schema: handoff\.schema\.json/)
})

test("workflow inspector edit mode describes edge and endpoint fields", () => {
  const edgeInspector = buildWorkflowInspectorProjection({
    session: baseSession({
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          nodes: [
            { id: "node-1", agent_id: "agent-1" },
            { id: "node-2", agent_id: "agent-2" },
          ],
          edges: [{
            id: "edge-1",
            from_node_id: "node-1",
            to_node_id: "node-2",
            handoff_schema_ref: "edge.schema.json",
            validation_policy: "halt",
          }],
          endpoints: [{ id: "endpoint-1", alias: "Run", entry_node_id: "node-2" }],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    selectedWorkflowComponent: { kind: "edge", id: "edge-1" },
    inspectorMode: "edit",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.match(edgeInspector?.body ?? "", /Editable edge fields/)
  assert.match(edgeInspector?.body ?? "", /handoff-schema: edge\.schema\.json/)
  assert.match(edgeInspector?.body ?? "", /validation-policy: halt/)

  const endpointInspector = buildWorkflowInspectorProjection({
    session: baseSession({
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          nodes: [
            { id: "node-1", agent_id: "agent-1" },
            { id: "node-2", agent_id: "agent-2" },
          ],
          edges: [],
          endpoints: [{ id: "endpoint-1", alias: "Run", entry_node_id: "node-2" }],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    selectedWorkflowComponent: { kind: "endpoint", id: "endpoint-1" },
    inspectorMode: "edit",
    nodeInstructionsEditor: null,
    agentPaneEntries: {},
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.match(endpointInspector?.body ?? "", /Editable endpoint fields/)
  assert.match(endpointInspector?.body ?? "", /alias: Run/)
  assert.match(endpointInspector?.body ?? "", /entry-node: node-2/)
})

test("workflow inspector resolves edge selection to source node trace", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      agents: [
        agent({ id: "agent-1", agent_ref: "agent-a" }),
        agent({ id: "agent-2", agent_ref: "agent-b" }),
      ],
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          nodes: [
            { id: "node-1", agent_id: "agent-1" },
            { id: "node-2", agent_id: "agent-2" },
          ],
          edges: [{ id: "edge-1", from_node_id: "node-1", to_node_id: "node-2" }],
          endpoints: [],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-2",
    selectedWorkflowComponent: { kind: "edge", id: "edge-1" },
    inspectorMode: "trace",
    nodeInstructionsEditor: null,
    agentPaneEntries: {
      "agent-1": [{ id: 1, role: "assistant", text: "source trace" }],
      "agent-2": [{ id: 2, role: "assistant", text: "target trace" }],
    },
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Trace")
  assert.equal(inspector?.meta[1], "Selected: edge edge-1 -> node node-1")
  assert.equal(inspector?.transcriptAgentId, "agent-1")
  assert.equal(inspector?.transcriptEntries?.[0]?.text, "source trace")
})

test("workflow inspector resolves endpoint selection to entry node trace", () => {
  const inspector = buildWorkflowInspectorProjection({
    session: baseSession({
      agents: [
        agent({ id: "agent-1", agent_ref: "agent-a" }),
        agent({ id: "agent-2", agent_ref: "agent-b" }),
      ],
      workflows: [
        {
          id: "workflow-1",
          alias: "Release",
          nodes: [
            { id: "node-1", agent_id: "agent-1" },
            { id: "node-2", agent_id: "agent-2" },
          ],
          edges: [],
          endpoints: [{ id: "endpoint-1", alias: "Run", entry_node_id: "node-2" }],
        },
      ],
    }),
    selectedWorkflowId: "workflow-1",
    selectedWorkflowNodeId: "node-1",
    selectedWorkflowComponent: { kind: "endpoint", id: "endpoint-1" },
    inspectorMode: "trace",
    nodeInstructionsEditor: null,
    agentPaneEntries: {
      "agent-1": [{ id: 1, role: "assistant", text: "old trace" }],
      "agent-2": [{ id: 2, role: "assistant", text: "entry trace" }],
    },
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Trace")
  assert.equal(inspector?.meta[1], "Selected: endpoint endpoint-1 -> node node-2")
  assert.equal(inspector?.transcriptAgentId, "agent-2")
  assert.equal(inspector?.transcriptEntries?.[0]?.text, "entry trace")
})

function baseSession(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "workspace-1",
    worktree_id: "worktree-1",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 1,
    agents: [agent()],
    config_state: { version: 1, values: {} },
    workflows: [
      {
        id: "workflow-1",
        alias: "Release",
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
        edges: [],
        endpoints: [],
      },
    ],
    ...overrides,
  }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: "Builder",
    provider: "codex",
    model: "gpt-5",
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

function workflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: "run-1",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status: "Running",
    invocation_prompt: "Run",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
    ...overrides,
  }
}
