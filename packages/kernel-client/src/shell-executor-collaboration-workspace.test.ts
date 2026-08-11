import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  ArrobaMcpServerConfig,
  ArrobaSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  daemonHealth,
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand attaches standalone shell clients when switching sessions", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSession" in request) {
          return { SessionCreated: { session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
    clientId: "arroba-shell-test",
  })
  assert.equal(result.ok, true)
  assert.equal(result.contextUpdates?.attachmentId, "attachment-shell")
  assert.deepEqual(requests, [
    { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } },
    { AttachToSession: { session_id: "session-2", client_id: "arroba-shell-test", capability_level: "FullTerminal" } },
  ])
})

test("executeShellCommand manages session invites and members", async () => {
  const session = makeSession({
    id: "session-1",
    owner_user_id: "local",
    members: [{ user_id: "local", joined_at_ms: 0, invited_by_user_id: null }],
    invites: [],
  })
  const invite = {
    invite_id: "invite-1",
    session_id: "session-1",
    created_by_user_id: "local",
    created_at_ms: 100,
    expires_at_ms: null,
    max_uses: 1,
    used_count: 0,
    revoked_at_ms: null,
  }
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateSessionInvite" in request) {
          return { SessionInviteCreated: { invite: { invite, invite_token: "arroba-session-invite-v1.token" }, session } }
        }
        if ("JoinSessionInvite" in request) {
          return {
            SessionInviteJoined: {
              member: { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" },
              session: { ...session, members: [...(session.members ?? []), { user_id: "ana", joined_at_ms: 200, invited_by_user_id: "local" }] },
            },
          }
        }
        if ("ListSessionMembers" in request) {
          return {
            SessionMembersListed: {
              members: session.members,
              invites: [
                invite,
                { ...invite, invite_id: "invite-revoked", revoked_at_ms: 300 },
                { ...invite, invite_id: "invite-expired", expires_at_ms: 1 },
                { ...invite, invite_id: "invite-exhausted", used_count: 1 },
              ],
            },
          }
        }
        if ("RevokeSessionInvite" in request) {
          return { SessionInviteRevoked: { invite: { ...invite, revoked_at_ms: 300 }, session } }
        }
        return { SessionAttached: { attachment: { id: "attachment-shell" } } }
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })
  const inviteResult = await executeShellCommand(parseShellCommand("session invite create"), context, { client: fake.client })
  const joinResult = await executeShellCommand(parseShellCommand("session join arroba-session-invite-v1.token ana"), context, { client: fake.client, clientId: "shell-ana" })
  const membersResult = await executeShellCommand(parseShellCommand("session members"), context, { client: fake.client })
  const invitesResult = await executeShellCommand(parseShellCommand("session invites"), context, { client: fake.client })
  const revokeResult = await executeShellCommand(parseShellCommand("session revoke-invite invite-1"), context, { client: fake.client })

  assert.match(inviteResult.message ?? "", /session invite invite-1/)
  assert.match(joinResult.message ?? "", /joined session session-1 as ana/)
  assert.match(membersResult.message ?? "", /Session members/)
  assert.match(invitesResult.message ?? "", /Session invites\n- invite-1 uses=0\/1/)
  assert.doesNotMatch(invitesResult.message ?? "", /invite-(?:revoked|expired|exhausted)/)
  assert.match(revokeResult.message ?? "", /revoked session invite invite-1/)
  assert.deepEqual(requests, [
    { CreateSessionInvite: { session_id: "session-1", expires_in_ms: null, max_uses: 1, collaboration_level: "private" } },
    { JoinSessionInvite: { invite_token: "arroba-session-invite-v1.token", user_id: "ana" } },
    { AttachToSession: { session_id: "session-1", client_id: "shell-ana", capability_level: "FullTerminal" } },
    { ListSessionMembers: { session_id: "session-1" } },
    { ListSessionMembers: { session_id: "session-1" } },
    { RevokeSessionInvite: { session_id: "session-1", invite_ref: "invite-1" } },
  ])
})

test("executeShellCommand manages workspace links", async () => {
  const session = makeSession({ id: "session-1" })
  const link: WorkspaceLinkDefinition = {
    link_id: "workspace-link-1",
    session_id: "session-1",
    name: "shared-repo",
    created_by_user_id: "local",
    created_at_ms: 100,
    attachments: [],
  }
  const attached = {
    ...link,
    attachments: [{
      link_id: link.link_id,
      user_id: "local",
      machine_id: "machine-1",
      kernel_id: "kernel-1",
      repo_root: "/repo",
      branch: null,
      repo_fingerprint: null,
      attached_at_ms: 200,
    }],
  }
  const syncStatus = {
    session_id: "session-1",
    mode: "tracked",
    footer_state: "tracked",
    sync_groups: [{
      group_id: link.link_id,
      group_name: link.name,
      target_count: 1,
      ready_targets: 1,
      degraded_targets: 0,
      conflicted_targets: 0,
    }],
    targets: [{
      link_id: link.link_id,
      link_name: link.name,
      user_id: "local",
      machine_id: "machine-1",
      kernel_id: "kernel-1",
      repo_root: "/repo",
      branch: null,
      repo_fingerprint: null,
      status: "ready",
      attached_at_ms: 200,
    }],
    conflicts: [{
      conflict_id: "conflict-1",
      link_id: link.link_id,
      source_agent_id: "agent-1",
      target_user_id: "local",
      target_repo_root: "/repo",
      path: "src/app.ts",
      next_action: "reconcile target",
    }],
    ignore: { ignore_file: ".arrobaignore", rules: ["ignored/**"], force_excludes: [".git/**"] },
  }
  const auditEvents = [{
    event_id: "event-1",
    sequence: 1,
    timestamp_ms: 1_700_000_000_000,
    session_id: "session-1",
    worktree_path: "/repo",
    kind: "workspace_live_sync_mode_changed",
    metadata: {
      previous_mode: "unrestricted",
      mode: "tracked",
      caller_user_id: "local",
      command_source: "shell",
      caller_kind: "user",
      client_id: "shell-1",
      machine_id: "machine-1",
      scope: "selected_workspace_worktree",
      other_repositories: "unrestricted",
    },
  }]
  const requests: Record<string, unknown>[] = []
  const fake = {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("CreateWorkspaceLink" in request) {
          return { WorkspaceLinkCreated: { link, session } }
        }
        if ("ListWorkspaceLinks" in request) {
          return { WorkspaceLinksListed: { links: [attached] } }
        }
        if ("ShowWorkspaceLink" in request) {
          return { WorkspaceLinkShown: { link: attached } }
        }
        if ("AttachWorkspaceLink" in request) {
          return { WorkspaceLinkAttached: { link: attached, attachment: attached.attachments[0], session } }
        }
        if ("DetachWorkspaceLink" in request) {
          return { WorkspaceLinkDetached: { link, detached: attached.attachments, session } }
        }
        if ("GetWorkspaceLiveSyncStatus" in request) {
          return { WorkspaceLiveSyncStatus: { status: syncStatus } }
        }
        if ("SetWorkspaceLiveSyncMode" in request) {
          return { WorkspaceLiveSyncModeUpdated: { session } }
        }
        if ("SetUserConfigValue" in request) {
          return {
            UserConfigUpdated: {
              config: { version: 1, providers: { workspace_live_sync: "tracked" } },
              path: "/tmp/config.toml",
              effects: {
                provider_reload: {
                  path: "providers.workspace_live_sync",
                  message: "workspace live sync policy updated; provider reloads: 0 reloaded, 0 deferred, 0 unaffected",
                  reloaded_count: 0,
                  deferred_count: 0,
                  unaffected_count: 0,
                  skipped_count: 0,
                  details: [],
                },
              },
            },
          }
        }
        if ("QueryRecall" in request) {
          return { RecallEvents: { events: auditEvents } }
        }
        throw new Error("unexpected request")
      },
    },
  }
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo", sessionId: "session-1" })

  const createResult = await executeShellCommand(parseShellCommand("workspace link create shared-repo"), context, { client: fake.client })
  const listResult = await executeShellCommand(parseShellCommand("workspace link list"), context, { client: fake.client })
  const showResult = await executeShellCommand(parseShellCommand("workspace link show shared-repo"), context, { client: fake.client })
  const attachResult = await executeShellCommand(parseShellCommand("workspace link attach shared-repo"), context, { client: fake.client })
  const syncResult = await executeShellCommand(parseShellCommand("workspace sync status"), context, { client: fake.client })
  const syncDoctorResult = await executeShellCommand(parseShellCommand("workspace sync doctor"), context, { client: fake.client })
  const syncTargetsResult = await executeShellCommand(parseShellCommand("workspace sync targets"), context, { client: fake.client })
  const syncConflictsResult = await executeShellCommand(parseShellCommand("workspace sync conflicts"), context, { client: fake.client })
  const syncIgnoreResult = await executeShellCommand(parseShellCommand("workspace sync ignore"), context, { client: fake.client })
  const syncAuditResult = await executeShellCommand(parseShellCommand("workspace sync audit --limit 3"), context, { client: fake.client })
  const modeResult = await executeShellCommand(parseShellCommand("workspace sync mode tracked"), context, { client: fake.client })
  const enableResult = await executeShellCommand(parseShellCommand("workspace sync enable managed"), context, { client: fake.client })
  const enableTrackedResult = await executeShellCommand(parseShellCommand("workspace sync enable tracked"), context, { client: fake.client })
  const directOffResult = await executeShellCommand(parseShellCommand("workspace sync off"), context, { client: fake.client })
  const directManagedResult = await executeShellCommand(parseShellCommand("workspace sync managed"), context, { client: fake.client })
  const disableResult = await executeShellCommand(parseShellCommand("workspace sync disable"), context, { client: fake.client })
  const defaultResult = await executeShellCommand(parseShellCommand("workspace sync default tracked"), context, { client: fake.client })
  const syncLinkResult = await executeShellCommand(parseShellCommand("workspace sync link shared-repo"), context, { client: fake.client })
  const legacyModeOnResult = await executeShellCommand(parseShellCommand("workspace sync mode on"), context, { client: fake.client })
  const legacyModeOffResult = await executeShellCommand(parseShellCommand("workspace sync mode off"), context, { client: fake.client })
  const legacyEnableOnResult = await executeShellCommand(parseShellCommand("workspace sync enable on"), context, { client: fake.client })
  const legacyDefaultOnResult = await executeShellCommand(parseShellCommand("workspace sync default on"), context, { client: fake.client })
  const detachResult = await executeShellCommand(parseShellCommand("workspace link detach shared-repo"), context, { client: fake.client })
  const invalidResourceResult = await executeShellCommand(parseShellCommand("workspace unknown"), context, { client: fake.client })

  assert.match(createResult.message ?? "", /created workspace link shared-repo/)
  assert.match(listResult.message ?? "", /attachments=1/)
  assert.match(showResult.message ?? "", /workspace link shared-repo/)
  assert.match(attachResult.message ?? "", /live sync mode is unchanged; choose `workspace sync managed` or `workspace sync tracked` to start syncing this session/)
  assert.match(syncResult.message ?? "", /workspace live sync: tracked/)
  assert.match(syncResult.message ?? "", /scope=selected workspace\/worktree only; other repositories are unrestricted/)
  assert.match(syncResult.message ?? "", /sync_groups=1/)
  assert.match(syncResult.message ?? "", /next=inspect workspace sync conflicts, ask an agent to reconcile, then rerun workspace sync status/)
  assert.match(syncResult.message ?? "", /group shared-repo \(workspace-link-1\) targets=1 ready=1 degraded=0 conflicts=0/)
  assert.match(syncResult.message ?? "", /source=agent-1 target=local:\/repo/)
  assert.match(syncResult.message ?? "", /rule ignored\/\*\*/)
  assert.match(syncResult.message ?? "", /force-exclude \.git\/\*\*/)
  assert.match(syncDoctorResult.message ?? "", /workspace live sync doctor: conflict/)
  assert.match(syncDoctorResult.message ?? "", /problems:\n- src\/app\.ts from agent-1 blocked on local:\/repo/)
  assert.match(syncDoctorResult.message ?? "", /inspect=workspace sync targets; workspace sync conflicts; workspace sync ignore; workspace sync audit/)
  assert.match(syncTargetsResult.message ?? "", /group shared-repo \(workspace-link-1\) targets=1 ready=1 degraded=0 conflicts=0/)
  assert.match(syncTargetsResult.message ?? "", /ready shared-repo: local \/repo machine=machine-1 kernel=kernel-1/)
  assert.match(syncTargetsResult.message ?? "", /next=inspect workspace sync conflicts/)
  assert.match(syncConflictsResult.message ?? "", /src\/app\.ts source=agent-1 target=local:\/repo: reconcile target/)
  assert.match(syncIgnoreResult.message ?? "", /ignore=\.arrobaignore/)
  assert.match(syncIgnoreResult.message ?? "", /rule ignored\/\*\*/)
  assert.match(syncIgnoreResult.message ?? "", /force-exclude \.git\/\*\*/)
  assert.match(syncAuditResult.message ?? "", /workspace live sync audit: 1/)
  assert.match(syncAuditResult.message ?? "", /unrestricted -> tracked by local via shell/)
  assert.match(syncAuditResult.message ?? "", /scope=selected_workspace_worktree; other_repositories=unrestricted/)
  assert.match(modeResult.message ?? "", /current session workspace live sync set to tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(enableResult.message ?? "", /current session workspace live sync enabled: managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(enableTrackedResult.message ?? "", /current session workspace live sync enabled: tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(directOffResult.message ?? "", /disabled; other repositories remain unrestricted/)
  assert.match(directManagedResult.message ?? "", /current session workspace live sync set to managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(disableResult.message ?? "", /disabled; other repositories remain unrestricted/)
  assert.match(defaultResult.message ?? "", /default workspace live sync for new sessions set to tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(syncLinkResult.message ?? "", /live sync mode is unchanged; choose `workspace sync managed` or `workspace sync tracked` to start syncing this session/)
  assert.match(legacyModeOnResult.message ?? "", /usage: workspace sync mode off\|managed\|tracked/)
  assert.match(legacyModeOffResult.message ?? "", /disabled; other repositories remain unrestricted/)
  assert.match(legacyEnableOnResult.message ?? "", /usage: workspace sync enable \[managed\|tracked\]/)
  assert.match(legacyDefaultOnResult.message ?? "", /usage: workspace sync default off\|managed\|tracked/)
  assert.match(detachResult.message ?? "", /detached 1 workspace link attachment/)
  assert.match(invalidResourceResult.message ?? "", /workspace sync .*link/)
  assert.deepEqual(requests, [
    { CreateWorkspaceLink: { session_id: "session-1", name: "shared-repo" } },
    { ListWorkspaceLinks: { session_id: "session-1" } },
    { ShowWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo" } },
    { AttachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo", branch: null, repo_fingerprint: null } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    { GetWorkspaceLiveSyncStatus: { session_id: "session-1" } },
    {
      QueryRecall: {
        session_id: "session-1",
        agent_id: null,
        provider: null,
        model: null,
        workflow_id: null,
        machine_id: null,
        repo_root: null,
        worktree_path: null,
        kind: "workspace_live_sync_mode_changed",
        text: null,
        after_sequence: null,
        before_sequence: null,
        limit: 3,
      },
    },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "tracked" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "managed" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "tracked" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "managed" } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { SetUserConfigValue: { path: "providers.workspace_live_sync", value: "tracked" } },
    { AttachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo", branch: null, repo_fingerprint: null } },
    { SetWorkspaceLiveSyncMode: { session_id: "session-1", mode: "unrestricted" } },
    { DetachWorkspaceLink: { session_id: "session-1", link_ref: "shared-repo", repo_root: "/repo" } },
  ])
})

test("executeShellCommand changes workspace live sync default without a current session", async () => {
  const requests: Record<string, unknown>[] = []
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("workspace sync default managed"), context, {
    client: {
      send: async (request: Record<string, unknown>) => {
        requests.push(request)
        if ("SetUserConfigValue" in request) {
          return {
            UserConfigUpdated: {
              config: { version: 1, providers: { workspace_live_sync: "managed" } },
              path: "/tmp/config.toml",
              effects: {},
            },
          }
        }
        throw new Error("unexpected request")
      },
    },
  })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /default workspace live sync for new sessions set to managed \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.deepEqual(requests, [
    { SetUserConfigValue: { path: "providers.workspace_live_sync", value: "managed" } },
  ])
})
