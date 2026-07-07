import assert from "node:assert/strict"
import test from "node:test"

import {
  externalProviderSessionPage,
  externalProviderSessionPageHasMore,
  externalProviderSessionPageNextCursor,
  externalProviderSessionPageSessions,
  externalProviderSessionPageState,
  externalProviderSessionAtSelection,
  externalProviderSessionModifiedLabel,
  externalProviderSessionModeLabel,
  externalProviderSessionSelectionIndex,
  externalProviderSessionsSorted,
  externalProviderSessionTitle,
  mergeExternalProviderSessionsSorted,
  mergeExternalProviderSessions,
  type ExternalProviderSessionRecord,
} from "./external-provider-sessions.js"

test("external provider sessions are projected newest first with deterministic fallback order", () => {
  const sessions = externalProviderSessionsSorted([
    externalSession("codex:older", 100, {
      provider_session_id: "older",
      title: "  ",
      first_prompt_preview: "  Review checkout  ",
    }),
    externalSession("codex:newer", 200, {
      provider_session_id: "newer",
      title: "  New task  ",
      first_prompt_preview: "ignored",
    }),
    externalSession("opencode:zeta", 100, {
      provider: "opencode",
      provider_session_id: "zeta",
      title: "Zeta",
    }),
    externalSession("codex:alpha", 100, {
      provider_session_id: "alpha",
      title: "Alpha",
    }),
  ])

  assert.deepEqual(sessions.map((session) => session.external_session_id), [
    "codex:newer",
    "codex:alpha",
    "codex:older",
    "opencode:zeta",
  ])
  assert.equal(externalProviderSessionTitle(sessions[0]!), "New task")
  assert.equal(externalProviderSessionTitle(sessions[2]!), "Review checkout")
})

test("external provider sessions treat invalid modification times as missing for deterministic order", () => {
  const sessions = externalProviderSessionsSorted([
    {
      ...externalSession("opencode:zeta", 100, {
        provider: "opencode",
        provider_session_id: "zeta",
      }),
      last_modified_at_ms: Number.NaN,
    },
    externalSession("codex:alpha", 0, {
      provider_session_id: "alpha",
    }),
    externalSession("codex:recent", 20, {
      provider_session_id: "recent",
    }),
  ])

  assert.deepEqual(sessions.map((session) => session.external_session_id), [
    "codex:recent",
    "codex:alpha",
    "opencode:zeta",
  ])
})

test("external provider session title falls back through prompt, provider id, and external id", () => {
  assert.equal(externalProviderSessionTitle(externalSession("codex:title", 100, {
    title: "  Title  ",
    first_prompt_preview: "Prompt",
  })), "Title")
  assert.equal(externalProviderSessionTitle(externalSession("codex:prompt", 100, {
    title: " ",
    first_prompt_preview: "  Prompt  ",
  })), "Prompt")
  assert.equal(externalProviderSessionTitle(externalSession("codex:provider", 100, {
    title: null,
    first_prompt_preview: null,
    provider_session_id: "provider-session",
  })), "provider-session")
  assert.equal(externalProviderSessionTitle({
    ...externalSession("codex:external", 100),
    provider_session_id: "",
    title: "",
    first_prompt_preview: "",
  }), "codex:external")
})

test("external provider session labels normalize mode and modification time", () => {
  const session = externalSession("codex:time", Date.UTC(2026, 0, 2, 10, 30))
  assert.equal(externalProviderSessionModeLabel(session), "observed")
  assert.equal(externalProviderSessionModifiedLabel(session), "2026-01-02 10:30")
  assert.equal(externalProviderSessionModifiedLabel(session, { utcSuffix: true }), "2026-01-02 10:30 UTC")
  assert.equal(externalProviderSessionModifiedLabel(externalSession("codex:missing", 0)), "-")
  assert.equal(externalProviderSessionModifiedLabel({
    ...externalSession("codex:invalid", 100),
    last_modified_at_ms: Number.NaN,
  }), "-")
})

test("external provider session page projection accepts kernel and client page shapes", () => {
  const older = externalSession("codex:older", 100)
  const newer = externalSession("codex:newer", 200)

  assert.deepEqual(externalProviderSessionPage({
    sessions: [older, newer],
    has_more: true,
    next_cursor: "kernel-cursor",
  }), {
    sessions: [newer, older],
    hasMore: true,
    nextCursor: "kernel-cursor",
  })

  assert.deepEqual(externalProviderSessionPageSessions({
    externalProviderSessions: [older, newer],
  }).map((session) => session.external_session_id), ["codex:newer", "codex:older"])
  assert.deepEqual(externalProviderSessionPageSessions({
    sessions: [newer],
    externalProviderSessions: [older],
  }).map((session) => session.external_session_id), ["codex:older"])
  assert.deepEqual(externalProviderSessionPageState({
    hasMore: true,
    nextCursor: " client-cursor ",
  }), { hasMore: true, nextCursor: "client-cursor" })
  assert.deepEqual(externalProviderSessionPageState({
    externalProviderSessionsHasMore: true,
    externalProviderSessionsNextCursor: " ",
  }), { hasMore: true, nextCursor: null })
  assert.deepEqual(externalProviderSessionPageState({
    externalProviderSessionsHasMore: true,
    externalProviderSessionsNextCursor: "remote-cursor",
  }), { hasMore: true, nextCursor: "remote-cursor" })
  assert.deepEqual(externalProviderSessionPageState({
    hasMore: false,
    nextCursor: "stale-cursor",
  }), { hasMore: false, nextCursor: null })
  assert.equal(externalProviderSessionPageHasMore(null), false)
  assert.equal(externalProviderSessionPageNextCursor({}), null)
})

test("external provider session merge dedupes by external session id with newest metadata winning", () => {
  const merged = mergeExternalProviderSessions(
    [
      externalSession("external-1", 100, { title: "first" }),
      externalSession("external-2", 200),
    ],
    [
      externalSession("external-1", 300, { title: "duplicate" }),
      externalSession("external-3", 400),
    ],
  )

  assert.deepEqual(merged.map((session) => ({
    id: session.external_session_id,
    title: session.title,
  })), [
    { id: "external-1", title: "duplicate" },
    { id: "external-2", title: "external-2" },
    { id: "external-3", title: "external-3" },
  ])
})

test("external provider session merge keeps first metadata when modification times tie", () => {
  const merged = mergeExternalProviderSessions(
    [externalSession("external-1", 100, { title: "first" })],
    [externalSession("external-1", 100, { title: "tie" })],
  )

  assert.deepEqual(merged.map((session) => ({
    id: session.external_session_id,
    title: session.title,
  })), [
    { id: "external-1", title: "first" },
  ])
})

test("external provider session merge preserves rich metadata from older records", () => {
  const merged = mergeExternalProviderSessions(
    [
      externalSession("external-1", 100, {
        title: "Original task",
        title_source: "first_prompt",
        first_prompt_preview: "Review the runtime state",
        created_at_ms: 10,
        worktree_path: "/worktree/old",
        account_profile: "default",
        capabilities: { can_read_history: true },
      }),
    ],
    [
      externalSession("external-1", 300, {
        title: " ",
        title_source: null,
        first_prompt_preview: null,
        created_at_ms: null,
        worktree_path: "",
        account_profile: null,
        capabilities: {},
      }),
    ],
  )

  assert.deepEqual(merged.map((session) => ({
    id: session.external_session_id,
    title: session.title,
    titleSource: session.title_source,
    firstPromptPreview: session.first_prompt_preview,
    createdAtMs: session.created_at_ms,
    modifiedAtMs: session.last_modified_at_ms,
    worktreePath: session.worktree_path,
    accountProfile: session.account_profile,
    canReadHistory: session.capabilities?.can_read_history,
  })), [
    {
      id: "external-1",
      title: "Original task",
      titleSource: "first_prompt",
      firstPromptPreview: "Review the runtime state",
      createdAtMs: 10,
      modifiedAtMs: 300,
      worktreePath: "/worktree/old",
      accountProfile: "default",
      canReadHistory: true,
    },
  ])
})

test("external provider session sorted merge dedupes and projects final order", () => {
  const merged = mergeExternalProviderSessionsSorted(
    [
      externalSession("codex:old", 100),
      externalSession("opencode:zeta", 200, {
        provider: "opencode",
        provider_session_id: "zeta",
      }),
    ],
    [
      externalSession("codex:old", 300, { title: "updated" }),
      externalSession("claude:recent", 400, { provider: "claude" }),
    ],
  )

  assert.deepEqual(merged.map((session) => ({
    id: session.external_session_id,
    title: session.title,
  })), [
    { id: "claude:recent", title: "claude:recent" },
    { id: "codex:old", title: "updated" },
    { id: "opencode:zeta", title: "opencode:zeta" },
  ])
})

test("external provider session selection prefers ids and clamps index fallback", () => {
  const sessions = [
    externalSession("external-1", 100),
    externalSession("external-2", 200),
  ]
  assert.equal(externalProviderSessionSelectionIndex(sessions, {
    selectedExternalProviderSessionId: "external-2",
    selectedExternalProviderSessionIndex: 0,
  }), 1)
  assert.equal(externalProviderSessionAtSelection(sessions, {
    selectedExternalProviderSessionId: "external-2",
    selectedExternalProviderSessionIndex: 0,
  })?.external_session_id, "external-2")
  assert.equal(externalProviderSessionSelectionIndex(sessions, {
    selectedExternalProviderSessionId: "external-missing",
    selectedExternalProviderSessionIndex: 8,
  }), 1)
  assert.equal(externalProviderSessionSelectionIndex(sessions, {
    selectedExternalProviderSessionIndex: -8,
  }), 0)
  assert.equal(externalProviderSessionSelectionIndex(sessions, {
    selectedExternalProviderSessionIndex: Number.NaN,
  }), 0)
  assert.equal(externalProviderSessionAtSelection([], {
    selectedExternalProviderSessionIndex: 8,
  }), null)
})

function externalSession(
  id: string,
  lastModifiedAtMs: number,
  overrides: Partial<ExternalProviderSessionRecord> = {},
): ExternalProviderSessionRecord {
  return {
    external_session_id: id,
    provider: "codex",
    provider_session_id: `${id}-provider`,
    title: id,
    title_source: "provider",
    first_prompt_preview: id,
    created_at_ms: 1,
    last_modified_at_ms: lastModifiedAtMs,
    capabilities: {
      can_read_history: true,
    },
    ...overrides,
  }
}
