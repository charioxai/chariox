import type { SessionAgentDefaults, SessionProjectSelection } from "./kernel-types.js"
import {
  type WorkspaceLiveSyncModeInput,
  workspaceLiveSyncModeProtocolValue,
} from "./workspace-live-sync-mode.js"

export function createSessionRequest(
  workspaceId: string,
  worktreeId: string,
  alias?: string,
  agentDefaults?: SessionAgentDefaults,
  sliceRef?: string | null,
  workspaceLiveSyncMode?: WorkspaceLiveSyncModeInput | null,
  kernelRef?: string | null,
  worktreePlacement?: Record<string, unknown> | null,
  projectSelection?: SessionProjectSelection | null,
) {
  return {
    CreateSession: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      alias: alias ?? null,
      ...(agentDefaults ? { agent_defaults: agentDefaults } : {}),
      slice_ref: sliceRef ?? null,
      ...(kernelRef?.trim() ? { kernel_ref: kernelRef.trim() } : {}),
      ...(worktreePlacement ? { worktree_placement: worktreePlacement } : {}),
      ...(workspaceLiveSyncMode ? { workspace_live_sync_mode: workspaceLiveSyncModeProtocolValue(workspaceLiveSyncMode) } : {}),
      ...(projectSelection ? { project_selection: projectSelection } : {}),
    },
  }
}

export function listProjectsRequest(includeArchived = false) {
  return { ListProjects: { include_archived: includeArchived } }
}

export function renameProjectRequest(projectId: string, name: string) {
  return { RenameProject: { project_id: projectId, name } }
}

export function archiveProjectRequest(projectId: string) {
  return { ArchiveProject: { project_id: projectId } }
}

export function deleteProjectRequest(projectId: string) {
  return { DeleteProject: { project_id: projectId } }
}

export function restoreProjectRequest(projectId: string) {
  return { RestoreProject: { project_id: projectId } }
}

export function listSessionsRequest() {
  return { ListSessions: null }
}

export function resolveSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    ResolveSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function attachToSessionRequest(sessionId: string, clientId: string) {
  return {
    AttachToSession: {
      session_id: sessionId,
      client_id: clientId,
      capability_level: "FullTerminal",
    },
  }
}

export function detachFromSessionRequest(attachmentId: string) {
  return {
    DetachFromSession: {
      attachment_id: attachmentId,
    },
  }
}

export function listSessionMembersRequest(sessionId: string) {
  return {
    ListSessionMembers: {
      session_id: sessionId,
    },
  }
}

export function createSessionInviteRequest(
  sessionId: string,
  expiresInMs: number | null = null,
  maxUses: number | null = null,
  collaborationLevel: "private" | "transparent" | "full" = "private",
) {
  return {
    CreateSessionInvite: {
      session_id: sessionId,
      expires_in_ms: expiresInMs,
      max_uses: maxUses,
      collaboration_level: collaborationLevel,
    },
  }
}

export function joinSessionInviteRequest(inviteToken: string, userId: string) {
  return {
    JoinSessionInvite: {
      invite_token: inviteToken,
      user_id: userId,
    },
  }
}

export function revokeSessionInviteRequest(sessionId: string, inviteRef: string) {
  return {
    RevokeSessionInvite: {
      session_id: sessionId,
      invite_ref: inviteRef,
    },
  }
}

export function createWorkspaceLinkRequest(sessionId: string, name: string) {
  return {
    CreateWorkspaceLink: {
      session_id: sessionId,
      name,
    },
  }
}

export function listWorkspaceLinksRequest(sessionId: string) {
  return {
    ListWorkspaceLinks: {
      session_id: sessionId,
    },
  }
}

export function showWorkspaceLinkRequest(sessionId: string, linkRef: string) {
  return {
    ShowWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
    },
  }
}

export function attachWorkspaceLinkRequest(
  sessionId: string,
  linkRef: string,
  repoRoot?: string | null,
  branch?: string | null,
  repoFingerprint?: string | null,
) {
  return {
    AttachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
      branch: branch ?? null,
      repo_fingerprint: repoFingerprint ?? null,
    },
  }
}

export function detachWorkspaceLinkRequest(sessionId: string, linkRef: string, repoRoot?: string | null) {
  return {
    DetachWorkspaceLink: {
      session_id: sessionId,
      link_ref: linkRef,
      repo_root: repoRoot ?? null,
    },
  }
}

export function getWorkspaceLiveSyncStatusRequest(sessionId: string) {
  return {
    GetWorkspaceLiveSyncStatus: {
      session_id: sessionId,
    },
  }
}

export function setWorkspaceLiveSyncModeRequest(sessionId: string, mode: WorkspaceLiveSyncModeInput) {
  return {
    SetWorkspaceLiveSyncMode: {
      session_id: sessionId,
      mode: workspaceLiveSyncModeProtocolValue(mode),
    },
  }
}

export function endSessionRequest(sessionId: string) {
  return {
    EndSession: {
      session_id: sessionId,
    },
  }
}

export function deleteSessionRequest(sessionRef: string, workspaceId?: string) {
  return {
    DeleteSession: {
      session_ref: sessionRef,
      workspace_id: workspaceId ?? null,
    },
  }
}

export function aliasSessionRequest(sessionId: string, alias: string) {
  return {
    AliasSession: {
      session_id: sessionId,
      alias,
    },
  }
}

export function getSessionStateRequest(sessionId: string) {
  return {
    GetSessionState: {
      session_id: sessionId,
    },
  }
}

export function updateMetaagentTaskRequest(
  sessionId: string,
  metaagentId: string,
  updates: { taskMarkdown?: string | null; planMarkdown?: string | null },
) {
  return {
    UpdateMetaagentTask: {
      session_id: sessionId,
      metaagent_id: metaagentId,
      ...(updates.taskMarkdown !== undefined ? { task_markdown: updates.taskMarkdown } : {}),
      ...(updates.planMarkdown !== undefined ? { plan_markdown: updates.planMarkdown } : {}),
    },
  }
}

export function pauseMetaagentTaskRequest(sessionId: string, metaagentId: string) {
  return {
    PauseMetaagentTask: {
      session_id: sessionId,
      metaagent_id: metaagentId,
    },
  }
}

export function resumeMetaagentTaskRequest(sessionId: string, metaagentId: string) {
  return {
    ResumeMetaagentTask: {
      session_id: sessionId,
      metaagent_id: metaagentId,
    },
  }
}

export function abortMetaagentTaskRequest(
  sessionId: string,
  metaagentId: string,
  reason?: string | null,
) {
  return {
    AbortMetaagentTask: {
      session_id: sessionId,
      metaagent_id: metaagentId,
      reason: reason ?? null,
    },
  }
}
