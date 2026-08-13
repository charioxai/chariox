import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkspaceLinkDefinition,
  WorkspaceLiveSyncStatus,
} from "./cli-types.js"
import type { RecallEvent } from "@chariox/kernel-client"
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
      ignore_file: ".charioxignore",
      rules: ["ignored/**", "*.secret"],
      force_excludes: [".git/**", ".chariox/**"],
    },
  }
  const notices: string[] = []
  const footers: string[] = []
  const modeUpdates: string[] = []
  const defaultUpdates: string[] = []
  const statusUpdates: string[] = []
  const appliedSessions: string[] = []
  const auditCalls: string[] = []
  const auditEvents: RecallEvent[] = [{
    event_id: "event-1",
    sequence: 12,
    timestamp_ms: Date.parse("2026-06-02T12:00:00.000Z"),
    workspace_id: "workspace-1",
    session_id: "session-1",
    worktree_path: "/repo/main",
    kind: "workspace_live_sync_mode_changed",
    role: "system",
    content: "Workspace live sync mode changed to tracked.",
    metadata: {
      caller_user_id: "user-1",
      previous_mode: "managed",
      mode: "tracked",
      command_source: "Local",
      caller_kind: "Client",
      client_id: "cli-1",
      machine_id: "machine-1",
      scope: "selected_workspace_worktree",
      other_repositories: "unrestricted",
    },
  }]
  const deps = workspaceDeps({
    getWorkspaceLiveSyncStatus: async () => status,
    setWorkspaceLiveSyncStatus: (nextStatus) => {
      if (nextStatus) statusUpdates.push(nextStatus.footer_state)
    },
    listWorkspaceLiveSyncAudit: async (sessionId, limit) => {
      auditCalls.push(`${sessionId}:${limit ?? "default"}`)
      return auditEvents
    },
    setWorkspaceLiveSyncMode: async (sessionId, mode) => {
      modeUpdates.push(`${sessionId}:${mode}`)
      return {
        session: session({ workspace_live_sync_mode: mode }),
        effects: [{
          kind: "provider_reload",
          path: "session.workspace_live_sync_mode",
          message: "session workspace live sync mode updated; provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
          provider_reload: {
            reloaded: 1,
            deferred: 0,
            unaffected: 0,
          },
        }],
      }
    },
    setUserConfigValue: async (path, value) => {
      defaultUpdates.push(`${path}:${value}`)
    },
    appendNotice: (message) => notices.push(message),
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
    applySessionState: (nextSession) => {
      appliedSessions.push(nextSession.workspace_live_sync_mode ?? "config-default")
    },
  })

  await runWorkspace(deps, "/workspace sync status")
  await runWorkspace(deps, "/workspace sync doctor")
  await runWorkspace(deps, "/workspace sync targets")
  await runWorkspace(deps, "/workspace sync conflicts")
  await runWorkspace(deps, "/workspace sync ignore")
  await runWorkspace(deps, "/workspace sync audit --limit 3")
  await runWorkspace(deps, "/workspace sync mode tracked")
  await runWorkspace(deps, "/workspace sync enable managed")
  await runWorkspace(deps, "/workspace sync enable tracked")
  await runWorkspace(deps, "/workspace sync disable")
  await runWorkspace(deps, "/workspace sync default tracked")

  assert.match(notices[0] ?? "", /Workspace live sync: managed footer=conflict/)
  assert.match(notices[0] ?? "", /Scope: selected workspace\/worktree only; other repositories are unrestricted/)
  assert.match(notices[0] ?? "", /Sync groups: 1/)
  assert.match(notices[0] ?? "", /Next: inspect \/workspace sync conflicts, ask an agent to reconcile, then rerun \/workspace sync status/)
  assert.match(notices[0] ?? "", /Rules: ignored\/\*\*, \*\.secret/)
  assert.match(notices[0] ?? "", /Force excludes: \.git\/\*\*, \.chariox\/\*\*/)
  assert.match(notices[1] ?? "", /Workspace live sync doctor: conflict/)
  assert.match(notices[1] ?? "", /Scope: selected workspace\/worktree only; other repositories are unrestricted/)
  assert.match(notices[1] ?? "", /Problems:/)
  assert.match(notices[1] ?? "", /shared has 1 conflicted target/)
  assert.match(notices[1] ?? "", /src\/app\.ts from agent-1 blocked on user-2:\/repo\/peer/)
  assert.match(notices[1] ?? "", /Inspect: \/workspace sync targets; \/workspace sync conflicts; \/workspace sync ignore; \/workspace sync audit/)
  assert.match(notices[2] ?? "", /conflict shared: user-2 \/repo\/peer branch=main machine=machine-2 kernel=kernel-2/)
  assert.match(notices[2] ?? "", /Group shared \(link-1\) targets=1 ready=0 degraded=0 conflicts=1/)
  assert.match(notices[2] ?? "", /Next: inspect \/workspace sync conflicts/)
  assert.match(notices[3] ?? "", /src\/app\.ts source=agent-1 target=user-2:\/repo\/peer next=reconcile target/)
  assert.match(notices[4] ?? "", /Ignore file: \.charioxignore/)
  assert.match(notices[4] ?? "", /rule ignored\/\*\*/)
  assert.match(notices[4] ?? "", /rule \*\.secret/)
  assert.match(notices[4] ?? "", /force-exclude \.git\/\*\*/)
  assert.match(notices[5] ?? "", /Workspace live sync audit: 1/)
  assert.match(notices[5] ?? "", /2026-06-02T12:00:00.000Z managed -> tracked by user-1 via Local/)
  assert.match(notices[5] ?? "", /scope=selected_workspace_worktree; other_repositories=unrestricted/)
  assert.match(notices[5] ?? "", /caller=Client client=cli-1 machine=machine-1 worktree=\/repo\/main/)
  assert.match(notices[5] ?? "", /Next: use \/workspace sync status/)
  assert.deepEqual(statusUpdates, [
    "conflict",
    "conflict",
    "conflict",
    "conflict",
    "conflict",
    "conflict",
    "conflict",
    "conflict",
    "conflict",
  ])
  assert.deepEqual(appliedSessions, ["tracked", "managed", "tracked", "unrestricted"])
  assert.deepEqual(auditCalls, ["session-1:3"])
  assert.deepEqual(modeUpdates, [
    "session-1:tracked",
    "session-1:managed",
    "session-1:tracked",
    "session-1:unrestricted",
  ])
  assert.deepEqual(defaultUpdates, ["providers.workspace_live_sync:tracked"])
  assert.deepEqual(footers.slice(-4), [
    "info:current session workspace live sync enabled: managed (selected workspace/worktree only; other repositories unrestricted); provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
    "info:current session workspace live sync enabled: tracked (selected workspace/worktree only; other repositories unrestricted); provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
    "info:current session workspace live sync disabled; other repositories remain unrestricted; provider reloads: 1 reloaded, 0 deferred, 0 unaffected",
    "info:default workspace live sync for new sessions set to tracked (selected workspace/worktree only; other repositories unrestricted)",
  ])

  status.targets = []
  notices.length = 0
  await runWorkspace(deps, "/workspace sync targets")
  assert.match(notices[0] ?? "", /Group shared \(link-1\) targets=1 ready=0 degraded=0 conflicts=1/)
  assert.doesNotMatch(notices[0] ?? "", /No workspace live sync targets/)
  assert.equal(statusUpdates.length, 10)
})

test("workspace sync audit renders empty state", async () => {
  const notices: string[] = []
  const footers: string[] = []
  const deps = workspaceDeps({
    listWorkspaceLiveSyncAudit: async () => [],
    appendNotice: (message) => notices.push(message),
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync audit")

  assert.match(notices[0] ?? "", /No workspace live sync audit events/)
  assert.match(notices[0] ?? "", /Next: change mode with \/workspace sync off\|managed\|tracked/)
  assert.equal(footers[0], "info:workspace live sync audit: 0")
})

test("workspace sync slash commands use off managed tracked mode names", async () => {
  const footers: string[] = []
  const modeUpdates: string[] = []
  const defaultUpdates: string[] = []
  const deps = workspaceDeps({
    setWorkspaceLiveSyncMode: async (sessionId, mode) => {
      modeUpdates.push(`${sessionId}:${mode}`)
    },
    setUserConfigValue: async (path, value) => {
      defaultUpdates.push(`${path}:${value}`)
    },
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync mode on")
  await runWorkspace(deps, "/workspace sync mode off")
  await runWorkspace(deps, "/workspace sync enable on")
  await runWorkspace(deps, "/workspace sync default on")
  await runWorkspace(deps, "/workspace sync default off")

  assert.deepEqual(modeUpdates, ["session-1:unrestricted"])
  assert.deepEqual(defaultUpdates, ["providers.workspace_live_sync:off"])
  assert.deepEqual(footers, [
    "error:usage: /workspace sync mode off|managed|tracked",
    "info:current session workspace live sync disabled; other repositories remain unrestricted",
    "error:usage: /workspace sync enable [managed|tracked]",
    "error:usage: /workspace sync default off|managed|tracked",
    "info:default workspace live sync for new sessions disabled; other repositories remain unrestricted",
  ])
})

test("workspace sync default can be changed without an attached session", async () => {
  const footers: string[] = []
  const defaultUpdates: string[] = []
  const deps = workspaceDeps({
    isAttached: () => false,
    setUserConfigValue: async (path, value) => {
      defaultUpdates.push(`${path}:${value}`)
    },
    flashFooter: (message, tone) => footers.push(`${tone}:${message}`),
  })

  await runWorkspace(deps, "/workspace sync default managed")
  await runWorkspace(deps, "/workspace sync status")

  assert.deepEqual(defaultUpdates, ["providers.workspace_live_sync:managed"])
  assert.deepEqual(footers, [
    "info:default workspace live sync for new sessions set to managed (selected workspace/worktree only; other repositories unrestricted)",
    "error:attach to a session before viewing workspace live sync",
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
  assert.equal(footers[0], "info:linked /repo/peer for workspace live sync via shared; next: /workspace sync managed, or /workspace sync tracked on workers without managed write fencing")
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

function session(overrides: Partial<RuntimeSession> = {}): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
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
    ...overrides,
  }
}
