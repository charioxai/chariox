import assert from "node:assert/strict"
import test from "node:test"

import type { WorkspaceLiveSyncStatus } from "./kernel-types.js"
import {
  workspaceLiveSyncHealthLabel,
  workspaceLiveSyncProblems,
} from "./shell-workspace-format.js"

test("workspace live sync health label uses one shared precedence order", () => {
  assert.equal(workspaceLiveSyncHealthLabel(status({
    footer_state: "conflict",
    mode: "unrestricted",
  })), "conflict")
  assert.equal(workspaceLiveSyncHealthLabel(status({
    sync_groups: [{ ...group(), degraded_targets: 1 }],
  })), "degraded")
  assert.equal(workspaceLiveSyncHealthLabel(status({
    targets: [{ ...target(), status: "degraded" }],
  })), "degraded")
  assert.equal(workspaceLiveSyncHealthLabel(status({
    mode: "unrestricted",
    footer_state: "off",
  })), "off")
  assert.equal(workspaceLiveSyncHealthLabel(status({
    targets: [],
  })), "no-targets")
  assert.equal(workspaceLiveSyncHealthLabel(status({
    footer_state: "syncing",
  })), "syncing")
  assert.equal(workspaceLiveSyncHealthLabel(status()), "healthy")
})

test("workspace live sync problems describe off, empty, degraded, and conflicted state", () => {
  assert.deepEqual(workspaceLiveSyncProblems(status({
    mode: "unrestricted",
    footer_state: "off",
    targets: [],
  })), ["live sync is off for this session"])
  assert.deepEqual(workspaceLiveSyncProblems(status({
    targets: [],
  })), ["no synced worktrees or remote attachments are linked"])
  assert.deepEqual(workspaceLiveSyncProblems(status({
    sync_groups: [{ ...group(), degraded_targets: 2, conflicted_targets: 1 }],
    conflicts: [{
      conflict_id: "conflict-1",
      link_id: "link-1",
      source_agent_id: "agent-1",
      target_user_id: "user-2",
      target_repo_root: "/repo/peer",
      path: "src/app.ts",
      next_action: "reconcile target",
    }],
  })), [
    "shared has 2 degraded targets",
    "shared has 1 conflicted target",
    "src/app.ts from agent-1 blocked on user-2:/repo/peer",
  ])
})

function status(overrides: Partial<WorkspaceLiveSyncStatus> = {}): WorkspaceLiveSyncStatus {
  return {
    session_id: "session-1",
    mode: "managed",
    footer_state: "managed",
    sync_groups: [group()],
    targets: [target()],
    conflicts: [],
    ignore: {
      ignore_file: null,
      rules: [],
      force_excludes: [],
    },
    ...overrides,
  }
}

function group(): WorkspaceLiveSyncStatus["sync_groups"][number] {
  return {
    group_id: "link-1",
    group_name: "shared",
    target_count: 1,
    ready_targets: 1,
    degraded_targets: 0,
    conflicted_targets: 0,
  }
}

function target(): WorkspaceLiveSyncStatus["targets"][number] {
  return {
    link_id: "link-1",
    link_name: "shared",
    user_id: "user-2",
    machine_id: "machine-2",
    kernel_id: "kernel-2",
    repo_root: "/repo/peer",
    branch: "main",
    repo_fingerprint: null,
    status: "ready",
    attached_at_ms: 1,
  }
}
