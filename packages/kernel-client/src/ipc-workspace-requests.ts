export function searchWorkspaceDirectoriesRequest(query: string, limit?: number) {
  return {
    SearchWorkspaceDirectories: {
      query,
      limit: limit ?? null,
    },
  }
}

export function createWorkspaceDirectoryRequest(path: string) {
  return {
    CreateWorkspaceDirectory: {
      path,
    },
  }
}

export function listWorkspaceWorktreesRequest(workspaceId: string) {
  return {
    ListWorkspaceWorktrees: {
      workspace_id: workspaceId,
    },
  }
}

export function createWorkspaceWorktreeRequest(
  workspaceId: string,
  options: { path?: string; branch?: string; baseRef?: string } = {},
) {
  return {
    CreateWorkspaceWorktree: {
      workspace_id: workspaceId,
      path: options.path ?? null,
      branch: options.branch ?? null,
      base_ref: options.baseRef ?? null,
    },
  }
}

export function deleteWorkspaceWorktreeRequest(
  workspaceId: string,
  worktreeId: string,
  options: { force?: boolean } = {},
) {
  return {
    DeleteWorkspaceWorktree: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      force: options.force ?? false,
    },
  }
}

export function createWorkspacePullRequestRequest(
  workspaceId: string,
  worktreeId: string,
  options: {
    title?: string | null
    body?: string | null
    baseRef?: string | null
    draft?: boolean
  } = {},
) {
  return {
    CreateWorkspacePullRequest: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      title: options.title?.trim() || null,
      body: options.body?.trim() || null,
      base_ref: options.baseRef?.trim() || null,
      draft: options.draft ?? false,
    },
  }
}

export function getWorkspaceGitOverviewRequest(
  workspaceId: string,
  worktreeId: string,
  compareRef?: string | null,
) {
  return {
    GetWorkspaceGitOverview: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      compare_ref: compareRef?.trim() || null,
    },
  }
}

export function listWorkspaceFilesRequest(
  workspaceId: string,
  worktreeId: string,
  pathPrefix = "",
  compareRef?: string | null,
  limit?: number | null,
) {
  return {
    ListWorkspaceFiles: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      path_prefix: pathPrefix.trim() || null,
      compare_ref: compareRef?.trim() || null,
      limit: limit ?? null,
    },
  }
}

export function getWorkspaceFileContentRequest(
  workspaceId: string,
  worktreeId: string,
  path: string,
  compareRef?: string | null,
  knownFingerprint?: string | null,
  maxBytes?: number | null,
) {
  return {
    GetWorkspaceFileContent: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      path: path.trim(),
      compare_ref: compareRef?.trim() || null,
      known_fingerprint: knownFingerprint?.trim() || null,
      max_bytes: maxBytes ?? null,
    },
  }
}

export function generateWorkspaceCommitMessageRequest(
  workspaceId: string,
  worktreeId: string,
  compareRef: string | null | undefined,
  sessionId: string,
  agentId: string,
) {
  return {
    GenerateWorkspaceCommitMessage: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      compare_ref: compareRef?.trim() || null,
      session_id: sessionId,
      agent_id: agentId,
    },
  }
}

export function runWorkspaceCommitMessageUtilityRequest(
  workspaceId: string,
  worktreeId: string,
  compareRef: string | null | undefined,
  sessionId: string,
  agentId: string,
) {
  return {
    RunAgentUtility: {
      session_id: sessionId,
      agent_id: agentId,
      kind: "WorkspaceCommitMessage",
      input: {
        WorkspaceCommitMessage: {
          workspace_id: workspaceId,
          worktree_id: worktreeId,
          compare_ref: compareRef?.trim() || null,
        },
      },
    },
  }
}

export function commitWorkspaceChangesRequest(
  workspaceId: string,
  worktreeId: string,
  message: string,
) {
  return {
    CommitWorkspaceChanges: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      message,
    },
  }
}

export function pushWorkspaceBranchRequest(
  workspaceId: string,
  worktreeId: string,
  forceWithLease = false,
) {
  return {
    PushWorkspaceBranch: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      force_with_lease: forceWithLease,
    },
  }
}

export function commitAndPushWorkspaceChangesRequest(
  workspaceId: string,
  worktreeId: string,
  message: string,
) {
  return {
    CommitAndPushWorkspaceChanges: {
      workspace_id: workspaceId,
      worktree_id: worktreeId,
      message,
    },
  }
}
