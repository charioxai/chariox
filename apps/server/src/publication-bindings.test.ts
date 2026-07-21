import assert from "node:assert/strict"
import test from "node:test"

import { resolvePublicationProviderModelBindings } from "./publication-bindings.js"
import type { WorkflowPublicationSnapshot } from "./publication-types.js"

test("publication bindings resolve the default model sentinel without prompting", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "codex",
      model: "default",
      effort: null,
    }],
  } as unknown as WorkflowPublicationSnapshot

  const resolved = await resolvePublicationProviderModelBindings(
    snapshot,
    "/tmp/arroba-publication-default-model-bindings-does-not-exist.json",
    {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: {
          catalog: {
            all: [{ id: "codex", models: { "gpt-5.6-sol": {} } }],
          },
        },
      }) as T,
    },
    { promptReplacement: false },
  )

  assert.equal(resolved.snapshot.agents?.[0]?.model, null)
  assert.equal(resolved.changed, false)
})

test("publication bindings keep OpenCode models provider-qualified after catalog validation", async () => {
  for (const capturedModel of ["gpt-5.2", "opencode/gpt-5.2"]) {
    const snapshot = {
      workflow: {
        id: "workflow-1",
        nodes: [{ id: "node-1", agent_id: "agent-1" }],
      },
      agents: [{
        id: "agent-1",
        provider: "opencode",
        model: capturedModel,
        effort: null,
      }],
    } as unknown as WorkflowPublicationSnapshot

    const resolved = await resolvePublicationProviderModelBindings(
      snapshot,
      `/tmp/arroba-publication-opencode-${capturedModel.replaceAll("/", "-")}-bindings-does-not-exist.json`,
      {
        send: async <T>(): Promise<T> => ({
          ProviderCatalog: {
            catalog: {
              all: [{ id: "opencode", models: { "gpt-5.2": {} } }],
            },
          },
        }) as T,
      },
      { promptReplacement: false },
    )

    assert.equal(resolved.snapshot.agents?.[0]?.model, "opencode/gpt-5.2")
    assert.equal(resolved.changed, false)
  }
})

test("publication bindings validate the Claude family against runtime adapter catalogs", async () => {
  const snapshot = {
    workflow: {
      id: "workflow-1",
      nodes: [{ id: "node-1", agent_id: "agent-1" }],
    },
    agents: [{
      id: "agent-1",
      provider: "claude",
      model: "claude-sonnet-5",
      effort: null,
    }],
  } as unknown as WorkflowPublicationSnapshot

  const resolved = await resolvePublicationProviderModelBindings(
    snapshot,
    "/tmp/arroba-publication-claude-family-bindings-does-not-exist.json",
    {
      send: async <T>(): Promise<T> => ({
        ProviderCatalog: {
          catalog: {
            all: [{ id: "claude-headless", models: { "claude-sonnet-5": {} } }],
          },
        },
      }) as T,
    },
    { promptReplacement: false },
  )

  assert.equal(resolved.snapshot.agents?.[0]?.provider, "claude")
  assert.equal(resolved.snapshot.agents?.[0]?.model, "claude-sonnet-5")
  assert.equal(resolved.changed, false)
})
