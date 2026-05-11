# Kernel-CLI Protocol

This page describes the current local protocol between the Arroba Kernel and the TypeScript CLI.

It reflects the current implementation, not the long-term remote/federated design.

Current shared local daemon protocol version: `17`.

Primary implementation sources:

- `apps/kernel/src/runtime_transport.rs`
- `apps/cli/src/ipc.ts`
- `apps/cli/src/ipc-requests.ts`

## Transport

The primary local transport is a WebSocket connection from the CLI to the kernel.

Default endpoint:

- `ws://127.0.0.1:${ARROBA_KERNEL_PORT}/kernel`

The CLI still retains compatibility fallback logic for the older local socket path, but the main path is WebSocket and event-driven.

## Frame Types

The protocol uses JSON frames.

Incoming frames from CLI to kernel:

- `request`
- `subscribe`
- `unsubscribe`

Outgoing frames from kernel to CLI:

- `response`
- `event`

## Request/Response Frames

### Request

```json
{
  "type": "request",
  "request_id": "uuid-or-random-id",
  "request": { "...": "LocalDaemonRequest payload" }
}
```

### Response

```json
{
  "type": "response",
  "request_id": "same-id",
  "response": { "...": "response payload or null" },
  "error": null
}
```

When an error is present:

```json
{
  "type": "response",
  "request_id": "same-id",
  "response": null,
  "error": {
    "code": "transport_or_request_code",
    "message": "human-readable message",
    "retryable": true
  }
}
```

## Kernel Event IDs

Protocol version `13` adds the `agent_activity` projection to `PromptSubmitted` responses so clients update badges from kernel-owned turn state at prompt admission, without output-based inference.

Protocol version `17` adds optional editable custom choices for choice interactions. `RuntimeInteraction` may include `custom_choice`, and `RespondToInteraction` may include `custom_reply` when responding with that custom choice id.

Protocol version `11` adds `GetWorkspaceFileContent` for read-only file preview, including language metadata, bounded content, fingerprint refresh, and not-modified responses.

Protocol version `10` adds resolved compare-ref and truncation metadata to `WorkspaceFilesListed` responses.

Protocol version `9` adds `RunAgentUtility` for hidden kernel-owned agent utilities.

Protocol version `8` adds Workspace Git action requests for commit-message generation, commit, push, and commit-and-push.

Protocol version `7` adds `ListWorkspaceFiles` and Workspace Git change totals for CLI Workspace panels.

Protocol version `6` adds top-level `agent_activity` to `SessionState` responses.
Clients must use this authoritative kernel projection for agent runtime badges after refresh.

Protocol version `5` makes kernel transport event IDs monotonic for each kernel identity across process restarts.
Clients may keep using `event_id` as a replay cursor after reconnect without dropping fresh post-restart events as duplicates.

## Workspace Git Overview

Protocol version `4` adds `GetWorkspaceGitOverview` for CLI Workspace/Git panels.
The request is inspection-only; it never mutates a session, agent, directory, or worktree.

Request payload:

```json
{
  "GetWorkspaceGitOverview": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "compare_ref": "origin/main"
  }
}
```

Response payload:

```json
{
  "WorkspaceGitOverview": {
    "overview": {
      "workspace_id": "/Users/miguel/arroba",
      "worktree_id": "/Users/miguel/arroba",
      "repo_root": "/Users/miguel/arroba",
      "repo_label": "mgutierrez09/arroba",
      "branch": "main",
      "compare_ref": "origin/main",
      "compare_refs": [
        { "name": "origin/main", "detail": "remote", "selected": true }
      ],
      "totals": { "files": 1, "additions": 23, "deletions": 9 },
      "files": [
        { "path": "apps/kernel/src/runtime/router.rs", "status": "modified", "additions": 23, "deletions": 9 }
      ],
      "generated_at_ms": 1778080000000
    }
  }
}
```

## Workspace Repo Files

Protocol version `7` adds `ListWorkspaceFiles` for lazy repo tree inspection.
Protocol version `10` adds `compare_ref`, `total_entries`, and `truncated` to the listing so clients can detect the exact diff base and bounded result sets.
Protocol version `11` adds `GetWorkspaceFileContent` for bounded read-only previews of files from the same repo/worktree context.
The request is shallow: pass a `path_prefix` to list only one folder level.
The response includes changed flags and line counts relative to the same `compare_ref` used by `GetWorkspaceGitOverview`.

Request payload:

```json
{
  "ListWorkspaceFiles": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "path_prefix": "apps/kernel",
    "compare_ref": "origin/main",
    "limit": 500
  }
}
```

Response payload:

```json
{
  "WorkspaceFilesListed": {
    "listing": {
      "workspace_id": "/Users/miguel/arroba",
      "worktree_id": "/Users/miguel/arroba",
      "path_prefix": "apps/kernel",
      "compare_ref": "origin/main",
      "total_entries": 42,
      "truncated": false,
      "entries": [
        {
          "path": "apps/kernel/src",
          "name": "src",
          "kind": "directory",
          "changed": true,
          "status": "changed",
          "additions": 23,
          "deletions": 9
        }
      ],
      "generated_at_ms": 1778080000000
    }
  }
}
```

File content request payload:

```json
{
  "GetWorkspaceFileContent": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "path": "apps/kernel/src/runtime/router.rs",
    "compare_ref": "origin/main",
    "known_fingerprint": "optional-last-fingerprint",
    "max_bytes": 750000
  }
}
```

The kernel rejects absolute paths, parent-directory escapes, directories, and unreadable files. Text files return UTF-8 content; binary or non-UTF-8 files return base64 content. Clients should pass the previous `fingerprint` on refresh so unchanged open previews avoid reloading content.

Response payload:

```json
{
  "WorkspaceFileContent": {
    "content": {
      "workspace_id": "/Users/miguel/arroba",
      "worktree_id": "/Users/miguel/arroba",
      "path": "apps/kernel/src/runtime/router.rs",
      "name": "router.rs",
      "language": "rust",
      "mime": "text/x-rust",
      "encoding": "utf-8",
      "content_text": "fn main() {}\n",
      "size_bytes": 13,
      "mtime_ms": 1778080000000,
      "fingerprint": "sha256-of-path-size-mtime",
      "sha256": "sha256-of-returned-content",
      "truncated": false,
      "status": "modified",
      "additions": 23,
      "deletions": 9,
      "compare_ref": "origin/main",
      "generated_at_ms": 1778080000100
    }
  }
}
```

Not-modified response payload:

```json
{
  "WorkspaceFileContentNotModified": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "path": "apps/kernel/src/runtime/router.rs",
    "fingerprint": "sha256-of-path-size-mtime",
    "generated_at_ms": 1778080000200
  }
}
```

## Workspace Git Actions

Protocol version `8` adds mutation requests for Workspace Git panels. The kernel owns these actions; clients must not shell out directly.

Protocol version `9` adds hidden agent utilities. Utilities run through the kernel and are not submitted as visible terminal prompts, so they do not create prompt history, transcript blobs, or turn state.

`WorkspaceCommitMessage` is provider-backed. The kernel collects bounded git context, but the commit subject itself is generated by the selected idle agent's provider runtime. It currently requires a local Codex provider runtime that can be launched or reused for the selected agent. Unsupported providers, busy agents, remote-backed agents without worker forwarding, provider launch failures, git inspection failures, and model failures are returned as errors; there is no kernel heuristic or silent fallback.

Run agent utility request:

```json
{
  "RunAgentUtility": {
    "session_id": "session-id",
    "agent_id": "agent-id",
    "kind": "WorkspaceCommitMessage",
    "input": {
      "WorkspaceCommitMessage": {
        "workspace_id": "/Users/miguel/arroba",
        "worktree_id": "/Users/miguel/arroba",
        "compare_ref": "origin/main"
      }
    }
  }
}
```

The kernel rejects this request if the selected agent is not part of the session, is remote-backed without worker forwarding, is currently busy, cannot launch a supported provider runtime, or if the provider utility turn fails. Response:

```json
{
  "AgentUtilityCompleted": {
    "result": {
      "utility_run_id": "utility-...",
      "session_id": "session-id",
      "agent_id": "agent-id",
      "kind": "WorkspaceCommitMessage",
      "output": {
        "WorkspaceCommitMessage": {
          "message": "Update workspace git panel"
        }
      },
      "generated_at_ms": 1778080000000
    }
  }
}
```

Generate commit message request:

```json
{
  "GenerateWorkspaceCommitMessage": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "compare_ref": "origin/main",
    "session_id": "session-id",
    "agent_id": "agent-id"
  }
}
```

This compatibility request is backed by the same `RunAgentUtility` execution path and has the same provider-backed requirements. Response:

```json
{
  "WorkspaceCommitMessageGenerated": {
    "message": "Update workspace git panel"
  }
}
```

Commit request:

```json
{
  "CommitWorkspaceChanges": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "message": "Update workspace git panel"
  }
}
```

Push request:

```json
{
  "PushWorkspaceBranch": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "force_with_lease": false
  }
}
```

Commit-and-push request:

```json
{
  "CommitAndPushWorkspaceChanges": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba",
    "message": "Update workspace git panel"
  }
}
```

Delete worktree request:

```json
{
  "DeleteWorkspaceWorktree": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba-feature",
    "force": false
  }
}
```

Create pull request request:

```json
{
  "CreateWorkspacePullRequest": {
    "workspace_id": "/Users/miguel/arroba",
    "worktree_id": "/Users/miguel/arroba-feature",
    "title": "Update workspace git panel",
    "body": "Optional pull request body",
    "base_ref": "main",
    "draft": true
  }
}
```

Git action response:

```json
{
  "WorkspaceGitActionCompleted": {
    "result": {
      "workspace_id": "/Users/miguel/arroba",
      "worktree_id": "/Users/miguel/arroba",
      "action": "commit",
      "message": "committed workspace changes",
      "commit_sha": "abcdef...",
      "branch": "main",
      "generated_at_ms": 1778080000000
    }
  }
}
```

Pull request response:

```json
{
  "WorkspacePullRequestCreated": {
    "pull_request": {
      "workspace_id": "/Users/miguel/arroba",
      "worktree_id": "/Users/miguel/arroba-feature",
      "branch": "arroba/feature",
      "base_ref": "main",
      "url": "https://github.com/example/repo/pull/1",
      "title": "Update workspace git panel",
      "draft": true,
      "generated_at_ms": 1778080000000
    }
  }
}
```

## Subscription Lifecycle

The CLI opens the WebSocket and then subscribes to one session/attachment event stream.

### Subscribe

```json
{
  "type": "subscribe",
  "request_id": "id",
  "session_id": "session-id",
  "attachment_id": "attachment-id",
  "resume_from_event_id": 123
}
```

`resume_from_event_id` is optional and is used for reconnect/resume.

### Unsubscribe

```json
{
  "type": "unsubscribe",
  "request_id": "id"
}
```

## Event Frames

All pushed events are wrapped like this:

```json
{
  "type": "event",
  "event_id": 124,
  "event": {
    "event": "event_name",
    "...": "event payload"
  }
}
```

Current event kinds:

### `terminal_output`

Provider output and other terminal stream records.

Payload:

- `records`

### `runtime_notices`

Kernel/runtime notices for the current attachment.

Payload:

- `notices`

### `assistant_message_completed`

Structured completion signal for the final assistant message from OpenCode.

Payload:

- `session_id`
- `provider_run_id`
- `agent_id`
- `message_id`
- `completed_at_ms`

Clients use this as transcript metadata for assistant-message grouping. Runtime status must continue to come from kernel-projected `agent_activity`.

### `session_snapshot`

Current session snapshot plus current provider run snapshot.

Payload:

- `session`
- `provider_run`
- `agent_activity`

`agent_activity` is the canonical runtime-status source. When a prompt is active it includes
`active_turn` with the kernel prompt id, provider run id, and current prompt status. Clients must
not infer IDLE/WORKING from assistant text events.

The session snapshot is also how the CLI hydrates workflow definitions and other current workspace state on attach/rejoin.

Provider run token usage uses distinct fields for cumulative and context-window counts:

- `usage.total_tokens`: cumulative provider-run token usage
- `usage.last_tokens`: latest provider-reported turn usage
- `usage.context_tokens`: current model context occupancy when the provider reports a value that does not exceed the known context window
- `usage.context_window`: model context limit

### `session_unavailable`

Sent when the subscribed session is no longer available.

Payload:

- `session_id`
- `message`

### `heartbeat`

Liveness heartbeat for an active subscription.

Payload:

- `session_id`

### `transport_resumed`

Emitted after reconnect/resubscribe when the stream resumes.

Payload:

- `session_id`
- `resumed_from_event_id`

### `transport_closed`

This is emitted client-side by the CLI transport wrapper when the WebSocket closes.

Payload:

- `message`

## Recovery Semantics

The current transport supports:

- durable `event_id`
- reconnect/resubscribe
- `resume_from_event_id`
- bounded recent-event replay
- heartbeat/liveness

The kernel keeps a bounded recent-event history per session for replay on reconnect.

## Backpressure

The kernel enforces a bounded outgoing queue per connection.

If a client becomes too slow, the kernel closes the socket with policy close code `1008` and reason:

```text
kernel transport overloaded; reconnecting
```

The CLI can then reconnect and resubscribe with `resume_from_event_id`.

## Scope

This protocol page covers the current kernel-CLI local transport only.

It does not describe:

- future agent transports
- relay transport
- directory/discovery
- workflow run scheduling protocol
