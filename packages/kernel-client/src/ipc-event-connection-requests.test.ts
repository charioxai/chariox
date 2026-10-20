import assert from "node:assert/strict"
import test from "node:test"

import {
  getEventConnectionRequest,
  installEventConnectionRequest,
  listEventConnectionDependenciesRequest,
  listEventConnectionResourcesRequest,
  listEventConnectionsRequest,
  observeEventConnectionAuthorizationRequest,
  refreshEventConnectionRequest,
  reconnectEventConnectionRequest,
  removeEventConnectionRequest,
  testEventConnectionRequest,
  listEventConnectionEventsRequest,
  replayEventConnectionEventRequest,
} from "./ipc-event-publication-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"

test("event connection lifecycle requests match protocol 261", () => {

  assert.deepEqual(listEventConnectionsRequest({ generatorId: "dev.chariox.github" }), {
    ListEventConnections: {
      generator_id: "dev.chariox.github",
      cursor: null,
      limit: 20,
    },
  })
  assert.deepEqual(getEventConnectionRequest("connection-1"), {
    GetEventConnection: { connection_id: "connection-1" },
  })
  assert.deepEqual(installEventConnectionRequest("dev.chariox.github", "https://example.test"), {
    InstallEventConnection: {
      generator_id: "dev.chariox.github",
      return_url: "https://example.test",
    },
  })
  assert.deepEqual(observeEventConnectionAuthorizationRequest("authorization-1"), {
    ObserveEventConnectionAuthorization: { authorization_id: "authorization-1" },
  })
  assert.deepEqual(refreshEventConnectionRequest("connection-1"), {
    RefreshEventConnection: { connection_id: "connection-1" },
  })
  assert.deepEqual(testEventConnectionRequest("connection-1", "pull_request.opened"), {
    TestEventConnection: {
      connection_id: "connection-1",
      event_type: "pull_request.opened",
    },
  })
  assert.deepEqual(listEventConnectionEventsRequest("connection-1", 20), {
    ListEventConnectionEvents: { connection_id: "connection-1", limit: 20 },
  })
  assert.deepEqual(replayEventConnectionEventRequest("connection-1", "delivery-1"), {
    ReplayEventConnectionEvent: { connection_id: "connection-1", delivery_id: "delivery-1" },
  })
  assert.deepEqual(reconnectEventConnectionRequest("connection-1", "https://example.test"), {
    ReconnectEventConnection: {
      connection_id: "connection-1",
      return_url: "https://example.test",
    },
  })
  assert.deepEqual(listEventConnectionResourcesRequest("connection-1", { query: "chariox" }), {
    ListEventConnectionResources: {
      connection_id: "connection-1",
      query: "chariox",
      cursor: null,
      limit: 20,
    },
  })
  assert.deepEqual(listEventConnectionDependenciesRequest("connection-1"), {
    ListEventConnectionDependencies: { connection_id: "connection-1" },
  })
  assert.deepEqual(removeEventConnectionRequest("connection-1", true), {
    RemoveEventConnection: { connection_id: "connection-1", confirm: true },
  })
})
