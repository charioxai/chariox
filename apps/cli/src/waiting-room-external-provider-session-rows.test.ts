import assert from "node:assert/strict"
import test from "node:test"

import type { ExternalProviderSessionRecord } from "./cli-types.js"
import {
  waitingRoomExternalProviderSessionRows,
  waitingRoomExternalProviderSessions,
} from "./waiting-room-external-provider-session-rows.js"
import type { WaitingRoomState } from "./waiting-room-types.js"

test("unattached agents render as selectable waiting-room rows with load older action", () => {
  const rows = waitingRoomExternalProviderSessionRows(
    waitingRoomState({ focus: "external-session", externalSessionIndex: 0 }),
    {
      externalProviderSessions: [
        externalSession({
          external_session_id: "codex:abc",
          provider: "codex",
          title: "Review payment flow",
          last_modified_at_ms: Date.UTC(2026, 0, 2, 10, 30),
        }),
      ],
      externalProviderSessionsHasMore: true,
      externalProviderSessionsNextCursor: "cursor-2",
    },
    {
      inventoryLoading: false,
      loadingText: "loading",
      titleWidth: 28,
    },
  )

  assert.deepEqual(rows.map((row) => row.id), [
    "external-provider-session-header",
    "external-session:codex:abc",
    "external-provider-session-more",
  ])
  assert.equal(rows[1]?.title, "Review payment flow")
  assert.equal(rows[1]?.selectable, true)
  assert.equal(rows[1]?.focused, true)
  assert.equal(rows[2]?.title, "Load older unattached agents")
  assert.equal(rows[2]?.selectable, true)
})

test("unattached agents show a loading row while inventory is pending", () => {
  const rows = waitingRoomExternalProviderSessionRows(
    waitingRoomState(),
    {},
    {
      inventoryLoading: true,
      loadingText: "loading unattached agents",
      titleWidth: 28,
    },
  )

  assert.deepEqual(rows, [{
    id: "external-provider-sessions-loading",
    title: "Unattached agents",
    value: "loading unattached agents",
    titleWidth: 28,
    indent: 1,
    focused: false,
    selectable: false,
    scrollbar: "",
  }])
})

test("unattached agents are projected newest first with normalized title fallback", () => {
  const sessions = waitingRoomExternalProviderSessions({
    externalProviderSessions: [
      externalSession({
        external_session_id: "codex:older",
        provider_session_id: "older",
        title: "  ",
        first_prompt_preview: "  Review checkout  ",
        last_modified_at_ms: 100,
      }),
      externalSession({
        external_session_id: "codex:newer",
        provider_session_id: "newer",
        title: "  New task  ",
        first_prompt_preview: "ignored",
        last_modified_at_ms: 200,
      }),
    ],
  })

  assert.deepEqual(sessions.map((session) => session.external_session_id), [
    "codex:newer",
    "codex:older",
  ])

  const rows = waitingRoomExternalProviderSessionRows(
    waitingRoomState({ focus: "external-session", externalSessionIndex: 1 }),
    { externalProviderSessions: sessions },
    { inventoryLoading: false, loadingText: "loading", titleWidth: 28 },
  )

  assert.equal(rows[1]?.title, "New task")
  assert.equal(rows[2]?.title, "Review checkout")
  assert.equal(rows[2]?.focused, true)
})

function waitingRoomState(overrides: Partial<WaitingRoomState> = {}): WaitingRoomState {
  return {
    focus: "new",
    sessionIndex: 0,
    machineIndex: 0,
    remoteKernelIndex: 0,
    terminalIndex: 0,
    worktreeSelectionId: "existing:/workspace",
    workspaceLiveSyncMode: "off",
    providerId: "opencode",
    modelId: "opencode/gpt-5.4",
    effort: "high",
    themeId: "opencode",
    introStep: 0,
    keyState: { up: false, down: false, left: false, right: false },
    ...overrides,
  }
}

function externalSession(overrides: Partial<ExternalProviderSessionRecord> = {}): ExternalProviderSessionRecord {
  return {
    external_session_id: "codex:abc",
    provider: "codex",
    provider_session_id: "abc",
    title: "External task",
    title_source: "provider",
    first_prompt_preview: "External task",
    created_at_ms: 1,
    last_modified_at_ms: 2,
    capabilities: {
      can_read_history: true,
    },
    ...overrides,
  }
}
