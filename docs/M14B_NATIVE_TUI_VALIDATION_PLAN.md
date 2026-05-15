# M14B Native TUI Validation Plan

## Status

In progress. This milestone replaces the earlier split-native TUI plan with a
validation-first plan for Arroba-managed provider-native TUIs:

```text
arroba codex [session-ref]
arroba opencode [session-ref]
arroba claude [session-ref]
```

Every native TUI launch is owned by Arroba. If no session ref is provided,
Arroba creates a session and the first native-TUI agent. If a session ref is
provided, Arroba attaches a new native-TUI agent to that Arroba session. Native
TUI launches never attach to an existing provider run.

This plan is intentionally about provider-native TUI behavior, not managed I/O.
Prompt attachments are files/images transferred with a prompt. Managed I/O is
the separate Arroba MCP `read_artifact`/`write_artifact` workspace-coordination
system and remains covered by the managed-I/O milestones and drills.

2026-05-14 update: Codex/OpenCode prompt attachment byte transfer is
implemented for relay-attached native TUI clients and Arroba TUI submissions.
The same-host relay drill and direct Docker slice-target drill now validate
native-TUI-origin and Arroba-TUI-origin attachments with two provider-native
TUIs plus one Arroba observer in the same session. These are not yet the full
standard home-worker or home-managed slice topologies.

2026-05-14 permissions update: local Codex/OpenCode mixed native TUI
permission drills pass in both directions. Claude local permissions use an
origin-aware contract: permissions for prompts entered in Claude Code's native
TUI stay in Claude Code's native permission UI, while permissions for prompts
entered from an Arroba TUI are bridged into Arroba and can be answered there.
This keeps the initiating CLI in control of the user-facing permission prompt.

2026-05-14 Claude attachment update: local Claude native TUI prompt attachments
are implemented and covered by `live-native-tui-attachment-drill.mjs --provider
claude`. Native-origin `@file`/`@image` prompts are captured as kernel prompt
attachments. Arroba-origin text attachments are delivered through Claude hook
`additionalContext`; Arroba-origin non-text attachments are materialized and
submitted to Claude's TUI as native `@path` mentions.

2026-05-14 standard remote finding, now superseded: the original native TUI
launch contract could not validate standard home-worker native TUI because it
only launched provider runs on the home kernel. Standard home-worker native TUI
requires remote-backed native provider-run launch so provider execution happens
on the worker.

2026-05-15 implementation update: the first remote-backed native provider-run
path is in progress. Native launchers accept `--machine`/`--kernel-ref` and
move the native TUI agent onto a worker lease before launching the provider run.
For remote placement, Codex/OpenCode require `--server-in-kernel` and Claude
requires `--remote-rendered`, so provider execution is worker-owned rather than
handed a local provider endpoint. The home kernel forwards native provider-run
launches for remote-backed agents to the worker kernel over relay peer transport.
The live drill now has a `--standard-home-worker` mode for an isolated
same-host home/worker relay topology. True cross-host Hetzner validation still
needs endpoint reachability checks for Codex/OpenCode worker-owned provider
servers and Claude credential availability on the worker.

2026-05-15 standard home-worker validation update: Codex and OpenCode pass the
same-host standard home-worker drills for prompt/turns, provider permissions,
and prompt attachments. The drill uses two native TUIs plus one Arroba observer
CLI in one session, separated provider runs, no cross-agent marker
contamination, and badge transitions back to idle. Codex uses a native TUI
projection path that translates home-kernel session output into Codex app-server
notifications for the visible provider TUI while preserving the home-kernel
prompt queue and worker-owned provider execution. Claude is still pending
remote-rendered PTY validation and worker credential checks.

## Goal

Validate and complete native TUI parity across the three providers and three
execution scenarios.

Providers:

- Codex
- OpenCode
- Claude Code

Functional areas:

- prompt and turn observation with two provider TUIs plus one Arroba TUI in one
  Arroba session
- provider permissions
- prompt attachments, meaning files/images sent with a prompt
- MCPs and skills

Scenarios:

- local: provider TUI and provider execution are on the same host/kernel
- standard remote: home kernel owns the session, worker kernel owns provider
  execution
- slice: home kernel owns the session and manages a slice/worker execution
  environment

Interim drill target:

- direct slice target: the native TUI and observer connect directly to the
  slice kernel through relay. This validates cross-filesystem provider
  execution and attachment materialization, but it is not the final
  home-managed slice topology.

## Current Coverage

Prompt/turns:

- Local Codex/OpenCode: covered by native TUI drills.
- Local Claude: covered by the Claude native TUI drill.
- Same-host relay Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs`.
- Standard remote home-worker: Codex/OpenCode prompt/turn coverage passes in
  same-host relay mode. Claude is pending remote-rendered PTY validation and
  worker credential checks.
- Direct slice target Codex/OpenCode: covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker`, but home-managed slice is not covered.

Permissions:

- Local Codex/OpenCode: covered by `live-native-tui-permission-drill.mjs` in
  both native-TUI-origin and Arroba-TUI-origin directions.
- Local Claude: product behavior is implemented with origin-aware permission
  ownership. Claude-origin prompts defer to Claude Code's native permission UI;
  Arroba-origin prompts bridge the permission interaction into Arroba. Dedicated
  automated coverage is provided by `live-native-tui-permission-drill.mjs
  --provider claude`.
- Standard remote home-worker Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers
  codex,opencode --include-permissions`. Native-origin and Arroba-origin
  prompts both surface permission interactions to the Arroba observer and can be
  approved there.
- Standard remote home-worker Claude and slice: not covered.

Prompt attachments:

- Local Codex/OpenCode: covered by `live-native-tui-attachment-drill.mjs`.
- Local Claude: covered by `live-native-tui-attachment-drill.mjs --provider
  claude` for native-origin and Arroba-origin image attachments. Text/file
  attachment delivery is also implemented through the same native capture and
  hook context paths.
- Same-host relay Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --providers opencode,codex
  --include-attachments`.
- Standard remote home-worker Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers
  codex,opencode --include-attachments`.
- Direct slice target Codex/OpenCode: covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker --providers opencode,codex --include-attachments`.
- Standard remote home-worker Claude: not covered.
- Home-managed slice: not covered.

MCPs and skills:

- Covered for normal Arroba provider runs by existing local and remote
  MCP/skill drills.
- Not covered for native TUI provider runs across the local, standard remote,
  and slice matrix.

## Attachment Transfer Contract

Local fast path:

- When the provider execution process can read the same filesystem path as the
  TUI/client, Arroba may pass a local path or `file://` URL directly to the
  provider.
- The local path fast path is acceptable for local native TUI drills and avoids
  unnecessary copying.

Transmission path:

- When the provider execution process may not see the TUI/client filesystem,
  Arroba must transmit attachment bytes.
- Native TUI proxies and Arroba TUI prompt submission should convert local
  attachments into `PromptAttachment.contents_base64` when the run is remote,
  slice-backed, or explicitly exercising attachment transmission.
- The kernel materializes inline attachment bytes on the provider-execution
  side before dispatch, then rewrites the provider-facing attachment reference
  to a machine-local file path or provider-supported inline payload.
- MIME and filename metadata must be preserved.

Provider notes:

- Codex supports image prompt parts as `localImage`/`image`; non-image files are
  currently described as prompt text with a path. Remote/slice transmission must
  materialize images on the provider side before the Codex turn starts.
- OpenCode supports file prompt parts. Remote/slice transmission must
  materialize files on the provider side before forwarding the OpenCode prompt.
- Claude structured runs support inline base64 image/text handling, but Claude
  native TUI hook/PTY mode needs separate validation for both Arroba-origin and
  provider-native attachments.

## Implementation Order

1. Clean native TUI drill naming.
   - Remove native-TUI managed-I/O artifact checks from native TUI drills.
   - Use `attachments` only for prompt files/images.
   - Keep managed I/O in its dedicated drills.

2. Implement and validate remote/slice prompt attachments for Codex and
   OpenCode.
   - Add a shared attachment-preparation helper in the CLI/native-TUI layer.
   - Preserve the local path fast path for local same-filesystem runs.
   - Encode local attachment bytes into `contents_base64` for remote/slice
     native TUI runs.
   - Confirm the kernel materializes those bytes on the provider-execution
     machine and provider-facing paths are local to that machine.
   - Add live checks for native-TUI-origin and Arroba-TUI-origin attachments.
     Same-host relay and direct slice target are validated for Codex/OpenCode;
     standard remote and home-managed slice remain to be added.

3. Revisit local permissions for all providers.
   - Codex/OpenCode local native TUI permissions pass in both directions.
   - Claude local permissions are origin-aware: Claude-origin permissions stay
     native; Arroba-origin permissions bridge into Arroba.
   - Claude deterministic live coverage is implemented with a native-TUI
     approval driver for native-origin prompts and Arroba interaction approval
     for Arroba-origin prompts.

4. Implement and validate local Claude prompt attachments.
   - Completed for local text/file and image attachments.
   - Native-origin `@file`/`@image` references are captured and submitted to the
     kernel as prompt attachments.
   - Arroba-origin text/file attachments are delivered through Claude hook
     `additionalContext`.
   - Arroba-origin images are materialized and injected into Claude Code as
     native `@path` mentions so the provider TUI handles them normally.

5. Validate standard remote home-worker native TUI.
   - In progress: native TUI launches can create remote-backed Arroba agents
     and request worker-owned native provider runs.
   - Current provider status:
     - Codex/OpenCode: prompt/turn, permission, and prompt-attachment drills pass
       in same-host home-worker relay mode.
     - Claude: pending remote-rendered PTY drill and credential validation on
       the worker.
   - Required product work:
     - validate native TUI `--machine`/`--kernel-ref` placement arguments for
       Codex, OpenCode, and Claude;
     - validate the kernel-owned remote native provider-run launch path that
       asks the selected worker kernel to launch/bind the provider-native
       runtime for the leased agent;
     - mirror native prompt/turn output, permission interactions, status, and
       prompt attachments back to the home session without making the relay a
       runtime authority;
     - run the standard home-worker live drill in same-host relay mode first,
       then repeat against the Hetzner relay where endpoint reachability and
       provider credentials allow it.
   - Run the prompt/turn, permission, and prompt-attachment matrix for all three
     providers.
   - Home kernel owns the session; worker kernel owns provider execution.

6. Validate slice native TUI.
   - Run the same prompt/turn, permission, and prompt-attachment matrix for all
     three providers.
   - Home kernel manages the slice/worker execution environment.

7. Validate MCPs and skills for native TUI runs.
   - Adapt existing MCP/skill drills to launch native-TUI provider runs.
   - Cover local, standard remote, and slice scenarios for all providers.
   - Keep managed-I/O marker writes out of native-TUI MCP/skill validation
     unless the drill is explicitly a managed-I/O drill.

## Matrix

Legend:

- `pass`: validated in current code
- `gap`: not validated or not implemented
- `recheck`: previously passed, but must be rerun after the native TUI cleanup
- `partial`: implemented or manually confirmed, but missing complete automated
  live-drill coverage

| Scenario | Provider | Prompt/turns | Permissions | Attachments | MCP/skills |
| --- | --- | --- | --- | --- | --- |
| local | Codex | pass | pass | pass | gap |
| local | OpenCode | pass | pass | pass | gap |
| local | Claude | pass | pass | pass | gap |
| standard remote | Codex | pass | pass | pass | gap |
| standard remote | OpenCode | pass | pass | pass | gap |
| standard remote | Claude | gap | gap | gap | gap |
| direct slice target | Codex | pass | gap | pass | gap |
| direct slice target | OpenCode | pass | gap | pass | gap |
| home-managed slice | Codex | gap | gap | gap | gap |
| home-managed slice | OpenCode | gap | gap | gap | gap |
| home-managed slice | Claude | gap | gap | gap | gap |

## Drill Requirements

Prompt/turn drills must launch:

- two provider-native TUIs in one Arroba session
- one observer Arroba TUI or automation-backed Arroba CLI in the same session

They must validate:

- provider-native prompt from agent A appears in Arroba history and observer UI
- provider-native prompt from agent B appears in Arroba history and observer UI
- Arroba-origin prompt to agent A appears in the provider-native UI path and
  completes
- Arroba-origin prompt to agent B appears in the provider-native UI path and
  completes
- responses are visible in Arroba history and observer UI
- no A/B cross-contamination
- agent footer badge changes from idle to working/thinking during the turn and
  returns to idle after completion

Attachment drills must validate:

- native-TUI-origin attachment reaches the provider execution side
- Arroba-TUI-origin attachment reaches the provider execution side
- local runs may pass local paths directly
- remote and slice runs must transmit bytes and materialize provider-local
  paths

Permission drills must validate:

- provider-native permission prompts surface according to the provider contract
- if Arroba-side response is supported, answering from Arroba resumes the same
  provider turn
- if a provider only supports native-TUI response, the native TUI remains
  coherent and Arroba state does not claim a false approval bridge

MCP/skill drills must validate:

- pre-granted MCPs and skills are visible to native-TUI provider runs
- same-turn skill requests work when supported
- remote/slice provider execution sees worker-local MCP definitions and
  materialized skill files
- provider-native MCP calls execute on the provider execution machine

## Cleanup Rules

- Do not add native-TUI managed-I/O artifact checks to this milestone.
- Do not use `artifact` in native TUI drill names unless the test is about
  generic test output files kept after failure.
- Keep behavior below clients where possible: kernel owns sessions, agents,
  provider runs, permissions, attachment materialization, history, and status.
- Any protocol shape changes must bump `LOCAL_DAEMON_PROTOCOL_VERSION`, update
  protocol snapshot/hash tests, and add a focused drill.
