# External Provider Session Tier 2 Observer Plan

## Goal

This document extends `docs/EXTERNAL_PROVIDER_SESSION_IMPORT_PLAN.md` and supersedes its Tier 3 adoption work for the next implementation phase.

The current target is Tier 2 only: Arroba discovers external provider sessions, imports one as a new Arroba session or as a new agent, continues it through provider resume state for Arroba-submitted prompts, and continuously observes new provider-native turns as `external_provider_observed` transcript records.

Tier 3 live attach, native TUI reparenting, and provider prompt interception are out of scope for this phase.

## Non-Goals

- Do not claim ownership of an already-running provider TUI.
- Do not route prompts typed in an external provider TUI through Arroba `SubmitPrompt`.
- Do not present provider-native permission prompts from external turns as answerable Arroba `RuntimeInteraction`s.
- Do not require external native TUIs to display Arroba-submitted turns.
- Do not implement Codex/OpenCode/Claude live attach or proxy adoption in this phase.

## User-Facing Mode Labels

External provider session rows should avoid Tier 3 language for now.

Allowed labels:

- `Observed`
- `Resume only`
- `Imported`
- `Unavailable`

The UI must not show `Live` for external sessions until a separate Tier 3 implementation exists.

## Persistent Import Metadata

Persist import metadata on the Arroba session, imported agent, and provider run where applicable:

```text
external_provider_session_id
external_provider
external_provider_session_provider_id
import_mode = observed_history | resume_only
observed_cursor
last_observed_turn_id
last_observed_at_ms
imported_at_ms
```

This metadata must survive kernel restart so the observer can resume without duplicating observed transcript entries.

## Kernel Services

### Waiting-Room Discovery

The existing external-session discovery service powers waiting-room inventory.

Responsibilities:

- Scan all supported providers.
- Normalize provider metadata.
- Sort/paginate external sessions for TUI and web.
- Mark known imported sessions.

Cadence:

- Default poll can remain around 30 seconds.
- Provider failures should degrade only that provider's section.

### Imported Transcript Observer

Add a separate kernel-owned observer for imported sessions.

Responsibilities:

- Watch only imported external provider sessions.
- Read provider history/export state for those sessions.
- Maintain provider-specific cursors.
- Deduplicate already observed turns.
- Append new turns as `external_provider_observed`.
- Emit normal session/history projection updates so attached TUI and web terminals refresh through existing paths.

Cadence:

- Active imported sessions: poll around 1 second.
- Recently changed imported sessions: stay active for a short window, for example two minutes.
- Idle imported sessions: decay to 15-30 second polling.
- Provider read failures: apply per-provider or per-session backoff without blocking the main app or other providers.

Concurrency:

- Provider filesystem/API reads must run outside the main app lock.
- The app lock should only be held to merge cursor state, append transcript entries, persist metadata, and publish projections.

## Provider Cursor Strategy

Use provider-specific stable cursors where possible.

Codex:

- Cursor by thread id plus item/turn id when available.
- Fallback to history file path plus offset, line number, content hash, and timestamp.

OpenCode:

- Cursor by session id plus message id or part id when available.
- Fallback to export/history ordering plus content hash and timestamp.

Claude:

- Cursor by transcript path plus event/message uuid when available.
- Fallback to JSONL line offset, content hash, and timestamp.

Every cursor update must be persisted after observed entries are appended.

## Transcript Records

Observed external turns should be appended through the normal history/projection path with source metadata:

```text
source = external_provider_observed
provider
provider_session_id
provider_turn_id
observed_at_ms
```

Rendering requirements:

- TUI and web should show observed external turns inline in the imported agent transcript.
- The source should be visibly labeled as external/provider-observed.
- Observed turns must not be rendered as active kernel prompts.
- Observed turns must not unlock kernel-managed prompt features such as hidden context, Arroba permissions, attachments, workspace live sync guarantees, MCP grants, or workflow controls.

## Import Semantics

### Import As New Arroba Session

When a user selects an external provider session from the waiting room:

1. Kernel creates a new Arroba session.
2. Kernel creates the first agent.
3. Kernel stores persistent external import metadata.
4. Kernel imports existing readable provider history as observed transcript entries.
5. Kernel starts the imported transcript observer for the imported agent.
6. Kernel launches/resumes a provider run only for Arroba-submitted continuation through provider resume state.
7. Clients attach through the existing session attachment path.

### Import As Agent Into Existing Arroba Session

When a user imports an external provider session into an existing Arroba session:

1. Kernel creates a new top-level agent in the current session.
2. Kernel stores persistent external import metadata.
3. Kernel imports existing readable provider history as observed transcript entries for that agent.
4. Kernel starts the imported transcript observer for that imported agent.
5. Kernel launches/resumes a provider run only for Arroba-submitted continuation through provider resume state.

TUI command support:

```text
/agent spawn --external <provider>:<provider-session-id>
/agent spawn --external-session <provider>:<provider-session-id>
```

Add this alias for the user-facing import wording:

```text
/agent spawn --import <provider>:<provider-session-id>
```

The command should reject placement options such as `--slice`, `--machine`, `--kernel`, and explicit worktree placement for external-session imports.

### Reimport Policy

The same external provider session may be imported into multiple Arroba sessions.

Behavior:

- Each Arroba imported agent gets its own transcript projection and cursor state.
- New external observed turns should appear in every Arroba import that watches that provider session.
- Arroba should not create one shared Arroba agent that responds in multiple Arroba sessions.
- Arroba-submitted prompts continue from the imported agent where they were submitted.

## Product UX

### TUI

Waiting room:

- Show external provider sessions under `Join Existing Sessions`.
- Keep Arroba sessions and external sessions as separate tables.
- Keep independent pagination with `Load Older`.

Session terminal:

- Show imported history.
- Show newly observed provider-native turns with source labeling.
- Show Arroba-submitted continuation turns normally.

Spawn:

- `/agent spawn --external ...` and `/agent spawn --import ...` should import an external provider session as a new agent.

### Web

Waiting room:

- Render the same external provider session projection as TUI.
- Importing a row creates a new Arroba session.
- Pagination should match the kernel cursor contract.

Create-agent popup:

- Keep two tabs:
  - `New Arroba Agent`
  - `Import External Session`
- The import tab reuses the external session table and pagination.
- Selecting an external session imports it as a new agent in the current Arroba session.

## Validation Drills

Run the drills for Codex, OpenCode, and Claude across TUI and web.

### Drill 1: External Discovery

1. Start or seed a provider session outside Arroba.
2. Create a unique marker prompt.
3. Verify waiting-room inventory lists the external provider session.
4. Verify provider, title, modified time, provider session id, and mode are present.

### Drill 2: Import As New Session

1. Import the external provider session from the waiting room.
2. Verify Arroba creates a new session and first agent.
3. Verify existing readable provider history appears as observed transcript entries.
4. Submit a prompt from Arroba.
5. Verify the provider run continues using provider resume state.

### Drill 3: Import As Agent

1. Attach to an existing Arroba session.
2. Run:

```text
/agent spawn --import <provider>:<provider-session-id>
```

3. Verify a new agent appears.
4. Verify existing provider history appears for that agent.
5. Verify Arroba-submitted prompts continue through provider resume state.

### Drill 4: New External Turn Observation

1. Import an external provider session.
2. After import, append or produce a new provider-native turn outside Arroba.
3. Verify the imported Arroba TUI terminal updates without manual refresh.
4. Verify the imported web terminal updates without manual refresh.
5. Verify the new turn is labeled `external_provider_observed`.

### Drill 5: Reimport Behavior

1. Import the same external provider session into two Arroba sessions.
2. Produce a new external provider-native turn.
3. Verify both Arroba imports observe the new turn.
4. Submit an Arroba prompt in only one imported agent.
5. Verify only that Arroba agent/session owns the Arroba-submitted continuation.

### Drill 6: Pagination

1. Seed more external sessions than one page across providers.
2. Verify newest sessions are globally sorted by modified time.
3. Verify `Load Older` fetches the next page without duplicates.
4. Verify Arroba session pagination and external session pagination remain independent.

### Drill 7: Safety Boundary

1. Trigger or simulate a provider-native permission prompt from an external turn.
2. Verify Arroba does not present it as a resolvable kernel-owned interaction.
3. Verify observed external turns do not claim workspace live sync managed mode, hidden context, MCP grants, or attachment handling.

## Screenshot Evidence

All evidence must be real product UI screenshots, not generated evidence cards.

Store evidence under:

```text
./.artifacts/external-provider-sessions-tier2/<timestamp>/<provider>/<surface>/
```

Required screenshots per provider:

- TUI waiting room external table.
- TUI import-as-session terminal with observed history.
- TUI `/agent spawn --import` result.
- TUI imported-agent terminal with newly observed external turn.
- Web waiting room external table.
- Web create-agent import tab.
- Web imported terminal with newly observed external turn.

Each drill should also write a JSON manifest containing:

```text
provider
surface
drill
external_provider_session_id
provider_session_id
arroba_session_id
agent_id
marker
observed_cursor_before
observed_cursor_after
screenshot_paths
assertion_results
```

## Test Plan

Kernel tests:

- Import metadata persists on session, agent, and provider run.
- Observer restores cursor state after restart.
- Observer deduplicates repeated provider history reads.
- Observer appends only new turns.
- Observer emits normal session/history projections.
- Active imported sessions poll faster than idle sessions.
- Provider read errors back off without blocking other providers.
- Same external session imported into two Arroba sessions receives separate transcript projections.

CLI/TUI tests:

- Waiting room renders external table and `Load Older`.
- `/agent spawn --external` imports as an agent.
- `/agent spawn --import` alias imports as an agent.
- Placement options are rejected for external imports.
- Observed entries render with external source labeling.

Web tests:

- Waiting room renders external sessions and pagination.
- Import-as-session sends the correct kernel request.
- Create-agent popup import tab sends import-agent request.
- Observed entries render with external source labeling.

Protocol tests:

- Update protocol version and shape snapshots if new serialized fields or requests are added.
- Add focused request/response snapshot coverage for import metadata and observer status if exposed.

## Acceptance Criteria

Tier 2 is complete when:

- Codex, OpenCode, and Claude external sessions can be discovered.
- External sessions render in TUI and web waiting rooms.
- External session pagination works independently from Arroba session pagination.
- Import as new Arroba session works.
- Import as agent works through TUI and web.
- Existing provider history is imported as observed transcript entries.
- New external provider-native turns after import appear in attached Arroba TUI and web terminals without manual refresh.
- Observed external turns are labeled and carry source metadata.
- Arroba-submitted prompts continue the imported provider session through resume state.
- External native TUIs are not claimed to receive Arroba-submitted turns.
- Provider-native permission prompts from external turns are not projected as answerable Arroba interactions.
- Product screenshots and drill manifests are collected under `./.artifacts`.
