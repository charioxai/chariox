# External Provider Session Import Plan

## Goal

Allow Arroba to discover, display, import, and continue provider sessions that were created outside Arroba, while preserving the kernel as the authority for Arroba-managed runtime behavior.

The implementation should aim for live two-way integration when a provider exposes a stable attach/proxy/event shape. When that is not available, Arroba should fall back to observed-history integration and clearly mark which turns were observed outside the kernel prompt path.

## Definitions

- **External provider session**: A durable provider-owned thread/session/history that was created outside Arroba.
- **External provider process**: A live OS process or provider app-server instance. This may or may not exist for a durable provider session.
- **Imported Arroba session**: An Arroba session created from an external provider session.
- **Imported Arroba agent**: An Arroba agent created inside an existing Arroba session from an external provider session.
- **Observed external turn**: A provider turn detected from provider-native history or events that did not enter through Arroba's `SubmitPrompt` path.

The user imports provider sessions, not OS processes. OS processes are optional transport endpoints and must not be treated as Arroba-managed unless Arroba launched or proxied them through a supported runtime path.

## Capability Tiers

Each discovered external provider session should advertise capability flags rather than relying on provider-wide assumptions:

- `can_resume`
- `can_read_history`
- `can_watch_history`
- `can_attach_live`
- `can_proxy_permissions`
- `can_receive_hidden_context`
- `supports_workspace_live_sync`

The effective import mode is derived from those flags:

- **Tier 3, live attach**: Arroba can attach to or proxy the provider session so provider-native prompts and Arroba prompts are both projected through the kernel session as they happen.
- **Tier 2, observed history**: Arroba can read provider history at import time and keep observing imported external sessions for new provider-native turns. New external turns are projected into Arroba transcripts as observed records, but they are not kernel-managed prompts and cannot use kernel-owned prompt features.
- **Tier 1, resume only**: Arroba can create a new managed provider run using provider resume state, but cannot observe new external activity until the user interacts through Arroba.

## Provider Targets

### Codex

Target Tier 3 where the Codex app-server/WebSocket endpoint supports live proxying or event subscription.

Fallback to Tier 2 by discovering Codex thread history and importing by Codex thread id.

Expected behavior:

- Arroba-origin prompts continue the selected Codex thread through `ProviderResumeState::from_codex_thread_id`.
- If live attach is active, native Codex TUI prompts appear in Arroba TUI and web terminals.
- If live attach is unavailable, externally submitted turns appear as observed history when detectable.

Tier 3 feasibility:

- Status: conditionally feasible, spike required.
- Relevant provider shape: current Codex CLI exposes experimental `app-server`, `remote-control`, `--remote`, `resume`, and structured app-server primitives around threads, turns, and streamed items.
- Feasible case: an external Codex session is already running behind an attachable app-server/remote-control endpoint, or can be resumed onto one without replacing the provider-owned thread.
- Unproven case: an arbitrary already-open Codex TUI that is not connected to an attachable app-server. The spike must verify whether app-server broadcasts turns from one client to another and whether an ordinary TUI can be discovered and attached after startup.
- Not acceptable as Tier 3: launching a separate Arroba-owned Codex resume process while the native external TUI keeps running independently. That is Tier 2 plus kernel-managed continuation, not live attach.

Tier 3 implementation spike:

1. Generate or vendor the installed Codex app-server schema for the version Arroba supports.
2. Discover endpoints from `codex remote-control`, process args, configured Unix sockets, and explicit user-supplied URLs.
3. Connect as a second app-server client and resume the provider thread by id.
4. Submit Arroba prompts through the app-server turn API rather than through a separate CLI process.
5. Subscribe to turn/item notifications and project provider-native turns into the kernel transcript.
6. Verify whether prompts submitted from the native Codex TUI are broadcast to the Arroba client within the active polling/live threshold.
7. Map permission/tool approval events to `RuntimeInteraction` only if app-server exposes a supported bidirectional approval channel.
8. If any required broadcast or approval channel is absent, classify Codex external ordinary TUI sessions as Tier 2 while keeping Tier 3 available for app-server-backed sessions.

### OpenCode

Target Tier 3 when an external `opencode serve` endpoint or session API can be discovered or explicitly attached.

Fallback to Tier 2 by listing local OpenCode sessions and tailing readable events or history.

Expected behavior:

- Arroba-origin prompts continue the selected OpenCode session through `ProviderResumeState::from_opencode_session_id`.
- OpenCode should be the strongest Tier 3 candidate because it already has structured HTTP/session APIs.

Tier 3 feasibility:

- Status: likely feasible for server-backed sessions, spike required.
- Relevant provider shape: current OpenCode CLI exposes `opencode serve`, `opencode attach <url>`, `opencode session`, `opencode export`, `opencode run --session`, and `opencode web`.
- Feasible case: the external session lives in, or can be attached through, a running OpenCode server endpoint.
- Tier 2-only case: a file-only session with no live server or no discoverable event/prompt API. Reading `opencode export` or local session files can observe history but does not put Arroba behind the native TUI.

Tier 3 implementation spike:

1. Discover running OpenCode servers through explicit config, process args, mDNS where enabled, and recent known endpoints.
2. Authenticate using the provider's supported server password/token mechanism when configured.
3. Map server session ids to Arroba external session ids using the session list/export API.
4. Find and implement the live event channel, preferably WebSocket or server-sent events; if only polling is available, keep the mode at Tier 2 observed history.
5. Submit Arroba prompts through the OpenCode server session endpoint so native `opencode attach` clients and Arroba operate on the same provider session.
6. Project server events into kernel terminal history and attached clients.
7. Map permissions/tool requests into one kernel-owned `RuntimeInteraction` only if the server protocol exposes supported request and reply events.
8. Run a two-client attach drill: native `opencode attach` sends a prompt that Arroba sees, Arroba sends a prompt that native `opencode attach` sees, then both clients observe the same assistant output.

### Claude Code

Target Tier 2 for externally started Claude sessions.

Tier 3 should remain supported only for Arroba-launched native-PTY Claude flows unless Claude exposes a stable attachable UI/server seam. Arroba cannot retroactively own an external Claude PTY or hook bridge.

Expected behavior:

- Arroba-origin prompts continue the selected Claude session through `ProviderResumeState::from_claude_session_id`.
- External Claude turns are imported as observed transcript records when history can be read.
- The UI must not imply kernel-owned permission or hidden-context behavior for externally submitted Claude turns.

Tier 3 feasibility:

- Status: not feasible for ordinary external Claude TUI sessions with the currently confirmed stable shape; conditionally unknown for sessions started with Claude Remote Control.
- Relevant provider shape: current Claude CLI exposes `--remote-control [name]`, `--session-id`, `--resume`, and `--input-format stream-json` / `--output-format stream-json` only in print mode. The existing native TUI integration relies on Arroba-owned PTY and hook setup; that cannot be retroactively inserted into an already-running external TUI.
- Not feasible case: an ordinary external interactive Claude TUI. Arroba can resume/read history, but it cannot take ownership of that process' stdin/stdout, hidden context hook, or permission bridge after the process started.
- Conditional case: sessions started with `claude --remote-control`. This requires a dedicated protocol spike because the CLI advertises the mode, but the plan must prove there is a supported second-client prompt/event/permission channel before claiming Tier 3.
- Not acceptable as Tier 3: `claude --print --input-format stream-json --output-format stream-json` for an already-running external TUI. That is useful for Arroba-launched programmatic runs, but not for attaching behind a native interactive session started outside Arroba.

Tier 3 implementation spike:

1. Start Claude with `--remote-control <name>` and inspect its advertised endpoint, auth, process state, and session identity.
2. Determine whether a second client can attach to the same remote-control session without stealing the interactive TUI.
3. Verify second-client prompt submission, user-message replay, assistant stream replay, tool/permission events, abort, and disconnect/reconnect behavior.
4. Check whether hidden context can be supplied through a supported remote-control channel. If not, Tier 3 must explicitly disable hidden-context expectations for external Claude sessions.
5. If remote-control supports the full bidirectional surface, implement conditional Tier 3 only for remote-control-backed Claude sessions.
6. If remote-control cannot support second-client live attach, keep Claude external sessions at Tier 2 with optional hook-enhanced observation only when the user preconfigured those hooks before starting Claude.

## Tier 2 Observer Requirements

Discovery/index polling is not enough for Tier 2. Full Tier 2 requires a separate imported-session observer that follows each imported external provider session and projects new provider-native turns into all Arroba terminals attached to the importing session.

Polling cadence:

- Idle discovered-but-not-imported sessions: slow index refresh, for example 30 seconds.
- Imported sessions with no recent change: slower transcript refresh, for example 15 to 30 seconds.
- Imported sessions that are running or recently modified: active transcript refresh around 1 second.
- A session should remain active for a short window after a detected change, for example two minutes, then decay back to idle.
- Provider errors should use per-provider backoff without blocking other providers or the main app.

Kernel implementation:

- Run provider discovery and transcript reads outside the main app lock, using async provider APIs or blocking filesystem reads isolated from state mutation.
- Hold the kernel/app lock only to merge index state, append observed transcript entries, mark import metadata, and publish projections.
- Maintain durable cursors per imported agent: provider turn id when stable; otherwise file path plus byte offset, line number, content hash, and last observed timestamp.
- Deduplicate observed turns by provider turn id or a stable content merge key.
- If the same external provider session is imported into multiple Arroba sessions, each imported agent receives its own observed transcript projection for that provider session. The provider session is not owned exclusively by the first Arroba import.
- Publish observed turns through the same session/history/projection event paths used by ordinary terminal updates so TUI and web terminals refresh naturally.
- Clearly label observed turns in product UI as external/provider-observed.

Tier 2 acceptance:

- A new prompt sent in the native provider TUI after import appears in the imported Arroba TUI and web terminal without requiring manual refresh.
- The same event is marked observed, not kernel-managed.
- Arroba-origin prompts continue through the Arroba-managed resumed provider run and remain eligible for normal Arroba features.
- Permission prompts from external provider-native turns are not presented as answerable kernel interactions unless the provider exposes a supported reply channel.

## Kernel Architecture

Add a kernel-owned `ExternalProviderSessionIndex` service beside the existing managed provider process tracker.

Responsibilities:

- Periodically discover external provider sessions across supported providers.
- Normalize provider-specific session metadata into a shared projection.
- Merge and sort sessions by `last_modified_at desc`.
- Track whether an external session has already been imported.
- Store enough index state for deduplication and stable pagination cursors.
- Publish changes through the normal kernel projection/event path.
- Run a separate imported-session observer for transcript deltas after import.
- Use active and idle polling cadences so running sessions can project new turns quickly without forcing expensive scans for every idle provider session.

Normalized record:

```text
external_session_id
provider
provider_session_id
title
title_source
first_prompt_preview
created_at_ms
last_modified_at_ms
worktree_path
account_profile
running_state
capabilities
already_imported
imported_session_ids
imported_agent_ids
```

Provider adapter additions:

```text
list_external_sessions(cursor, limit)
read_external_session(provider_session_id)
watch_external_session(provider_session_id)
attach_external_live(provider_session_id)
resume_state_for_external_session(provider_session_id)
```

Discovery must be best effort. Provider failures should degrade that provider's external-session section without breaking waiting room rendering or ordinary Arroba session attachment.

## Protocol Changes

Add daemon requests:

```text
external_provider_session.list
external_provider_session.refresh
external_provider_session.import_session
external_provider_session.import_agent
external_provider_session.watch_status
```

Add response shapes:

```text
ExternalProviderSessionList
ExternalProviderSessionImportCreated
ExternalProviderSessionImportAgentCreated
ExternalProviderSessionWatchStatus
```

Pagination:

- Use opaque cursors.
- Default limit should be small enough for terminal rendering, for example 25.
- Sort globally across providers by `last_modified_at desc`.
- Return `has_more` and `next_cursor`.
- Keep Arroba session pagination independent from external provider session pagination.

Because this changes serialized protocol shape:

1. Increment the shared local daemon protocol version.
2. Update protocol snapshot/hash tests.
3. Update web/native minimum supported protocol version only where the client depends on the new behavior.
4. Add focused drills that exercise list, import-new-session, import-agent, and pagination behavior.

## Import Semantics

### Import As New Arroba Session

When the user selects an external provider session from the waiting room:

1. Kernel creates a new Arroba session.
2. Kernel creates the first agent.
3. Kernel launches the provider run with the provider-specific `ProviderResumeState`.
4. Kernel stores import metadata on the session, agent, and provider run.
5. Clients attach to the new Arroba session through the existing session attachment path.

Naming priority:

1. Provider title, if available.
2. First sentence of the first user prompt.
3. `<Provider> imported session`.

### Import As Agent Into Existing Arroba Session

When the user imports an external provider session into an existing Arroba session:

1. Kernel creates a new top-level agent in the current session.
2. Kernel launches the provider run with the provider-specific `ProviderResumeState`.
3. Kernel focuses the new agent only if requested by the client or current spawn policy.

Store import metadata:

```text
external_provider_session_ref
imported_at_ms
import_mode
capability_tier
title_source
```

### Safety Boundaries

Imported external sessions are not automatically managed workspace live sync sessions.

Workspace live sync managed mode can only be claimed when the active provider process was launched behind Arroba's runtime boundary and write fence. External observed turns must not be treated as Arroba-managed prompts for permissions, hidden context, MCP grants, or write-fence guarantees.

## TUI UX

### Waiting Room

The Join Existing Sessions popup should show two sections:

1. Arroba sessions.
2. External Provider Sessions.

External Provider Sessions columns:

```text
Provider
Title
Modified
Worktree
State
Mode
```

Mode values:

```text
Live
Observed
Resume only
Imported
Unavailable
```

Each table has its own `Load Older` action.

### Spawn Command

Extend `/agent spawn`:

```text
/agent spawn --external codex:<thread-id>
/agent spawn --external opencode:<session-id>
/agent spawn --external claude:<session-id>
```

The equivalent alias is:

```text
/agent spawn --external-session <provider>:<provider-session-id>
```

The command creates a new agent in the current Arroba session and continues the selected provider session. External-session import is a provider-session import path, so it should reject placement options such as `--slice`, `--machine`, `--kernel`, and explicit worktree placement.

Optional follow-up shorthand can be added after the main path is stable:

```text
/agent spawn --import <provider>:<provider-session-id>
/agent spawn --provider-session <id>
```

## Web UX

### Waiting Room

The web waiting room should use the same kernel projection as TUI:

- Arroba sessions first.
- External Provider Sessions below.
- Provider filter and pagination.
- Capability badge per row.

Selecting an external provider session imports it as a new Arroba session.

### Agent Creation Popup

Use two tabs:

```text
New Arroba Agent
Import External Session
```

The import tab reuses the external provider session table and pagination logic.

Selecting an external provider session imports it as a new agent in the current Arroba session.

## Transcript Behavior

Kernel-submitted Arroba turns render normally.

Observed external turns render in the same terminal transcript but carry source metadata:

```text
source = external_provider_observed
provider
provider_session_id
observed_at_ms
provider_turn_id
```

Clients should visually distinguish observed external turns without making them feel disconnected from the conversation.

Tier 3 live turns may render as live conversation turns, but the kernel should still preserve source metadata internally so diagnostics can explain where each prompt originated.

## Runtime Interaction Behavior

For Tier 3 providers, provider-native permission requests should resolve through one kernel-owned `RuntimeInteraction` when the provider seam supports it.

For Tier 2 providers, external permission prompts must not be projected as kernel-owned interactions unless Arroba can reliably answer them through a supported provider channel.

Claude external sessions should default to Tier 2 and should not imply Arroba can own an external Claude PTY permission prompt.

## Live Drill Matrix

Run the drill suite for each provider and surface:

Providers:

- Codex
- OpenCode
- Claude Code

Surfaces:

- TUI terminal
- Web terminal

### Drill 1: External Session Discovery

Start the provider outside Arroba, create a unique marker prompt, then verify:

- Kernel external-session list includes the session.
- Provider, title, modified time, resume id, and capability tier are present.
- Waiting room shows the row under External Provider Sessions.

### Drill 2: Waiting Room Import

Open Arroba waiting room, select Join Existing Sessions, select the external provider session, then verify:

- A new Arroba session is created.
- The first agent is created with the expected name.
- The provider run uses the selected provider session id.
- A prompt from Arroba continues the provider thread.

### Drill 3: Spawn Import

Attach to an existing Arroba session and run:

```text
/agent spawn --external <provider>:<provider-session-id>
```

Verify:

- A new agent appears in the session.
- The provider run resumes the selected provider session.
- The imported agent can receive Arroba prompts.
- Placement options are rejected for external-session imports.

### Drill 4: Web Waiting Room Import

Open the web waiting room and import an external provider session.

Verify:

- The same external session list is rendered.
- Pagination works.
- Selecting a row opens an Arroba session.
- Prompt submission continues the provider thread.

### Drill 5: Web Agent Popup Import

Inside an existing web session:

1. Open the create-agent popup.
2. Switch to Import External Session.
3. Select an external provider session.

Verify:

- A new agent pane appears.
- The provider run resumes the selected external session.
- The imported agent can receive Arroba prompts.

### Drill 6: Pagination

Seed more external provider sessions than one page across all providers.

Verify:

- The first page shows the newest sessions merged across providers.
- `Load Older` fetches the next page.
- No duplicates appear across pages.
- Arroba session pagination and external session pagination are independent.

### Drill 7: Tier 3 Live Coherence

Run only where `can_attach_live` is true.

Verify:

- Prompt from provider-native TUI appears in Arroba TUI.
- Prompt from provider-native TUI appears in web terminal.
- Prompt from Arroba TUI appears in provider-native TUI.
- Prompt from web terminal appears in provider-native TUI.
- The same marker text appears across all active surfaces.

### Drill 8: Tier 2 Observed History

Run where Tier 3 is unavailable or disabled.

Verify:

- Prompt submitted outside Arroba is detected.
- Arroba displays it as an observed external turn.
- The transcript clearly marks the turn source.
- Arroba does not claim managed prompt features for that external turn.

### Drill 9: Permissions And Safety

For Tier 3 providers:

- Trigger a provider permission request.
- Verify exactly one kernel-owned `RuntimeInteraction`.
- Verify TUI and web both see the same interaction.
- Verify first valid answer resolves the provider request.

For Tier 2 providers:

- Trigger or simulate an external permission prompt.
- Verify Arroba does not present it as a resolvable kernel-owned interaction unless a supported provider channel exists.

### Drill 10: Workspace Live Sync Boundary

Import an external provider session and verify:

- Managed workspace live sync is disabled or downgraded unless Arroba launched the active process behind the write fence.
- Observed external turns are not treated as write-fenced managed turns.
- The UI and runtime metadata expose the downgrade reason.

## Screenshot And Evidence Collection

All evidence must be written under:

```text
./.artifacts/external-provider-sessions/<timestamp>/<provider>/<surface>/
```

The `.artifacts/` directory is intentionally ignored by git.

Each drill should collect product screenshots, not generated evidence cards:

- Provider-native TUI before import.
- TUI waiting room external table.
- TUI imported session terminal.
- TUI `/agent spawn --external` result.
- Web waiting room external table.
- Web import popup tab.
- Web imported terminal.
- Live coherence proof screenshots showing the same marker in provider-native, TUI, and web when Tier 3 is supported.
- Tier 2 observed-history screenshots showing observed source labeling.
- JSON manifest with provider ids, Arroba session ids, agent ids, provider session ids, capability tier, timestamps, screenshot paths, and assertion results.

Example manifest:

```json
{
  "provider": "opencode",
  "surface": "web",
  "drill": "tier3-live-coherence",
  "capability_tier": "live",
  "external_provider_session_id": "opencode:session-123",
  "provider_session_id": "session-123",
  "arroba_session_id": "session-abcd",
  "agent_id": "agent-efgh",
  "marker": "ARROBA_EXTERNAL_SESSION_DRILL_20260609_001",
  "screenshots": [
    ".artifacts/external-provider-sessions/20260609T120000Z/opencode/web/waiting-room.png",
    ".artifacts/external-provider-sessions/20260609T120000Z/opencode/web/imported-terminal.png"
  ],
  "assertions": [
    {
      "name": "external session listed",
      "passed": true
    },
    {
      "name": "marker visible in web terminal",
      "passed": true
    }
  ]
}
```

## Acceptance Criteria

The feature is complete when:

- Codex, OpenCode, and Claude external sessions can be discovered.
- External sessions render in TUI and web waiting room under Join Existing Sessions.
- Large external session lists paginate correctly.
- Import as new Arroba session works in TUI and web.
- Import as agent works through `/agent spawn` and the web create-agent popup.
- Codex and OpenCode attempt Tier 3 when capability flags allow it and degrade cleanly to Tier 2 otherwise.
- Claude supports Tier 2 without implying ownership of external PTY prompts or permission flows.
- Observed external turns are visible in Arroba terminals with source metadata.
- Kernel-submitted Arroba turns continue imported provider sessions through existing provider resume state.
- Protocol version, protocol snapshots, and focused tests are updated.
- End-to-end live drills pass for all providers and both TUI and web surfaces.
- Screenshot evidence and drill manifests are collected under `./.artifacts`.
