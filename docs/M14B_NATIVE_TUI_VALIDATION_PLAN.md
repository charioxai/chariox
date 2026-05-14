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
The same-host relay drill and local Docker slice drill now validate
native-TUI-origin and Arroba-TUI-origin attachments with two provider-native
TUIs plus one Arroba observer in the same session. Standard home-worker remote
coverage is still pending.

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

## Current Coverage

Prompt/turns:

- Local Codex/OpenCode: covered by native TUI drills.
- Local Claude: covered by the Claude native TUI drill.
- Same-host relay Codex/OpenCode/Claude: covered by
  `live-remote-native-tui-drill.mjs`.
- Standard remote home-worker: not covered for native TUI.
- Slice: partially covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker`, but this needs to be promoted into the official matrix
  and separated from same-host relay terminology.

Permissions:

- Local Codex/OpenCode: covered historically, but must be rerun and treated as
  untrusted until the current native TUI permission contract is reconfirmed.
- Local Claude: not covered by a dedicated permission drill.
- Standard remote and slice: not covered for all three providers in this native
  TUI matrix.

Prompt attachments:

- Local Codex/OpenCode: covered by `live-native-tui-attachment-drill.mjs`.
- Local Claude: not covered.
- Same-host relay Codex/OpenCode: covered by
  `live-remote-native-tui-drill.mjs --providers opencode,codex
  --include-attachments`.
- Slice Codex/OpenCode: covered by `live-remote-native-tui-drill.mjs
  --slice-local-docker --providers opencode,codex --include-attachments`.
- Standard remote home-worker: not covered.

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
   - Add live checks for native-TUI-origin and Arroba-TUI-origin attachments in
     both standard remote and slice scenarios.

3. Revisit local permissions for all providers.
   - Rerun Codex/OpenCode local native TUI permissions.
   - Fix Codex if approval requests do not surface or resolve correctly through
     Arroba.
   - Add Claude local native TUI permission coverage. If Claude permissions can
     only be answered in the native TUI, document that as the contract.

4. Implement and validate local Claude prompt attachments.
   - Validate Arroba-origin attachments into Claude native TUI mode.
   - Investigate whether Claude hook/transcript payloads expose native-origin
     file/image attachments. If not, fail with a clear unsupported state.

5. Validate standard remote home-worker native TUI.
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

| Scenario | Provider | Prompt/turns | Permissions | Attachments | MCP/skills |
| --- | --- | --- | --- | --- | --- |
| local | Codex | pass | recheck | pass | gap |
| local | OpenCode | pass | recheck | pass | gap |
| local | Claude | pass | gap | gap | gap |
| standard remote | Codex | gap | gap | gap | gap |
| standard remote | OpenCode | gap | gap | gap | gap |
| standard remote | Claude | gap | gap | gap | gap |
| slice | Codex | pass | gap | pass | gap |
| slice | OpenCode | pass | gap | pass | gap |
| slice | Claude | gap | gap | gap | gap |

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
