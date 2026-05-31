import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import { parseSlashCommand } from "./commands.js"
import {
  handleWorkspaceSlashCommand,
  type WorkspaceCommandHandlerDeps,
} from "./workspace-command-handlers.js"

test("workspace sync slash commands render status surfaces and mutate mode", async () => {
  const status: WorkspaceLiveSyncStatus = {
    session_id: "session-1",
    mode: "managed",
    footer_state: "conflict",
    sync_groups: [{
      group_id: "link-1",
      group_name: "shared",
      target_count: 1,
      ready_targets: 0,
      degraded_targets: 0,
      conflicted_targets: 1,
    }],
    targets: [{
      link_id: "link-1",
      link_name: "shared",
      user_id: "user-2",
      machine_id: "machine-2",
      kernel_id: "kernel-2",
      repo_root: "/repo/peer",
      branch: "main",
      repo_fingerprint: null,
      status: "conflict",
      attached_at_ms: 1,
    }],
    conflicts: [{
      conflict_id: "conflict-1",
      link_id: "link-1",
      source_agent_id: "agent-1",
      target_user_id: "user-2",
      target_repo_root: "/repo/peer",
      path: "src/app.ts",
      next_action: "reconcile target",
    }],
    ignore: {
      ignore_file: ".arrobaignore",
      rules: ["ignored/**", "*.secret"],
      force_excludes: [".git/**", ".arroba/**"],
    },
  }
  const notices: string[] = []
  const footers: string[] = []
  const modeUpdates: string[] = []
  const statusUpdates: string[] = []
  const deps = workspaceDeps({
    getWorkspaceLiveSyncStatus: async () => status,
    setWorkspaceLiveSyncStatus: (nextStatus) => {
      if (nextStatus) statusUpdates.push(nextStatus.footer_state)
    },
    setWorkspaceLiveSyncMode: async (sessionId, mode) => {
      modeUpdates.push(`${sessionId}:${mode}`)
    },
    appendNotice: (message) => notices.push(message),
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync status")
  await runWorkspace(deps, "/workspace sync targets")
  await runWorkspace(deps, "/workspace sync conflicts")
  await runWorkspace(deps, "/workspace sync ignore")
  await runWorkspace(deps, "/workspace sync mode tracked")
  await runWorkspace(deps, "/workspace sync enable managed")
  await runWorkspace(deps, "/workspace sync enable tracked")
  await runWorkspace(deps, "/workspace sync disable")

  assert.match(notices[0] ?? "", /Workspace live sync: managed footer=conflict/)
  assert.match(notices[0] ?? "", /Sync groups: 1/)
  assert.match(notices[0] ?? "", /Rules: ignored\/\*\*, \*\.secret/)
  assert.match(notices[1] ?? "", /conflict shared: user-2 \/repo\/peer branch=main/)
  assert.match(notices[1] ?? "", /Group shared \(link-1\) targets=1 ready=0 degraded=0 conflicts=1/)
  assert.match(notices[2] ?? "", /src\/app\.ts source=agent-1 target=user-2:\/repo\/peer next=reconcile target/)
  assert.match(notices[3] ?? "", /Ignore file: \.arrobaignore/)
  assert.match(notices[3] ?? "", /rule ignored\/\*\*/)
  assert.match(notices[3] ?? "", /rule \*\.secret/)
  assert.deepEqual(statusUpdates, ["conflict", "conflict", "conflict", "conflict"])
  assert.deepEqual(modeUpdates, [
    "session-1:tracked",
    "session-1:managed",
    "session-1:tracked",
    "session-1:unrestricted",
  ])
  assert.deepEqual(footers.slice(-4), [
    "info:workspace live sync mode set to tracked",
    "info:workspace live sync enabled: managed",
    "info:workspace live sync enabled: tracked",
    "info:workspace live sync disabled",
  ])

  status.targets = []
  notices.length = 0
  await runWorkspace(deps, "/workspace sync targets")
  assert.match(notices[0] ?? "", /Group shared \(link-1\) targets=1 ready=0 degraded=0 conflicts=1/)
  assert.doesNotMatch(notices[0] ?? "", /No workspace live sync targets/)
  assert.equal(statusUpdates.length, 5)
})

test("workspace sync slash commands use off managed tracked mode names", async () => {
  const footers: string[] = []
  const modeUpdates: string[] = []
  const deps = workspaceDeps({
    setWorkspaceLiveSyncMode: async (sessionId, mode) => {
      modeUpdates.push(`${sessionId}:${mode}`)
    },
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync mode on")
  await runWorkspace(deps, "/workspace sync mode off")
  await runWorkspace(deps, "/workspace sync enable on")

  assert.deepEqual(modeUpdates, ["session-1:unrestricted"])
  assert.deepEqual(footers, [
    "error:usage: /workspace sync mode off|managed|tracked",
    "info:workspace live sync disabled",
    "error:usage: /workspace sync enable [managed|tracked]",
  ])
})

test("workspace sync link uses sync-specific attach wording", async () => {
  const link: WorkspaceLinkDefinition = {
    link_id: "link-1",
    session_id: "session-1",
    name: "shared",
    created_by_user_id: "user-1",
    created_at_ms: 1,
    attachments: [],
  }
  const footers: string[] = []
  const attached: Array<[string, string | null | undefined]> = []
  const deps = workspaceDeps({
    attachWorkspaceLink: async (linkRef, repoRoot) => {
      attached.push([linkRef, repoRoot])
      return { link, session: session() }
    },
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync link shared ../peer")
  await runWorkspace(deps, "/workspace sync link")

  assert.deepEqual(attached, [["shared", "/repo/peer"]])
  assert.equal(footers[0], "info:linked /repo/peer for workspace live sync via shared; recommended mode: managed")
  assert.equal(footers[1], "error:usage: /workspace sync link <name-or-id> [repo-root]")
})

async function runWorkspace(
  deps: WorkspaceCommandHandlerDeps,
  raw: string,
): Promise<void> {
  const command = parseSlashCommand(raw)
  assert.equal(command?.kind, "workspace")
  await handleWorkspaceSlashCommand(
    deps,
    command as Extract<NonNullable<ReturnType<typeof parseSlashCommand>>, { kind: "workspace" }>,
  )
}

function workspaceDeps(
  overrides: Partial<WorkspaceCommandHandlerDeps> = {},
): WorkspaceCommandHandlerDeps {
  return {
    currentWorkspaceTarget: () => "/repo/main",
    currentWorktreeTarget: () => "/repo/main",
    setWorkspaceTarget: () => {},
    setWorktreeTarget: () => {},
    baseWorktree: "/repo/main",
    hasDynamicWorktreeTarget: false,
    isAttached: () => true,
    sessionState: session,
    flashFooter: () => {},
    appendNotice: () => {},
    applySessionState: () => {},
    ...overrides,
  }
}

function session(): RuntimeSession {
  return {
    id: "session-1",
    alias: null,
    status: "Active",
    workspace_id: "/repo/main",
    worktree_id: "/repo/main",
    created_at_ms: 1,
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: null,
    max_agents: 1,
    agents: [],
    config_state: { version: 1, values: {} },
  }
}
