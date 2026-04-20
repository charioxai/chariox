# M8 History And Persistence Plan

Status: planned.

## Goal

Make Arroba own the operational history and durable runtime state needed to stop, restart, and recover local and remote work. Arroba should keep active/recent session UX fast and reliable while allowing long-term archiving to be delegated to user-provided storage systems through an adapter protocol.

## Product Decisions

- Arroba owns operational transcript history. The CLI, shell, and future clients ask the kernel for transcript chunks and render them; they do not own durable transcript storage.
- Operational history is always local for v1 so active/recent sessions, session resume, provider continuation, workflow recovery, prompt history, and recent search do not depend on an external archive service.
- Archive history is separate from operational history. Archive can be disabled, backed by an external adapter, or later backed by a local archive implementation.
- If archive is disabled, operational transcripts/history can disappear after the configured retention period. Search continues to work for retained operational history only.
- Arroba never deletes operational transcript content for an archived session/agent until the archive adapter confirms durable acceptance.
- Remote kernels are not auto-relaunched in v1. Users manually restart home and worker kernels; Arroba reconciles durable state when they come back.
- Git remains the source of truth for Git objects. Arroba stores commit pointers and searchable summaries, not full diffs.
- A small local archive index is v2. V1 does not keep a separate local search index for transcript content removed from operational history.

## Storage Model

### Operational History

Operational history is Arroba-owned and required.

Default v1 backend:

- SQLite on disk.
- WAL mode for crash recovery and concurrent readers/writers.
- Kernel-owned schema migrations.
- Indexed by session, agent, provider, model, turn, prompt, workflow, machine, repo, worktree, event kind, timestamp, and Git commit SHA where relevant.
- Full-text search over retained operational prompt/output/error text plus compact Git subjects and changed paths.

Operational history stores canonical Arroba events and is the backing source for `GetSessionHistory`.

### Archive History

Archive history is optional and adapter-backed.

Archive modes:

```toml
[history.archive]
mode = "disabled" # disabled | external
```

When disabled, Arroba may delete operational transcript/history after retention. Those deleted transcripts are gone from Arroba history/search.

When external, Arroba sends canonical events to the configured adapter:

```toml
[history.archive]
mode = "external"
url = "http://127.0.0.1:49300"
token_env = "ARROBA_HISTORY_TOKEN"
archive_deleted_agents = true
archive_before_delete = true
delete_operational_after_verified_archive = true
```

The adapter may be backed by Postgres, S3, ClickHouse, Elasticsearch, a company audit system, or anything else. Arroba only knows the adapter protocol.

Archive search is capability-negotiated:

- If adapter exposes search, Arroba can merge operational and archive search results.
- If adapter does not expose search, Arroba returns operational matches and reports that archived history is externally managed and not searchable through Arroba.
- Active/recent transcript UX never depends on archive search.

## User Config

History and persistence configuration lives in the Arroba TOML config file, currently resolved by the kernel as `~/.arroba/config.toml` or `$XDG_CONFIG_HOME/arroba/config.toml`.

Initial shape:

```toml
[history.operational]
backend = "sqlite"
path = "~/.arroba/history/operational.db"
retention_days = 30
max_size_mb = 5000
keep_pinned_sessions = true
archive_inactive_after_days = 30
archive_deleted_agents = true

[history.archive]
mode = "disabled"

[state]
backend = "sqlite"
path = "~/.arroba/state/kernel.db"
snapshot_interval_events = 1000
```

External archive example:

```toml
[history.archive]
mode = "external"
url = "http://127.0.0.1:49300"
token_env = "ARROBA_HISTORY_TOKEN"
archive_deleted_agents = true
archive_before_delete = true
delete_operational_after_verified_archive = true
require_durable_acceptance = true
```

## Event Schema

Canonical history events are provider-neutral and append-only.

Core fields:

```text
event_id
sequence
timestamp_ms
workspace_id
session_id
agent_id
agent_alias
provider
model
turn_id
prompt_id
provider_run_id
provider_session_id
workflow_id
workflow_run_id
workflow_node_id
machine_id
repo_root
worktree_path
kind
role
content
content_ref
metadata
candidate_agent_ids
candidate_prompt_ids
candidate_turn_ids
attribution_confidence
caused_by_event_id
```

Initial event kinds:

```text
user_prompt
provider_output
provider_reasoning
provider_tool
provider_error
provider_status
notice
session_created
agent_created
agent_moved
workflow_started
workflow_node_started
workflow_node_completed
mcp_granted
skill_granted
remote_machine_connected
remote_machine_disconnected
git_commit_detected
git_worktree_changed
git_worktree_dirty
git_worktree_clean
git_push_detected
```

Every event generated during an agent turn should include provider, model, turn id, and prompt id when available.

## Archive Adapter Protocol

The archive adapter is a user-run HTTP or local socket service.

Required endpoint:

```http
POST /arroba/history/events
```

Required semantics:

- Accept batches.
- Be idempotent by `event_id`.
- Return success only after durable write.
- Preserve event payloads exactly.
- Report accepted and rejected event ids.

Optional endpoints:

```http
GET  /arroba/history/capabilities
POST /arroba/history/query
POST /arroba/history/search
GET  /arroba/history/events/:event_id
```

Capabilities response:

```json
{
  "append": true,
  "query": true,
  "search": false,
  "full_text_search": false,
  "blob_refs": true
}
```

For v1, external archive adapters are not primary transcript stores. Operational history remains the active transcript source.

## Session And Agent Lifecycle

Session states:

```text
active
parked
archived
deleted
```

Agent history states:

```text
active
archived
deleted
```

Archiving can happen when:

- a session has not been opened for `archive_inactive_after_days`
- an agent is deleted and `archive_deleted_agents = true`
- a session is explicitly archived
- a session is explicitly deleted and `archive_before_delete = true`
- operational history exceeds configured size limits

For archived sessions/agents, Arroba keeps a small local stub:

```text
session_id / agent_id
alias
created_at
last_opened_at
archived_at
providers/models summary
archive_backend_id
archive_checkpoint/event range
worktrees
branches
last known status
```

Full transcript content can be removed from operational history only after verified archive, or immediately after retention expiry when archive is disabled.

## Conversation History Loading

`GetSessionHistory` remains the compatibility API for CLI transcript rendering, but its backend moves to operational history.

Clients should only request chunks:

```text
session_id
agent_id optional
cursor
limit / round count
```

The kernel owns pagination and transcript reconstruction. CLI and shell render returned entries and keep viewport state only.

## Durable Kernel State

Kernel state persistence is separate from transcript history.

Persist:

```text
sessions
agents
agent aliases
worktrees/directories
machines
remote machine records
workflows
workflow runs
workflow node runs
queues
pending prompts
MCP/skill grants
provider runs
provider resume descriptors
active/parked/interrupted state
```

Use state events plus compact snapshots:

- Append state-changing events.
- Periodically save snapshots.
- On boot, load latest snapshot and replay events after it.

## Boot Restore

On kernel start:

1. Acquire state-store lock.
2. Load durable config/state snapshot.
3. Replay state events after the snapshot.
4. Rebuild runtime projections.
5. Reopen operational history.
6. Reconcile provider processes.
7. Reconcile machines/workers.
8. Mark in-flight work as `resumable`, `recovering`, `interrupted`, `failed`, or `completed`.

Provider process memory is not guaranteed. Arroba guarantees its own state and best available provider resume.

## Provider Resume Descriptors

Provider-specific resume information is stored behind a provider-neutral wrapper:

```text
provider
provider_session_id
provider_run_id
model
effort
cwd
mcp_config_fingerprint
skill_grant_fingerprint
created_at
updated_at
```

Codex should use native resume ids. OpenCode uses equivalent support if available; otherwise runs fall back to transcript continuation or interrupted state.

## Remote Restart And Reconcile

V1 guarantee:

- Users manually restart home and worker kernels.
- Home owns canonical operational history and durable state.
- Worker reconnects and announces machine/kernel identity.
- Home reconciles remote agents, leases, grants, and pending work.
- Missing worker state surfaces as clear recovery/interrupted status.

Cases to support:

```text
home restarts while worker stayed up
worker restarts while home stayed up
both home and worker restart
relay disconnect/reconnect
remote agent idle during restart
remote agent active during restart
remote workflow active during restart
```

Worker keeps a small local WAL for events it generated but could not forward before disconnect/shutdown. Home deduplicates by event id.

## Git Observation

Arroba tracks Git activity without owning Git commands.

Before each agent turn, record:

```text
repo_root
worktree_path
branch
HEAD
dirty status summary
timestamp
agent_id
provider
model
turn_id
prompt_id
```

After each agent turn, compare:

```text
old HEAD -> new HEAD
git rev-list old..new
git status --porcelain
```

Emit:

```text
git_commit_detected
git_worktree_changed
git_worktree_dirty
git_worktree_clean
git_push_detected
```

For `git_commit_detected`, store searchable pointer data only:

```text
commit_sha
commit_subject
repo_root
worktree_path
branch
head_before
head_after
changed_paths
detected_at
session_id
agent_id if confident
provider
model
turn_id
prompt_id
candidate_agent_ids
candidate_prompt_ids
candidate_turn_ids
attribution_confidence
```

Do not store full diffs. Users can recover Git details from Git:

```bash
git show <sha>
git diff <sha>^ <sha>
git log --stat <sha>
```

Attribution levels:

```text
definite
likely
ambiguous
unattributed
```

If multiple agents could have caused a commit, store all candidates and show them in search results.

## Implementation Slices

### M8.1 Plan, Config, And Invariants

- Add this plan.
- Add TOML config structs for operational history, archive history, and durable state policy.
- Add config validation and `config set/unset` support for the new fields.
- Keep runtime behavior unchanged.

### M8.2 Canonical Event Types

- Add `HistoryEvent` and related enums/types.
- Add conversion from existing `SessionHistoryEntry` to canonical transcript events.
- Add event ids, sequence allocation, and metadata shape.

### M8.3 Operational SQLite Store

- Add SQLite backend for operational history.
- Enable WAL mode.
- Add migrations and indexes.
- Keep current JSONL store behind compatibility tests until cutover.

### M8.4 `GetSessionHistory` Cutover

- Serve `GetSessionHistory` from operational history.
- Preserve current pagination behavior.
- Verify CLI transcript rendering and prompt history still work.

### M8.5 History Query/Search API

- Add kernel query/search requests. **Landed:** `QueryHistory` and `SearchHistory` return canonical `HistoryEvent` rows from operational history with filters for session, agent, provider, model, workflow, machine, repo/worktree, event kind, text, sequence cursor, and bounded limits.
- Add shell/CLI `history` commands.
- Search retained operational history; include archive search only if adapter supports it.

### M8.6 Archive Exporter

- Add archive adapter client. **Landed foundation:** disabled and external archive clients now implement the adapter append/capabilities protocol, including durable acceptance validation and optional bearer-token auth from `history.archive.token_env`.
- Add durable outbox/checkpointing. **Landed:** operational SQLite now includes a `history_archive_outbox` table with idempotent enqueue, pending-load, failed-attempt recording, accepted marking, and reopen-safe checkpoint coverage. External archive mode queues new transcript events, and the one-shot exporter flushes pending events to the adapter while checkpointing accepted/rejected outcomes.
- Add archive-disabled retention behavior. **Landed store safety layer:** operational history can prune rows before a cutoff, either allowing unarchived deletion for disabled archive mode or requiring verified archive acceptance. Pruned-empty sessions get a marker that disables legacy JSONL fallback, preventing deleted history from reappearing through compatibility reads.
- Add external archive mode with verified acceptance before operational deletion.

### M8.7 Agent/Session Archival Lifecycle

- Add archived/deleted stubs.
- Archive deleted agents when policy says so.
- Archive inactive sessions by policy.
- Surface archive status in session/agent listings.

### M8.8 Durable Kernel State Store

- Add persistent state snapshots and state event log. **Landed foundation:** `DurableKernelStateStore` opens the configured SQLite path with WAL mode, appends ordered state events, saves snapshots, reloads events after a sequence, and loads the latest snapshot after reopening.
- Persist sessions, agents, workflows, grants, machines, queues, and provider resume descriptors. **Started:** `DaemonApp` now owns the durable state store. Session creation records `session.created` with the default agent payload, local agent spawn records `agent.created`, and session end records `session.ended`.

### M8.9 Boot Restore

- Rebuild projections from durable state.
- Mark interrupted/recovering work correctly.
- Validate local restart drills.

### M8.10 Remote Restart/Reconcile

- Add worker WAL and home dedupe for forwarded events.
- Reconcile manually restarted workers.
- Validate home restart, worker restart, and both restart drills.

### M8.11 Git Observation

- Add pre/post turn Git snapshots.
- Emit Git history events with provider, model, turn, prompt, and candidate attribution.
- Add search coverage by commit SHA, subject, changed path, branch, worktree, provider, model, and prompt text.

## Live Drills

- Local kernel restart restores sessions, agents, retained history, grants, workflows.
- CLI loads transcript from operational history after restart.
- Agent prompt before restart appears after restart.
- Provider output before restart is searchable.
- Codex resume descriptor survives restart.
- Home restart with worker alive.
- Worker restart with home alive.
- Both home and worker manually restarted.
- Remote workflow interrupted/recovered status is visible after restart.
- Git commit detected after an agent turn.
- Ambiguous commit attribution with two candidate agents.
- Search finds commit by subject, changed path, agent, provider, model, and prompt text.

## V2

- Portable Arroba kernel image export/import.
- Remote kernel automatic relaunch.
- Remote MCP install/version repair.
- Packaged archive adapters for common systems.
- Small local archive index for externally archived content.
- Stronger optional Git tracking through hooks or managed Git wrappers.
- Retention, encryption, redaction, and policy-based history capture.
