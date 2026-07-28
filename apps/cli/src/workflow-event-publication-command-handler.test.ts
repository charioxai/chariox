import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession } from "@arroba/kernel-client"
import { handleWorkflowEventPublicationCommand } from "./workflow-event-publication-command-handler.js"

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
              generator_id: "dev.arroba.dummy",
              version: "1.0.0",
              protocol_version: 1,
              name: "Dummy Events",
              summary: "Deterministic events.",
              provider: "Arroba test harness",
              publisher: { id: "dev.arroba", name: "Arroba" },
              operator: { id: "hosted.arroba", name: "Arroba hosted service" },
              verification: "arroba",
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
  assert.match(notices[0] ?? "", /dev\.arroba\.dummy@1\.0\.0/)
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
