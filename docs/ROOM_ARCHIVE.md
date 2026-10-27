# Reversible Room archive (MP-08 / MP-09 / MP-10 / MP-11)

Design contract; implementation and acceptance evidence are tracked separately in
this lane's PUSH_READY.md. Archive is not EndSession.

## Lifecycle and authority

Add `Archived` to the shared session lifecycle. The owner may archive a Created,
Active or Parked Room; unarchive returns it to Parked. Both operations are
idempotent. Ended Rooms cannot be unarchived. Existing EndSession, including its
legacy attach behavior (a new default agent), is unchanged; no historical agent
reconstruction is promised.

Use one shared `SetSessionArchived { session_id, archived }` request and response,
and `ListArchivedSessions` for discovery. Default lists omit Archived Rooms;
explicit archive lists and Room resolution/history still find them. Local,
relay, native TUI, Web and iOS use the same kernel path. Feature minimum: local
636 / peer 239; clients hide archive controls on older kernels and never fall
back to EndSession. This does not raise unrelated client minimum versions.

Only the Room owner (including the ordinary implicit OS-local owner) can change
archive state. Collaborators retain read access and membership; agents cannot
archive/unarchive a Room, including through sudo. Remote requests use existing
membership and authenticated owner admission. Placement does not alter policy.

The durable archive transition closes execution admission before asynchronous
cleanup. Success means cleanup completed. A failure reports the failed resource
and permits retry; it must never report a fully archived success while cleanup
is pending. Restart replay keeps the archive fence and reconciles interrupted
cleanup before permitting unarchive. Unarchive never creates an agent or
launches a turn. In-flight dispatch must recheck the lifecycle fence after waits.

## Kept and released

| Resource | Archive | Unarchive |
| --- | --- | --- |
| Room and agent definitions | Keep IDs, aliases, owners, immutable parentage, selected agent, profiles, configuration, arrangement and placement intent | Same objects |
| Provider thread IDs | Keep canonical IDs and transcript cursors; stop provider processes | Fresh official provider process uses saved thread when supported |
| History and files | Keep transcript, attachments on disk, task outcomes, workflow definitions, worktrees and workspace bindings | Immediately readable |
| Active turn | Stop through existing provider cancellation/process ownership; settle as cancelled with archive reason | No automatic retry; owner sends a new prompt |
| Queued prompts | Cancel, retain history/outcome and archive reason | Never replay stale queued work |
| Wakes and goals | Cancel live wakes and processes, retain receipts; stop automatic goal continuation | New owner action needed to resume work |
| Interactions | Withdraw process-bound decisions; record withdrawal | A new run asks a fresh decision if needed |
| App capability grants | Keep durable bindings; suspend use with the Room execution fence | Existing unexpired, non-revoked bindings usable again; no new authority |
| Terminals and provider processes | Close/reap through existing owners | Create fresh on demand; terminal transcripts retained |
| Worker/remote leases | Release and confirm through existing lease API; retain placement target, never a stale lease as live authority | Reacquire through ordinary placement admission, report unavailable worker; no silent local fallback |
| Browser/slices/environment | Stop Room-owned live environment; keep saved state, slice bindings and browser profile; detach live actors | Existing normal start path restores saved state on demand |
| Workflow runs/claims | Cancel running workflows and release claims; keep definitions, history and terminal outcomes | Explicit new run; no claim resurrection |
| Schedules/triggers | Keep definitions, suspend dispatch; elapsed occurrences do not accumulate for replay | Future occurrences only, under existing admission |
| Membership/collaborators | Keep owner, members and invite policy | Same membership |

Codex resumes the saved thread through the official app-server/CLI resume path.
Claude Code resumes its saved session through its official resume option.
OpenCode resumes its saved session through its official server/session path.
Each needs its original provider profile and accessible provider storage. An
external provider endpoint cannot be resurrected by Chariox: it must reconnect.
An unavailable/deleted provider thread produces an explicit resume error;
archive itself does not promise provider-owned data retention. These semantics
require provider-specific live validation; a saved ID alone is not proof.

## Visibility, accounting and recovery

Archived Rooms remain readable by links, history, search and prior notifications.
Opening one never implicitly unarchives it. It shows its history and an owner
Unarchive action in place; collaborators see the archived state. No new prompt,
App invocation, browser input, wake, trigger or notification-driven execution may
start while archived. Archive emits one lifecycle update; old notifications are
kept, and completion notifications from cleanup must not imply a successful turn.
Historical usage meters remain; stopped runtime resources stop accruing usage.
Cloud managed-machine shutdown policy remains authoritative and unchanged.
Remote kernels retain the same identity and history; worker availability is
reported at resource reacquisition, rather than replacing the Room.

## UI proposal — approval required

All new visible text and destinations below are PROPOSAL. Render a copy of the
existing OS prototype with byte-identical CSS and existing components only.

1. Add an `Archived` quick row in the side panel, using the existing navigation
   row and archive icon. It opens the existing Rooms overview/card layout,
   filtered to archived Rooms. Each existing card menu offers `Unarchive`.
2. Keep the existing Room footer settings button and Archive menu item. For an
   archived Room that same menu item reads `Unarchive`. Archive closes execution
   admission immediately and shows any cleanup error with the existing notice.
3. An archived Room stays open with its agent pane and transcript readable.
   Reuse the existing pane status stripe: `Archived · Unarchive to continue`.
   The Prompt bar is disabled with `Unarchive this Room to send a prompt`; no
   settings popup or new component. Unarchive is available in the footer menu.
4. Links/search/notifications open this same archived Room view. The archive
   overview has the existing empty-state presentation when no Rooms are archived.
5. Classic UI can reuse its existing Room action menu and list filtering after
   approval; no separate lifecycle implementation. Native clients follow the
   same feature minimum. Phone mocks at 400×900 are explicitly requested by this
   lane brief, superseding the general parked-phone rule for proposals only.

## Required focused acceptance

Archive while a real provider turn is running and another prompt is queued:
both settle definitely, provider processes/terminals/wakes/leases stop, and new
execution is refused. Restart the kernel, list archived Rooms, resolve the old
link, unarchive, and verify the same Room and agent IDs/parentage/configuration,
readable history, retained grants and fresh lease admission. Run provider-specific
resume checks. Exercise owner/collaborator/agent negative admission, cleanup
failure/retry and restart during cleanup, dispatch races, schedule suspension,
legacy EndSession and old-kernel feature gating. UI acceptance requires approved
controls on real built clients; proposal mocks are not acceptance.
