import assert from "node:assert/strict"
import test from "node:test"

import type { ProviderCatalog } from "./provider-catalog.js"
import {
  deriveWaitingRoomActivationDecision,
  deriveWaitingRoomModelSelectionDecision,
  deriveWaitingRoomStateUpdate,
  deriveWaitingRoomVariantSelectionDecision,
} from "./waiting-room-controller.js"
import type { SessionListEntry } from "./sessions.js"
import type { WaitingRoomState } from "./waiting-room.js"

test("deriveWaitingRoomStateUpdate normalizes state and reports preference persistence", () => {
  const currentState = waitingRoomState({
    providerId: "opencode",
    modelId: "openai/gpt-5.4",
    effort: "medium",
  })

  const update = deriveWaitingRoomStateUpdate({
    currentState,
    nextState: waitingRoomState({
      sessionIndex: 7,
      providerId: "opencode",
      modelId: "openai/gpt-5.4",
      effort: "invalid",
    }),
    sessions: [session("session-1"), session("session-2")],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
  })

  assert.equal(update.normalizedState.sessionIndex, 1)
  assert.equal(update.nextProvider, "opencode")
  assert.equal(update.normalizedState.effort, "low")
  assert.equal(update.nextModel, "openai/gpt-5.4")
  assert.equal(update.nextEffort, "low")
  assert.equal(update.shouldPersistProviderPreferences, true)
})

test("deriveWaitingRoomActivationDecision returns create and join decisions from focus", () => {
  const sessions = [session("session-1")]
  const createDecision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({
      focus: "model",
      providerId: "opencode",
      modelId: "anthropic/claude-sonnet-4",
      effort: "medium",
    }),
    sessions,
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
  })
  const joinDecision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({
      focus: "session",
      providerId: "opencode",
      modelId: "openai/gpt-5.4",
      effort: "high",
    }),
    sessions,
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
  })

  assert.deepEqual(createDecision, {
    action: "create",
    launch: {
      provider: "opencode",
      model: "anthropic/claude-sonnet-4",
      effort: "medium",
    },
  })
  assert.deepEqual(joinDecision, {
    action: "join",
    session: sessions[0],
    launch: {
      provider: "opencode",
      model: "openai/gpt-5.4",
      effort: "high",
    },
  })
})

test("deriveWaitingRoomActivationDecision reports an error when join has no sessions", () => {
  const decision = deriveWaitingRoomActivationDecision({
    state: waitingRoomState({
      focus: "session",
    }),
    sessions: [],
    catalog: catalog(),
    currentProvider: "opencode",
    currentModel: "openai/gpt-5.4",
  })

  assert.deepEqual(decision, {
    action: "error",
    message: "no session available to join",
  })
})

test("deriveWaitingRoomModelSelectionDecision validates the selected model and normalizes effort", () => {
  const decision = deriveWaitingRoomModelSelectionDecision({
    modelId: "anthropic/claude-sonnet-4",
    state: waitingRoomState({
      providerId: "opencode",
      modelId: "openai/gpt-5.4",
      effort: "high",
    }),
    sessions: [session("session-1")],
    catalog: catalog(),
    currentProvider: "opencode",
    configuredEffort: "high",
  })

  assert.equal(decision.kind, "success")
  if (decision.kind !== "success") {
    return
  }

  assert.equal(decision.selectedModelId, "anthropic/claude-sonnet-4")
  assert.equal(decision.nextState.modelId, "anthropic/claude-sonnet-4")
  assert.equal(decision.nextState.effort, "medium")
  assert.deepEqual(decision.launch, {
    provider: "opencode",
    model: "anthropic/claude-sonnet-4",
    effort: "medium",
  })
})

test("deriveWaitingRoomVariantSelectionDecision validates variants against the active model", () => {
  const invalidDecision = deriveWaitingRoomVariantSelectionDecision({
    variant: "high",
    currentModelId: "anthropic/claude-sonnet-4",
    currentProviderId: "opencode",
    state: waitingRoomState({
      providerId: "opencode",
      modelId: "anthropic/claude-sonnet-4",
      effort: "medium",
    }),
    sessions: [],
    catalog: catalog(),
  })
  const validDecision = deriveWaitingRoomVariantSelectionDecision({
    variant: "low",
    currentModelId: "openai/gpt-5.4",
    currentProviderId: "opencode",
    state: waitingRoomState({
      providerId: "opencode",
      modelId: "openai/gpt-5.4",
      effort: "high",
    }),
    sessions: [],
    catalog: catalog(),
  })

  assert.deepEqual(invalidDecision, {
    kind: "error",
    message: "unknown variant: high",
  })
  assert.equal(validDecision.kind, "success")
  if (validDecision.kind !== "success") {
    return
  }

  assert.equal(validDecision.selectedVariant, "low")
  assert.equal(validDecision.nextState.effort, "low")
  assert.deepEqual(validDecision.launch, {
    provider: "opencode",
    model: "openai/gpt-5.4",
    effort: "low",
  })
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    providerId: "opencode",
    modelId: "openai/gpt-5.4",
    effort: "high",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

function session(id: string): SessionListEntry {
  return {
    id,
    alias: null,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Created",
    created_at_ms: 1,
    attachment_ids: [],
  }
}

function catalog(): ProviderCatalog {
  return {
    all: [
      {
        id: "codex",
        name: "Codex",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: {
              medium: {},
              high: {},
            },
          },
        },
      },
      {
        id: "openai",
        name: "OpenAI",
        models: {
          "gpt-5.4": {
            id: "gpt-5.4",
            name: "GPT-5.4",
            status: "active",
            variants: {
              low: {},
              high: {},
            },
          },
        },
      },
      {
        id: "anthropic",
        name: "Anthropic",
        models: {
          "claude-sonnet-4": {
            id: "claude-sonnet-4",
            name: "Claude Sonnet 4",
            status: "active",
            variants: {
              medium: {},
            },
          },
        },
      },
    ],
    default: {
      codex: "gpt-5.4",
      openai: "gpt-5.4",
      anthropic: "claude-sonnet-4",
    },
    connected: ["codex", "openai", "anthropic"],
  }
}
