import assert from "node:assert/strict"
import test from "node:test"

import {
  attachToSessionRequest,
  getSessionHistoryRequest,
  launchProviderRunRequest,
  submitPromptRequest,
} from "./ipc-requests.js"

test("getSessionHistoryRequest includes paging and agent targeting", () => {
  assert.deepEqual(
    getSessionHistoryRequest("session-1", 2, 1000, { before_entry_index: 12, before_entry_char_offset: 4 }, "agent-a"),
    {
      GetSessionHistory: {
        session_id: "session-1",
        agent_id: "agent-a",
        round_count: 2,
        max_chars: 1000,
        before_entry_index: 12,
        before_entry_char_offset: 4,
      },
    },
  )
})

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
      },
    },
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
