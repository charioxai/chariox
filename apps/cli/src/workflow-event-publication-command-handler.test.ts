import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "@chariox/kernel-client"
import {
  handleWorkflowEventPublicationCommand,
  handleWorkflowPublicationCommand,
} from "./workflow-event-publication-command-handler.js"

test("TUI publication handler creates an event-based publication for the selected workflow", async () => {
  const notices: string[] = []
  const requests: Record<string, unknown>[] = []
  const updated = session({
    workflows: [
      { id: "workflow-1", alias: null, nodes: [], edges: [], endpoints: [] },
      { id: "workflow-2", alias: null, nodes: [], edges: [], endpoints: [] },
    ],
  })
  await handleWorkflowPublicationCommand({
    ...deps(notices),
    selectedWorkflowId: () => "workflow-2",
    sendWorkflowEventPublicationRequest: async (request) => {
      requests.push(request)
      return {
        WorkflowPublicationCreated: {
          publication: {
            id: "publication-1",
            alias: "event_publication",
            workflow_id: "workflow-2",
            endpoint_id: "endpoint-1",
            kind: "event_based",
            enabled: true,
          },
          session: updated,
        },
      }
    },
  }, ["create", "endpoint-1", "event_publication", "--kind", "event_based"])

  assert.deepEqual(requests, [{
    CreateWorkflowPublication: {
      session_id: "session-1",
      workflow_ref: "workflow-2",
      endpoint_ref: "endpoint-1",
      expected_workflow_revision: null,
      operation_key: null,
      queue_ref: null,
      alias: "event_publication",
      kind: "event_based",
      route: null,
      methods: [],
      transport: null,
      parser: null,
      input_schema: null,
      trace_exposure: null,
      mode: null,
      sync_timeout_ms: null,
      poll_ms: null,
    },
  }])
  assert.match(notices[0] ?? "", /created workflow publication publication-1/)
})

test("TUI event publication handler renders paged catalog results", async () => {
  const notices: string[] = []
  const requests: Record<string, unknown>[] = []
  await handleWorkflowEventPublicationCommand({
    ...deps(notices),
    sendWorkflowEventPublicationRequest: async (request) => {
      requests.push(request)
      return {
        EventGeneratorCatalogPage: {
          page: {
            services: [{
              schema_version: 1,
              generator_id: "dev.chariox.dummy",
              version: "1.0.0",
              protocol_version: 2,
              name: "Dummy Events",
              summary: "Deterministic events.",
              provider: "Chariox test harness",
              publisher: { id: "dev.chariox", name: "Chariox" },
              operator: { id: "hosted.chariox", name: "Chariox hosted service" },
              verification: "chariox",
              categories: ["testing"],
              installed_count: 0,
              recommended: false,
              availability: "available",
              manifest_digest: `sha256:${"a".repeat(64)}`,
            }],
            next_cursor: "next-page",
            categories: [],
            facets: [],
            stale: false,
          },
        },
      }
    },
  }, ["catalog", "--limit", "1"])

  assert.deepEqual(requests, [{ GetEventGeneratorCatalogLanding: { limit: 1 } }])
  assert.match(notices[0] ?? "", /dev\.chariox\.dummy@1\.0\.0/)
  assert.match(notices[0] ?? "", /next cursor: next-page/)
})

test("TUI event publication handler applies session mutations", async () => {
  const notices: string[] = []
  const applied: RuntimeSession[] = []
  const updated = session({ workflow_event_bindings: [] })
  await handleWorkflowEventPublicationCommand({
    ...deps(notices),
    applySessionState: (value) => applied.push(value),
    sendWorkflowEventPublicationRequest: async () => ({
      WorkflowEventBindingUpdated: {
        binding: {
          id: "binding-1",
          status: "paused",
        },
        session: updated,
      },
    }),
  }, ["pause", "binding-1"])

  assert.deepEqual(applied, [updated])
  assert.deepEqual(notices, ["paused workflow event binding binding-1"])
})

function deps(notices: string[]) {
  return {
    sessionState: () => session(),
    currentWorkspaceTarget: () => "/workspace",
    applySessionState: () => {},
    appendNotice: (message: string) => notices.push(message),
    flashFooter: (message: string) => assert.fail(message),
  }
}

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/workspace",
    worktree_id: "/workspace/worktree",
    focused_agent_id: "agent-1",
    workflows: [{ id: "workflow-1", nodes: [], edges: [], endpoints: [] }],
    agents: [],
    ...overrides,
  } as RuntimeSession
}
