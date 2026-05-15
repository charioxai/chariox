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
same-host home/worker relay topology and a `--hetzner-worker` mode for a real
cross-host worker.

2026-05-15 standard home-worker validation update: Codex and OpenCode pass the
same-host standard home-worker drills for prompt/turns, provider permissions,
and prompt attachments. The drill uses two native TUIs plus one Arroba observer
CLI in one session, separated provider runs, no cross-agent marker
contamination, and badge transitions back to idle. Codex uses a native TUI
projection path that translates home-kernel session output into Codex app-server
notifications for the visible provider TUI while preserving the home-kernel
prompt queue and worker-owned provider execution.

2026-05-15 Claude standard home-worker update: Claude Code now has a
remote-rendered PTY path for worker-owned execution. Prompt/turn observation
passes with two Claude native TUIs plus one Arroba observer in the same Arroba
session, with no cross-agent marker contamination and badge transitions back to
idle. Image prompt attachments pass in both directions in same-host home-worker
mode: local Claude `@path` image prompts are intercepted by the remote-rendered
wrapper, transmitted as inline prompt attachments, materialized on the worker,
and injected into the worker-owned Claude TUI; Arroba-origin image attachments
follow the same worker materialization path. Permissions pass in both
native-origin and Arroba-origin directions: permission prompts surface in the
remote-rendered Claude native TUI and approval is sent through kernel-owned PTY
input to the worker provider run.

2026-05-15 actual Hetzner validation update: Codex and OpenCode pass against a
real Hetzner worker for prompt/turns, provider permissions, and prompt
attachments in both native-origin and Arroba-origin directions. The drill uses
SSH local forwarding for the relay and an SSH provider endpoint bridge for
worker-local Codex/OpenCode provider endpoints. Claude Code initially failed
extended Hetzner validation because macOS stores Claude Code credentials in the
Keychain while Linux expects `~/.claude/.credentials.json`; copying only
`.claude.json` transfers account metadata but not the login credential. Exporting
the local `Claude Code-credentials` Keychain payload into the worker
`~/.claude/.credentials.json` makes `claude auth status` green on the worker.
After that transfer, Claude Code passes the actual Hetzner prompt/turn,
permission, and image prompt-attachment drill through the remote-rendered PTY
path.

2026-05-15 home-managed slice validation update: Codex, OpenCode, and Claude
Code pass the local Docker home-managed slice drill for prompt/turns, provider
permissions, and prompt attachments. The native provider TUIs and Arroba
observer attach to the home kernel session; the home kernel places provider
execution on the slice worker through `slice_ref` and reuses the existing
leased-runtime projection path. Codex/OpenCode run in server-in-kernel mode with
worker-owned provider endpoints. Claude uses the remote-rendered PTY path.
Local Docker slice auth import now also copies Claude Code credentials from
`~/.claude/.credentials.json` or the macOS `Claude Code-credentials` Keychain
payload and marks `/workspace` trusted in the slice.

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
  same-host relay mode and against the Hetzner worker. Claude prompt/turn
  coverage passes through the remote-rendered PTY path in same-host relay mode
  and against the Hetzner worker.
- Direct slice target Codex/OpenCode: covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker`; retained only as a lower-level compatibility drill.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker`.

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
  approved there. The same coverage also passes with `--hetzner-worker`.
- Standard remote home-worker Claude: covered by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers claude
  --include-permissions`. Native-origin and Arroba-origin prompts both surface
  permission interactions in the remote-rendered Claude native TUI and can be
  approved through kernel-owned PTY input in same-host relay mode and with the
  actual Hetzner worker once the Linux worker has Claude credentials in
  `~/.claude/.credentials.json`.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker
  --include-permissions`.

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
  codex,opencode --include-attachments`. The same coverage also passes with
  `--hetzner-worker`.
- Direct slice target Codex/OpenCode: covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker --providers opencode,codex --include-attachments`.
- Standard remote home-worker Claude: covered for image prompt attachments in
  both native-origin and Arroba-origin directions by
  `live-remote-native-tui-drill.mjs --standard-home-worker --providers claude
  --include-attachments` in same-host relay mode and with the actual Hetzner
  worker once credentials are transferred.
- Home-managed slice Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs --home-managed-slice-local-docker
  --include-attachments`. Remote/slice placement forces byte transfer and
  provider-side materialization instead of passing host-local paths.

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
       in same-host home-worker relay mode and against the Hetzner worker.
     - Claude: prompt/turn, image prompt-attachment, and permission drills pass
       in same-host home-worker relay mode and against the Hetzner worker
       through the remote-rendered PTY path, after transferring the local
       Keychain credential payload to the worker's Linux credentials file.
   - Required product work:
     - validate native TUI `--machine`/`--kernel-ref` placement arguments for
       Codex, OpenCode, and Claude;
     - validate the kernel-owned remote native provider-run launch path that
       asks the selected worker kernel to launch/bind the provider-native
       runtime for the leased agent;
     - mirror native prompt/turn output, permission interactions, status, and
       prompt attachments back to the home session without making the relay a
       runtime authority;
      - keep the Hetzner worker drill in the standard regression set for all
        providers, with an explicit Claude credential-transfer preflight.
   - Run the prompt/turn, permission, and prompt-attachment matrix for all three
     providers.
   - Home kernel owns the session; worker kernel owns provider execution.

6. Validate slice native TUI.
   - Completed for local Docker home-managed slices.
   - Codex/OpenCode/Claude pass the same prompt/turn, permission, and
     prompt-attachment matrix as standard home-worker mode.
   - Home kernel manages the slice/worker execution environment; native TUIs do
     not attach directly to the slice kernel.
   - Local Docker slice startup reuses the home relay when available and only
     falls back to a slice-private relay for standalone slice workflows.

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
| standard remote | Claude | pass | pass | pass | gap |
| direct slice target | Codex | pass | gap | pass | gap |
| direct slice target | OpenCode | pass | gap | pass | gap |
| home-managed slice | Codex | pass | pass | pass | gap |
| home-managed slice | OpenCode | pass | pass | pass | gap |
| home-managed slice | Claude | pass | pass | pass | gap |

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
