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
    inspectorMode: "runtime",
    nodeInstructionsEditor: {
      workflowId: "workflow-1",
      nodeId: "node-1",
      draft: "draft instructions",
    },
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
          interval_seconds: 60,
          invocation_prompt: "Run checks",
          policy: "queue",
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
    inspectorMode: "runtime",
    nodeInstructionsEditor: null,
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Runtime")
  assert.deepEqual(inspector?.meta, [
    "Workflow: workflow-1 (Release)",
    "Selected node: node-1",
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
    inspectorMode: "runtime",
    nodeInstructionsEditor: null,
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.deepEqual(inspector?.meta.slice(0, 3), [
    "Workflow: workflow-1 (Release)",
    "Selected node: node-1",
    "Agent: another collaborator's agent",
  ])
  assert.doesNotMatch(inspector?.meta.join("\n") ?? "", /agent-hidden/)
})

test("workflow inspector projects terminal console output", () => {
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
    inspectorMode: "terminal",
    nodeInstructionsEditor: null,
    updateNodeInstructionsDraft: () => {},
    setNodeInstructionsInputRef: () => {},
  })

  assert.equal(inspector?.title, "Workflow Terminal")
  assert.deepEqual(inspector?.meta, [
    "Workflow: workflow-1 (Release)",
    "Entries: 2",
  ])
  assert.equal(inspector?.body, "first\nsecond\n")
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
