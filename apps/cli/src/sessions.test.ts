import assert from "node:assert/strict"
import test from "node:test"

import {
  decideBootstrapAction,
  formatSessionList,
  sessionBrowserStatus,
  sessionBrowserTimestamp,
  selectAttachableSession,
} from "./sessions.js"

test("formatSessionList renders aliases, attachment counts, and current session marker", () => {
  assert.equal(
    formatSessionList(
      [
        {
          id: "session-2",
          alias: "support",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          host_daemon_id: "home-kernel-1",
          host_machine_id: "home-machine-1",
          workspace_live_sync_mode: "tracked",
          status: "Active",
          created_at_ms: 2,
          attachment_ids: ["attachment-1", "attachment-2"],
          activity: {
            agent_count: 2,
            working_agent_count: 1,
            active_prompt_count: 1,
            queued_prompt_count: 0,
            error_agent_count: 0,
            remote_agent_count: 1,
            missing_worker_provider_run_count: 1,
          },
        },
        {
          id: "session-1",
          alias: null,
          workspace_id: "/tmp/demo",
          worktree_id: "/tmp/demo",
          status: "Ended",
          created_at_ms: 1,
          attachment_ids: [],
        },
      ],
      "session-2",
    ),
    [
      "Sessions",
      "- `support` (`session-2`) - active - 2 CLIs - arroba - home home-kernel-1@home-machine-1 - sync tracked - 1 remote/slice agent, 1 worker run gap - next: run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent - current",
      "- `session-1` - ended - 0 CLIs - demo - sync off",
    ].join("\n"),
  )
})

test("formatSessionList handles empty session sets", () => {
  assert.equal(formatSessionList([]), "No sessions found.")
})

test("formatSessionList surfaces home-proxy sync blockers", () => {
  assert.equal(
    formatSessionList([
      {
        id: "session-home-proxy",
        alias: "remote-tools",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        workspace_live_sync_mode: "managed",
        status: "Active",
        created_at_ms: 3,
        attachment_ids: ["attachment-1"],
        activity: {
          agent_count: 3,
          working_agent_count: 0,
          active_prompt_count: 0,
          queued_prompt_count: 0,
          error_agent_count: 0,
          remote_agent_count: 2,
          home_proxy_agent_count: 2,
          remote_extension_sync_issue_count: 1,
          remote_extension_pending_revoke_count: 1,
        },
      },
    ]),
    [
      "Sessions",
      "- `remote-tools` (`session-home-proxy`) - active - 1 CLI - arroba - sync managed - 2 remote/slice agents, 2 home-proxy agents, 1 extension sync issue, 1 pending revoke - next: keep the home revoke in place; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after the worker reconnects",
    ].join("\n"),
  )
})

test("formatSessionList routes aggregate stale home-proxy blockers through remote runtime", () => {
  assert.equal(
    formatSessionList([
      {
        id: "session-home-proxy",
        alias: "remote-tools",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        workspace_live_sync_mode: "tracked",
        status: "Active",
        created_at_ms: 3,
        attachment_ids: ["attachment-1"],
        activity: {
          agent_count: 2,
          working_agent_count: 0,
          active_prompt_count: 0,
          queued_prompt_count: 0,
          error_agent_count: 0,
          remote_agent_count: 1,
          home_proxy_agent_count: 1,
          remote_extension_sync_issue_count: 1,
        },
      },
    ]),
    [
      "Sessions",
      "- `remote-tools` (`session-home-proxy`) - active - 1 CLI - arroba - sync tracked - 1 remote/slice agent, 1 home-proxy agent, 1 extension sync issue - next: home keeps stale home-proxy calls blocked; run /kernel remote-runtime to identify affected agents, then use /extension sync-status and /extension sync-retry after worker connectivity is healthy",
    ].join("\n"),
  )
})

test("sessionBrowserTimestamp uses shared waiting-room timestamp labels", () => {
  assert.equal(sessionBrowserTimestamp(Date.UTC(2026, 0, 2, 10, 30)), "2026-01-02 10:30 UTC")
  assert.equal(sessionBrowserTimestamp(null), "-")
})

test("sessionBrowserStatus uses shared waiting-room activity status labels", () => {
  assert.equal(sessionBrowserStatus({ status: "remote_active" }), "Remote Active")
  assert.equal(sessionBrowserStatus({
    status: "Active",
    activity: {
      agent_count: 1,
      working_agent_count: 1,
      active_prompt_count: 0,
      queued_prompt_count: 0,
      error_agent_count: 0,
    },
  }), "Working")
})

test("selectAttachableSession ignores ended sessions and prefers the newest workspace match", () => {
  const selected = selectAttachableSession(
    [
      {
        id: "deadbeef00000001",
        alias: "old",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Ended",
        created_at_ms: 99,
        attachment_ids: [],
      },
      {
        id: "deadbeef00000002",
        alias: "keep",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Parked",
        created_at_ms: 10,
        attachment_ids: [],
      },
      {
        id: "deadbeef00000003",
        alias: "newest",
        workspace_id: "/Users/miguel/arroba",
        worktree_id: "/Users/miguel/arroba",
        status: "Active",
        created_at_ms: 20,
        attachment_ids: [],
      },
    ],
    "/Users/miguel/arroba",
    "/Users/miguel/arroba",
  )

  assert.equal(selected?.id, "deadbeef00000003")
})

test("decideBootstrapAction respects explicit session refs before the waiting room", () => {
  assert.deepEqual(
    decideBootstrapAction(
      { sessionId: "mai" },
      [
        {
          id: "deadbeef00000003",
          alias: "newest",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "resolve", sessionRef: "mai" },
  )
})

test("decideBootstrapAction lands in the waiting room by default", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [
        {
          id: "deadbeef00000001",
          alias: "old",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Ended",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "none" },
  )
})

test("decideBootstrapAction no longer auto-attaches existing sessions", () => {
  assert.deepEqual(
    decideBootstrapAction(
      {},
      [
        {
          id: "deadbeef00000003",
          alias: "newest",
          workspace_id: "/Users/miguel/arroba",
          worktree_id: "/Users/miguel/arroba",
          status: "Active",
          created_at_ms: 20,
          attachment_ids: [],
        },
      ],
      "/Users/miguel/arroba",
      "/Users/miguel/arroba",
    ),
    { action: "none" },
  )
})
