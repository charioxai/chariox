import assert from "node:assert/strict"
import { mkdtemp, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { test } from "node:test"

import { activatePublicationEventBindings } from "./publication-event-bindings.js"

test("activates materialized event routes on the independent runtime session", async () => {
  const packageRoot = await mkdtemp(join(tmpdir(), "chariox-server-event-bindings-"))
  const requests: unknown[] = []
  await writeFile(join(packageRoot, "event-bindings.local.json"), JSON.stringify({
    schema_version: 1,
    publication_id: "publication-1",
    destination_environment_id: "environment-1",
    secrets_included: false,
    bindings: [{
      source_binding_id: "source-binding-1",
      generator_id: "dev.chariox.github",
      generator_version: "1.0.0",
      manifest_digest: "sha256:manifest",
      event_type: "pull_request.synchronize",
      event_type_version: 1,
      filter: { repository: "charioxai/drill" },
      requested_scope: "repository:charioxai/drill",
      endpoint_id: "endpoint-1",
      queue_ref: "review",
      reply_mode: "thread",
      action_ids: ["notification.reply"],
      source_environment_id: "source-environment",
      source_revision: 2,
      activation: {
        connection_id: "connection-1",
        environment_id: "environment-1",
        mode: "authorized",
      },
    }],
  }))

  await activatePublicationEventBindings({
    client: {
      send: async (request) => {
        requests.push(request)
        return { WorkflowEventBindingCreated: { binding: { id: "runtime-binding-1" } } }
      },
    },
    packageRoot,
    publicationPackage: {
      schema_version: 1,
      package_version: 4,
      publication_id: "publication-1",
      workflow_id: "workflow-1",
      event_bindings_path: "event-bindings.local.json",
      hooks: [],
    },
    runtimeSessionId: "runtime-session-1",
  })

  assert.deepEqual(requests, [{
    CreateWorkflowEventBinding: {
      session_id: "runtime-session-1",
      publication_ref: "publication-1",
      generator_id: "dev.chariox.github",
      generator_version: "1.0.0",
      manifest_digest: "sha256:manifest",
      connection_id: "connection-1",
      connection_scope: "repository:charioxai/drill",
      event_type: "pull_request.synchronize",
      event_type_version: 1,
      filter: { repository: "charioxai/drill" },
      environment_id: "environment-1",
      queue_ref: "review",
      reply_mode: "thread",
      action_ids: ["notification.reply"],
    },
  }])
})

test("rejects changed destination authorization before contacting the kernel", async () => {
  const packageRoot = await mkdtemp(join(tmpdir(), "chariox-server-event-bindings-invalid-"))
  await writeFile(join(packageRoot, "event-bindings.local.json"), JSON.stringify({
    schema_version: 1,
    publication_id: "publication-1",
    destination_environment_id: "environment-1",
    secrets_included: false,
    bindings: [{
      source_binding_id: "source-binding-1",
      generator_id: "dev.chariox.github",
      generator_version: "1.0.0",
      manifest_digest: "sha256:manifest",
      event_type: "pull_request.synchronize",
      event_type_version: 1,
      filter: null,
      requested_scope: "repository:charioxai/drill",
      endpoint_id: "endpoint-1",
      queue_ref: null,
      reply_mode: "disabled",
      action_ids: [],
      source_environment_id: "source-environment",
      source_revision: 1,
      activation: {
        connection_id: "connection-1",
        environment_id: "foreign-environment",
        mode: "authorized",
      },
    }],
  }))
  let called = false
  await assert.rejects(activatePublicationEventBindings({
    client: {
      send: async () => {
        called = true
        return {}
      },
    },
    packageRoot,
    publicationPackage: {
      schema_version: 1,
      package_version: 4,
      publication_id: "publication-1",
      workflow_id: "workflow-1",
      event_bindings_path: "event-bindings.local.json",
      hooks: [],
    },
    runtimeSessionId: "runtime-session-1",
  }), /unauthorized destination/)
  assert.equal(called, false)
})
