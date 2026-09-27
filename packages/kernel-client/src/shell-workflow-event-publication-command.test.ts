import assert from "node:assert/strict"
import test from "node:test"

import { createDefaultShellContext } from "./shell-core.js"
import { executeWorkflowEventPublicationCommand } from "./shell-workflow-event-publication-command.js"

test("workflow event publication command browses a bounded catalog", async () => {
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
              publisher: { id: "chariox", name: "Chariox" },
              operator: { id: "chariox", name: "Chariox" },
              verification: "chariox",
              manifest_digest: "sha256:abc",
              protocol_version: 3,
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
      return {}
    },
  }
  const context = createDefaultShellContext({ sessionId: "session-1" })

  const catalog = await executeWorkflowEventPublicationCommand(
    ["catalog", "pull", "request", "--category", "developer-tools", "--limit", "12"],
    context,
    client,
  )

  assert.equal(catalog.ok, true)
  assert.match(catalog.message ?? "", /github@1\.0\.0/)
  assert.match(catalog.message ?? "", /next cursor: cursor-2/)
  assert.deepEqual(requests[0], {
    SearchEventGeneratorCatalog: {
      query: "pull request",
      category: "developer-tools",
      verification: null,
      cursor: null,
      limit: 12,
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
              generator_id: "dev.chariox.dummy",
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
    ["install", "dev.chariox.dummy"],
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
        generator_id: "dev.chariox.dummy",
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

test("workflow event publication command pages connections without a generator filter", async () => {
  const requests: Record<string, unknown>[] = []
  const client = {
    send: async (request: Record<string, unknown>) => {
      requests.push(request)
      return {
        EventConnectionsPage: {
          page: {
            connections: [],
            next_cursor: null,
          },
        },
      }
    },
  }

  const result = await executeWorkflowEventPublicationCommand(
    ["connections", "--cursor", "cursor-2", "--limit", "5"],
    createDefaultShellContext({ sessionId: "session-1" }),
    client,
  )

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{
    ListEventConnections: {
      generator_id: null,
      cursor: "cursor-2",
      limit: 5,
    },
  }])
})

test("workflow event publication command reports event delivery status", async () => {
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
      return {}
    },
  }
  const context = createDefaultShellContext({ sessionId: "session-1" })

  const status = await executeWorkflowEventPublicationCommand(["status"], context, client)

  assert.match(status.message ?? "", /connected; active App routes=1/)
  assert.deepEqual(requests, [{ GetEventDeliveryStatus: {} }])
})

test("workflow event publication command no longer creates direct bindings", async () => {
  for (const action of ["list", "attach", "pause", "transfer", "test"]) {
    const result = await executeWorkflowEventPublicationCommand(
      [action, "event-binding-1"],
      createDefaultShellContext({ sessionId: "session-1" }),
      { send: async () => { throw new Error("must not reach the kernel") } },
    )
    assert.equal(result.ok, false)
    assert.match(result.message ?? "", /usage: workflow trigger event catalog\|category/)
  }
})
