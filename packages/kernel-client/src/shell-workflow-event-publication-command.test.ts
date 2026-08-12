import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowEventBinding } from "./kernel-types.js"
import { createDefaultShellContext } from "./shell-core.js"
import { executeWorkflowEventPublicationCommand } from "./shell-workflow-event-publication-command.js"

const binding: WorkflowEventBinding = {
  id: "event-binding-1",
  publication_id: "publication-1",
  generator_id: "github",
  generator_version: "1.0.0",
  manifest_digest: "sha256:abc",
  connection_id: "connection-1",
  connection_scope: "repo:arroba/arroba",
  event_type: "pull_request.opened",
  event_type_version: 1,
  filter: { repository: "arroba/arroba" },
  event_interest_key: "sha256:interest",
  environment_id: "kernel-1",
  endpoint_id: "endpoint-1",
  queue_ref: "default",
  revision: 1,
  status: "active",
  created_at_ms: 1,
  updated_at_ms: 1,
}

test("workflow event publication command browses a bounded catalog and binds an event", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("SearchEventGeneratorCatalog" in request) {
        return {
          EventGeneratorCatalogPage: { page: {
            services: [{
              schema_version: 1,
              generator_id: "github",
              version: "1.0.0",
              name: "GitHub",
              summary: "Repository events",
              provider: "GitHub",
              publisher: { id: "arroba", name: "Arroba" },
              operator: { id: "arroba", name: "Arroba" },
              verification: "arroba",
              manifest_digest: "sha256:abc",
              protocol_version: 1,
              categories: ["developer-tools"],
              installed_count: 10,
              recommended: true,
              availability: "development_preview",
            }],
            next_cursor: "cursor-2",
            categories: [],
            facets: [],
            stale: false,
          } },
        }
      }
      return {
        WorkflowEventBindingCreated: {
          binding,
          session: { id: "session-1" },
        },
      }
    },
  }
  const context = createDefaultShellContext({ sessionId: "session-1" })

  const catalog = await executeWorkflowEventPublicationCommand(
    ["catalog", "pull", "request", "--category", "developer-tools", "--limit", "12"],
    context,
    client,
  )
  const subscribed = await executeWorkflowEventPublicationCommand(
    [
      "bind",
      "publication-1",
      "github",
      "pull_request.opened",
      "--generator-version",
      "1.0.0",
      "--manifest-digest",
      "sha256:abc",
      "--connection",
      "connection-1",
      "--scope",
      "repo:arroba/arroba",
      "--filter-json",
      "{\"repository\":\"arroba/arroba\"}",
    ],
    context,
    client,
  )

  assert.equal(catalog.ok, true)
  assert.match(catalog.message ?? "", /github@1\.0\.0/)
  assert.match(catalog.message ?? "", /next cursor: cursor-2/)
  assert.equal(subscribed.ok, true)
  assert.match(subscribed.message ?? "", /subscribed event-binding-1/)
  assert.deepEqual(requests[0], {
    SearchEventGeneratorCatalog: {
      query: "pull request",
      category: "developer-tools",
      verification: null,
      cursor: null,
      limit: 12,
    },
  })
  assert.deepEqual(requests[1], {
    CreateWorkflowEventBinding: {
      session_id: "session-1",
      publication_ref: "publication-1",
      generator_id: "github",
      generator_version: "1.0.0",
      manifest_digest: "sha256:abc",
      connection_id: "connection-1",
      connection_scope: "repo:arroba/arroba",
      event_type: "pull_request.opened",
      event_type_version: 1,
      filter: { repository: "arroba/arroba" },
      environment_id: null,
      queue_ref: null,
    },
  })
})

test("workflow event publication command authorizes and enumerates provider resources", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("InstallEventConnection" in request) {
        return {
          EventConnectionAuthorizationStarted: {
            authorization: {
              authorization_id: "authorization-1",
              generator_id: "dev.arroba.dummy",
              status: "ready",
              connection_id: "local-dummy",
              created_at_ms: 1,
            },
          },
        }
      }
      return {
        EventConnectionResourcesPage: {
          page: {
            resources: [{
              id: "default",
              name: "Default test environment",
              kind: "test_scope",
              connection_scope: "default",
            }],
            next_cursor: null,
          },
        },
      }
    },
  }
  const context = createDefaultShellContext({ sessionId: "session-1" })

  const authorization = await executeWorkflowEventPublicationCommand(
    ["install", "dev.arroba.dummy"],
    context,
    client,
  )
  const resources = await executeWorkflowEventPublicationCommand(
    ["resources", "local-dummy", "default", "--limit", "5"],
    context,
    client,
  )

  assert.match(authorization.message ?? "", /connection=local-dummy/)
  assert.match(resources.message ?? "", /Default test environment/)
  assert.deepEqual(requests, [
    {
      InstallEventConnection: {
        generator_id: "dev.arroba.dummy",
        return_url: null,
      },
    },
    {
      ListEventConnectionResources: {
        connection_id: "local-dummy",
        query: "default",
        cursor: null,
        limit: 5,
      },
    },
  ])
})

test("workflow event publication command maps lifecycle actions to shared kernel requests", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      if ("GetEventDeliveryStatus" in request) {
        return {
          EventDeliveryStatus: { status: {
            configured: true,
            connected: true,
            aeds_url: "wss://events.example.test",
            active_route_count: 1,
          } },
        }
      }
      return {
        WorkflowEventBindingUpdated: {
          binding: { ...binding, status: "paused" },
          session: { id: "session-1" },
        },
      }
    },
  }
  const context = createDefaultShellContext({ sessionId: "session-1" })

  const paused = await executeWorkflowEventPublicationCommand(
    ["pause", "event-binding-1"],
    context,
    client,
  )
  const status = await executeWorkflowEventPublicationCommand(["status"], context, client)

  assert.equal(paused.ok, true)
  assert.match(status.message ?? "", /connected/)
  assert.deepEqual(requests, [
    {
      SetWorkflowEventBindingStatus: {
        session_id: "session-1",
        binding_id: "event-binding-1",
        status: "paused",
      },
    },
    { GetEventDeliveryStatus: {} },
  ])
})
