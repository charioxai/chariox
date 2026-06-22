# External Provider Live Transcription Plan

## Goal

Make Arroba track provider sessions created outside Arroba, show their prompts
and outputs in the linked Arroba session/agent, and allow the user to continue
the same provider thread from Arroba without losing context.

The end state is live transcription whenever a provider exposes a live stream,
attach, proxy, PTY, or hook surface that Arroba can control. Persisted provider
history remains the recovery and discovery layer, but it is not sufficient as
the only live-output source when the provider stores only completed messages.

This plan extends:

- `docs/EXTERNAL_PROVIDER_SESSION_IMPORT_PLAN.md`
- `docs/EXTERNAL_PROVIDER_SESSION_TIER2_OBSERVER_PLAN.md`
- `docs/M24_CLAUDE_HEADLESS_PLAN.md`

## Feasibility Probe Results

The probes ran on 2026-06-22 with unique marker prompts. They monitored the
provider-owned storage while the provider process was active and inspected only
metadata, record types, lengths, and timestamps.

### Codex

Surface tested:

```text
codex exec --json -C <workspace> <prompt>
```

Storage:

```text
~/.codex/sessions/YYYY/MM/DD/*.jsonl
```

Observed writes for one 22.3s run:

```text
0.6s   2 lines  session metadata/task start
5.0s   7 lines  prompt/context records
7.4s   8 lines  reasoning record
21.9s  9 lines  agent message event
22.0s 12 lines  assistant message, token count, task complete
```

Conclusion:

- Codex JSONL is not a token-delta transcript.
- It can reveal session start, user prompt, in-flight state, reasoning metadata,
  final assistant output, and completion.
- Full live assistant transcription requires Codex app-server events or a proxy
  in front of the client/server stream.

### OpenCode

Surface tested:

```text
opencode run --dir <workspace> --format json --title <marker> <prompt>
```

Storage:

```text
~/.local/share/opencode/opencode.db
~/.local/share/opencode/opencode.db-wal
```

Current OpenCode persisted the run in SQLite tables:

```text
session
message
part
event
```

The JSON file under `storage/session_diff` was not the transcript authority for
the tested version.

Observed writes for one 23.6s run:

```text
1.0s   session, user message, assistant shell row
2.4s   step-start and reasoning parts
3.6s   reasoning part length changed, text part created with small length
23.6s  text part jumped to final length, step-finish created
```

Conclusion:

- OpenCode storage is DB/WAL-backed in the current local version.
- The DB can be watched for session/message/part changes.
- The DB did not prove continuous token-level growth; it gave early activity,
  partial part creation, then final content.
- Full live assistant transcription should use OpenCode server/SSE events or an
  Arroba proxy when a server endpoint is available.

### Claude -p

Surface tested:

```text
claude -p \
  --input-format text \
  --output-format stream-json \
  --include-partial-messages \
  --verbose \
  --session-id <uuid> \
  --permission-mode plan \
  <prompt>
```

Storage:

```text
~/.claude/projects/<cwd-slug>/<session-id>.jsonl
```

Observed writes for one 15.4s run:

```text
0.9s   7 lines  queue/user/attachment records
2.3s   8 lines  ai-title
15.0s 12 lines  assistant thinking/text, last-prompt, ai-title
```

Conclusion:

- `claude -p` can stream partial messages on stdout.
- Its persisted JSONL does not store those partial chunks as deltas.
- Full live transcription for Arroba-owned `claude -p` comes from stdout
  stream-json parsing.
- Externally launched `claude -p` can be observed from files only at coarse
  history boundaries unless its stdout is wrapped by Arroba from launch.

### Claude Headless

Surface tested:

```text
script -q /dev/null claude \
  --session-id <uuid> \
  --permission-mode plan \
  <prompt>
```

This simulates the planned hidden PTY surface. The first run in `/tmp` was
blocked by Claude workspace trust and produced no transcript. The trusted
workspace run produced a transcript.

Storage:

```text
~/.claude/projects/<cwd-slug>/<session-id>.jsonl
```

Observed writes for one interactive PTY run:

```text
0.3s    2 lines  mode and permission mode
0.5s    9 lines  user and attachment records
5.5s   10 lines  ai-title
12.2s  13 lines  assistant thinking/text and system record
70.1s  17 lines  last-prompt/title/mode records after PTY termination
```

Conclusion:

- Claude headless JSONL is not a token-delta transcript.
- Arroba-owned headless live output must come from the kernel-owned PTY,
  hook bridge, and transcript tailer together.
- Externally launched interactive Claude sessions can be observed from files
  for prompt/final output, but cannot be retroactively made fully kernel-owned
  unless the process was launched with an attachable/hookable control surface.

## Core Design Decision

Use two layers, not one:

1. **Provider storage observer**
   - Discovers external sessions.
   - Imports existing history.
   - Watches imported sessions for new externally-originated turns.
   - Recovers missed records after disconnect/restart.
   - Labels records as `external_provider_observed`.

2. **Live stream adapter**
   - Used when Arroba owns, wraps, proxies, or attaches to a live provider
     stream.
   - Emits actual live deltas/tool events/status into the Arroba session.
   - Converts provider-native prompts into `SubmitPrompt` only when Arroba is
     truly in the prompt path.

Persisted files alone are not enough for the ultimate live-transcription goal
for Codex, Claude -p, or Claude headless. OpenCode's DB provides the richest
storage signal, but the probe still did not prove complete token-delta storage.
Therefore the implementation must pursue live taps/proxies first and use
storage as a fallback, not redefine "live" to mean "after completion."

## Origin And Control Semantics

Every transcript record projected into Arroba must preserve origin:

```text
origin = arroba_managed | external_observed | external_live_attached
provider
provider_session_id
provider_item_id
provider_turn_id
observed_at_ms
live_stream_id
```

Rules:

- `arroba_managed`: prompt entered through Arroba `SubmitPrompt`; all Arroba
  features apply.
- `external_observed`: detected from provider storage/export after provider
  execution; label visibly as external; no Arroba hidden context, permission,
  MCP grant, or workspace live-sync guarantees.
- `external_live_attached`: provider events are streamed live to Arroba, but
  prompts are still not kernel-managed unless the provider-native client is
  reparented through an Arroba proxy or hook.

## Provider Implementation Plan

### Codex

Required paths:

1. Storage observer for `~/.codex/sessions` and `archived_sessions`.
2. App-server observer for thread history and live notifications when an
   app-server endpoint is available.
3. Reparented proxy path using the existing `arroba codex` machinery.

Work:

- Replace the current first-300-lines Codex reader with an incremental JSONL
  reader keyed by thread id, path, byte offset, line number, and item ids.
- Preserve and parse Codex record types:
  - `session_meta`
  - `event_msg.task_started`
  - `event_msg.user_message`
  - `response_item.message`
  - `response_item.reasoning`
  - `event_msg.agent_message`
  - `event_msg.task_complete`
  - tool and command records when present.
- Add an app-server live observer:
  - connect to explicit/discovered `codex app-server` endpoints;
  - call `thread/resume` or `thread/read` for the imported thread;
  - subscribe to `item/started`, `item/completed`,
    `item/agentMessage/delta`, `turn/completed`, and status notifications.
- Keep the current native TUI proxy as the only full interception path:
  - `codex --remote <arroba-proxy>`;
  - proxy catches `turn/start`;
  - proxy submits through kernel `SubmitPrompt`;
  - kernel projects output back to Codex and Arroba clients.
- For Codex Desktop, attempt only supported endpoint/control discovery. If the
  desktop app does not expose a documented attach/proxy target, classify
  direct desktop turns as observed storage/app-server events, not full
  interception.

### OpenCode

Required paths:

1. SQLite observer for `~/.local/share/opencode/opencode.db` and WAL changes.
2. OpenCode server/SSE observer when a running `opencode serve` endpoint is
   discoverable or explicitly supplied.
3. Reparented proxy path using the existing `arroba opencode` machinery.

Work:

- Add OpenCode DB discovery:
  - read `session`, `message`, `part`, and `event`;
  - map `session.id` to external session id;
  - use `time_updated`, message id, part id, and content hash as cursor data.
- Update current OpenCode external discovery because JSON/JSONL scan is not
  enough for current OpenCode storage.
- Watch DB/WAL with filesystem notifications and active polling fallback.
- Parse part types:
  - `text`
  - `reasoning`
  - `step-start`
  - `step-finish`
  - tool/permission/event part types seen in real sessions.
- Add server/SSE observation:
  - discover running endpoints from process args, explicit user config,
    mDNS when enabled, and recent known endpoint cache;
  - authenticate with configured OpenCode server password/token;
  - subscribe to session events;
  - stream part deltas when the server emits them.
- Use existing OpenCode proxy for full interception:
  - `opencode attach <arroba-proxy> --session <session-id>`;
  - proxy converts native prompt requests into `SubmitPrompt`;
  - server/SSE output fans out to Arroba.

### Claude -p

Required paths:

1. JSONL observer for external `~/.claude/projects/.../*.jsonl`.
2. Stream-json stdout parser for Arroba-owned or Arroba-wrapped `claude -p`.

Work:

- Keep the existing `claude -p` provider adapter as the primary live path for
  Arroba-managed Claude -p turns.
- Add an external JSONL observer for provider-native sessions:
  - cursor by transcript path and message uuid;
  - parse `user`, `assistant`, `system`, `attachment`, `queue-operation`,
    `last-prompt`, and `ai-title`;
  - label externally-originated records.
- For externally launched `claude -p`, full live transcription requires the
  process stdout stream. Provide an optional wrapper mode:
  - `arroba observe claude-p -- <claude -p args...>` or equivalent;
  - launches Claude exactly as requested;
  - tees stream-json stdout to the terminal/caller and the kernel observer;
  - binds the discovered Claude session id to an imported Arroba agent.
- Do not claim full live transcription for a `claude -p` process that was
  already launched without Arroba wrapping and whose stdout is not available.

### Claude Headless

Required paths:

1. JSONL observer for external interactive Claude sessions.
2. Kernel-owned hidden PTY driver for Arroba-managed headless.
3. Hook bridge for prompt submit, stop, permissions, and hidden context.

Work:

- Complete the `Claude headless` architecture from
  `docs/M24_CLAUDE_HEADLESS_PLAN.md`.
- Use a kernel-owned PTY for managed headless turns:
  - wait for prompt readiness;
  - type Arroba prompts;
  - capture PTY output for status/debug/fallback;
  - keep primary transcript projection through hook/transcript parsing.
- Tail Claude JSONL continuously:
  - parse user, assistant thinking, assistant text, system, attachment,
    permission, and hook-related records;
  - dedupe by uuid and transcript cursor;
  - use `Stop` hook as a completion signal, not as the only output source.
- For externally launched interactive Claude:
  - observe JSONL for prompt/final output;
  - do not present permissions as Arroba-owned unless the external process was
    launched with an Arroba-compatible hook bridge;
  - do not claim hidden context or MCP grants apply.
- Investigate Claude `--remote-control` only as a separate attach surface. It
  must prove prompt, output, permission, abort, resume, and hidden-context
  channels before it can be classified above observed-history.

## Kernel Services

### External Provider Session Index

Extend the existing indexer so providers can use structured backends:

```text
Codex: JSONL session tree
OpenCode: SQLite database plus WAL, server API when available
Claude: JSONL project transcripts
```

Fixes required:

- Sort all provider candidates by modified time before applying caps.
- Remove hard first-300-line limits for imported sessions.
- Keep discovery caps only for broad waiting-room scans, not targeted reads.
- Store path/database identity in the record so targeted observers do not scan
  the whole provider state tree every second.

### Imported Session Observer

Replace whole-file reread/dedupe with provider cursors:

```text
provider
provider_session_id
source_kind = file | sqlite | app_server | server_sse | pty | stdout
source_ref
byte_offset
line_number
sqlite_table
sqlite_row_id
provider_item_id
provider_message_id
provider_turn_id
content_hash
updated_at_ms
```

The observer must emit two classes of records:

- activity/status records when a new external turn starts or changes;
- transcript records when user/assistant/tool/reasoning content is available.

### Live Stream Fanout

Do not rely on a generic `external_provider_history_updated` sentinel for live
output. The kernel should fan out actual terminal records where possible:

```text
provider_status
user_prompt
provider_output_delta
provider_output_final
reasoning_delta
tool_call
tool_output
permission_observed
turn_completed
```

`recoverHistory()` remains a resync fallback for clients that miss events.

## Product UX

- External sessions visible in TUI and web waiting rooms.
- Imported agent transcript shows observed records inline.
- External-origin user prompts get a compact label such as `external`.
- Provider output records get provider-aware labels:
  - `codex observed`
  - `opencode observed`
  - `claude observed`
- When live attached but not intercepted, use a label such as
  `external live`.
- When a prompt is sent from Arroba into the imported agent, the transcript
  returns to normal managed Arroba styling for that turn.
- Agent/session details should show the provider session id and current
  external integration mode:
  - `observing files`
  - `observing database`
  - `attached to server events`
  - `proxied/native managed`
  - `resume only`

## Drills

### Feasibility Drill

Automate the probes from this plan:

```text
apps/cli/scripts/live-external-provider-storage-feasibility-drill.mjs
```

Providers:

- Codex
- OpenCode
- Claude -p
- Claude headless

Assertions:

- create a unique marker prompt;
- identify the provider session id and storage source;
- record mutation timestamps while the provider turn is active;
- classify the storage stream:
  - `delta`
  - `partial`
  - `status-only`
  - `final-only`
  - `none`
- write a JSON manifest under `.artifacts`.

### External Observation Drill

For each provider:

1. Create a provider session outside Arroba.
2. Import it as an Arroba session.
3. Attach Arroba TUI and web terminal.
4. Submit another external provider-native prompt.
5. Verify the imported Arroba agent updates without manual refresh.
6. Verify the prompt/output are labeled external.
7. Submit a prompt from Arroba.
8. Verify the provider continues the same provider session id.
9. Verify the Arroba-origin turn is managed, not external.

### Live Attach Drill

For providers with a live attach path:

- Codex through app-server/proxy.
- OpenCode through server/SSE/proxy.
- Claude -p through Arroba-owned stdout wrapper.
- Claude headless through Arroba-owned PTY/hook bridge.

Assertions:

- live deltas/tool events appear in Arroba before the provider turn completes;
- final history matches provider storage after completion;
- reconnect/recover does not duplicate records;
- permission prompts are answerable only when the provider channel supports
  replies through Arroba.

### Browser Drill

Use real Aruba web UI:

1. Launch kernel on the browser-terminal dev port.
2. Create or select an external provider session.
3. Open Aruba web waiting room.
4. Verify external session row appears.
5. Import external session.
6. Observe external prompt/output arrive in the imported agent pane.
7. Send Arroba prompt from the browser terminal.
8. Verify the same provider session id continues.
9. Capture screenshots and JSON manifests.

## Acceptance Criteria

The live transcription work is complete only when:

- Codex, OpenCode, Claude -p, and Claude headless external sessions are
  discoverable from current provider storage.
- OpenCode discovery supports current SQLite/WAL storage.
- Imported external sessions update attached TUI and web terminals without
  manual refresh.
- Storage observers recover final provider history after disconnect/restart.
- Live stream adapters emit real pre-completion output where the provider
  exposes a stream that Arroba can own, wrap, proxy, or attach to.
- Provider records preserve origin and are labeled correctly in UI.
- Arroba-origin prompts continue the same provider session as managed turns.
- External-origin prompts are never misrepresented as kernel-managed turns.
- Permission and hidden-context behavior is only enabled when Arroba is in a
  supported provider control path.
- End-to-end drills produce artifacts for all providers and both TUI/web
  surfaces.

