import assert from "node:assert/strict"
import test from "node:test"

import type { SessionListEntry } from "./sessions.js"
import { createWaitingRoomLifecycleConfirmationController } from "./waiting-room-lifecycle-confirmation-controller.js"

test("waiting room lifecycle confirmation prompts before deleting a session", () => {
  let now = 10
  const controller = createWaitingRoomLifecycleConfirmationController({
    now: () => now,
    confirmationWindowMs: 100,
  })

  const first = controller.confirm("delete", {
    action: "delete",
    session: session("session-1", "Docs"),
  })
  assert.deepEqual(first, {
    action: "await-confirmation",
    target: {
      kind: "session",
      id: "session-1",
      label: "session Docs",
      verb: "delete",
    },
    message: "press D again to delete session Docs",
    tone: "error",
  })
  assert.equal(controller.pending()?.expiresAtMs, 110)

  now = 20
  const second = controller.confirm("delete", {
    action: "delete",
    session: session("session-1", "Docs"),
  })
  assert.deepEqual(second, {
    action: "confirmed",
    target: {
      kind: "session",
      id: "session-1",
      label: "session Docs",
      verb: "delete",
    },
  })
  assert.equal(controller.pending(), null)
})

test("waiting room lifecycle confirmation resets for changed or expired targets", () => {
  let now = 10
  const controller = createWaitingRoomLifecycleConfirmationController({
    now: () => now,
    confirmationWindowMs: 100,
  })

  controller.confirm("delete", {
    action: "delete-machine",
    machineId: "machine-1",
    label: "builder",
  })
  const changedTarget = controller.confirm("delete", {
    action: "delete-kernel",
    kernelId: "kernel-1",
    label: "builder-kernel",
  })
  assert.equal(changedTarget.action, "await-confirmation")
  assert.equal(changedTarget.target.kind, "kernel")

  now = 110
  const expired = controller.confirm("delete", {
    action: "delete-kernel",
    kernelId: "kernel-1",
    label: "builder-kernel",
  })
  assert.equal(expired.action, "await-confirmation")
  assert.equal(controller.pending()?.expiresAtMs, 210)
})

test("waiting room lifecycle confirmation formats bulk archive targets", () => {
  const controller = createWaitingRoomLifecycleConfirmationController()

  const result = controller.confirm("archive", {
    action: "archive-all",
    sessions: [session("session-1"), session("session-2")],
  })

  assert.deepEqual(result, {
    action: "await-confirmation",
    target: {
      kind: "sessions",
      id: "all",
      label: "2 sessions",
      verb: "archive",
    },
    message: "press A again to archive 2 sessions",
    tone: "info",
  })
})

test("waiting room lifecycle confirmation requires the project shortcut twice", () => {
  const controller = createWaitingRoomLifecycleConfirmationController()
  const decision = {
    action: "archive-project" as const,
    project: {
      id: "project-1",
      owner_user_id: "owner",
      workspace_id: "/workspace",
      name: "Frontend",
      kind: "named" as const,
      status: "active" as const,
      created_at_ms: 1,
      updated_at_ms: 2,
      session_count: 1,
      joined_collaborator_count: 0,
      pending_collaboration_invite_count: 0,
    },
  }

  assert.equal(controller.confirm("archive", decision).action, "await-confirmation")
  assert.deepEqual(controller.confirm("archive", decision), {
    action: "confirmed",
    target: {
      kind: "project",
      id: "project-1",
      label: "project Frontend",
      verb: "archive",
    },
  })
})

test("waiting room lifecycle confirmation formats slice delete targets", () => {
  const controller = createWaitingRoomLifecycleConfirmationController()

  const result = controller.confirm("delete", {
    action: "delete-slice",
    sliceId: "slice-1",
    label: "linux-dev",
  })

  assert.deepEqual(result, {
    action: "await-confirmation",
    target: {
      kind: "slice",
      id: "slice-1",
      label: "slice linux-dev",
      verb: "delete",
    },
    message: "press D again to delete slice linux-dev",
    tone: "error",
  })
})

test("waiting room lifecycle confirmation can be cleared explicitly", () => {
  const controller = createWaitingRoomLifecycleConfirmationController()

  controller.confirm("delete", {
    action: "delete-machine",
    machineId: "machine-1",
    label: "builder",
  })
  assert.notEqual(controller.pending(), null)

  controller.clear()

  assert.equal(controller.pending(), null)
})

function session(id: string, alias: string | null = null): SessionListEntry {
  return {
    id,
    alias,
    workspace_id: "/workspace",
    worktree_id: "/workspace/tree",
    status: "Created",
    created_at_ms: 1,
    attachment_ids: [],
  }
}
