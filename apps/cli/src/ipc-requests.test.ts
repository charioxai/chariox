import assert from "node:assert/strict"
import test from "node:test"

import {
  attachToSessionRequest,
  launchProviderRunRequest,
  submitPromptRequest,
} from "./ipc-requests.js"

test("launchProviderRunRequest normalizes blank effort to null", () => {
  assert.deepEqual(
    launchProviderRunRequest("session-1", "opencode", "default", "gpt-5.4", " ", "agent-a"),
    {
      LaunchProviderRun: {
        session_id: "session-1",
        agent_id: "agent-a",
        adapter_key: "opencode",
        provider: "opencode",
        account_profile: "default",
        model: "gpt-5.4",
        variant: null,
        structured_endpoint: null,
        provider_session_id: null,
        native_tui: false,
      },
    },
  )
})

test("launchProviderRunRequest includes native TUI binding metadata", () => {
  assert.deepEqual(
    launchProviderRunRequest("session-1", "codex", "default", "gpt-5.4", "", "agent-a", {
      structuredEndpoint: "ws://127.0.0.1:45321",
      providerSessionId: "codex-thread-1",
      nativeTui: true,
    }),
    {
      LaunchProviderRun: {
        session_id: "session-1",
        agent_id: "agent-a",
        adapter_key: "codex",
        provider: "codex",
        account_profile: "default",
        model: "gpt-5.4",
        variant: null,
        structured_endpoint: "ws://127.0.0.1:45321",
        provider_session_id: "codex-thread-1",
        native_tui: true,
      },
    },
  )
})

test("launchProviderRunRequest maps Claude modes to the Claude adapter", () => {
  assert.deepEqual(
    launchProviderRunRequest(
      "session-1",
      "claude-headless",
      "default",
      "claude-headless/claude-sonnet-4-6",
      "high",
      "agent-a",
    ).LaunchProviderRun,
    {
      session_id: "session-1",
      agent_id: "agent-a",
      adapter_key: "claude",
      provider: "claude-headless",
      account_profile: "default",
      model: "claude-sonnet-4-6",
      variant: "high",
      structured_endpoint: null,
      provider_session_id: null,
      native_tui: false,
    },
  )
  assert.equal(
    launchProviderRunRequest("session-1", "claude-p", "default", "claude-p/claude-opus-4-7", "", null)
      .LaunchProviderRun.adapter_key,
    "claude",
  )
})

test("attach and submit requests preserve full terminal fields", () => {
  assert.deepEqual(attachToSessionRequest("session-1", "cli-1"), {
    AttachToSession: {
      session_id: "session-1",
      client_id: "cli-1",
      capability_level: "FullTerminal",
    },
  })
  assert.deepEqual(
    submitPromptRequest("session-1", "attachment-1", "agent-a", "hi", [{ url: "file:///a.txt", mime: "text/plain", filename: "a.txt" }]),
    {
      SubmitPrompt: {
        session_id: "session-1",
        attachment_id: "attachment-1",
        target_agent_id: "agent-a",
        prompt: "hi",
        attachments: [{ url: "file:///a.txt", mime: "text/plain", filename: "a.txt" }],
      },
    },
  )
})
