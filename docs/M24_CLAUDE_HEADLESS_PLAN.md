# M24 Claude Headless Provider Plan

## Goal

Add a new `Claude headless` provider mode that runs the interactive Claude Code
CLI without `-p`, through the user's Claude Code login, while presenting the
same standard Chariox agent surface as Codex, OpenCode, and the current Claude
stream-json provider. Rename the current Claude stream-json provider surface to
`Claude -p`.

This plan does not introduce Anthropic SDK, Claude API, hosted agent services,
or provider credential storage in Chariox. Claude execution remains through the
official Claude Code CLI and Claude Code login state.

## Non-Goals

- Do not replace `chariox claude` native TUI. Native TUI remains a visible
  provider-native client surface.
- Do not use Anthropic SDK, Agent SDK, or direct API calls.
- Do not use Claude Remote Control as Chariox's backend control protocol.
- Do not make Cloud, relay, or web app a runtime authority.
- Do not remove `Claude -p`; keep it available for users who prefer the
  structured stream-json path.

## Naming And Compatibility

Provider catalog labels:

- `Claude headless`: new default candidate for subscription-backed interactive
  Claude Code usage.
- `Claude -p`: current `claude -p --input-format stream-json
  --output-format stream-json` provider path.
- `Claude native TUI`: the explicit `chariox claude` provider-native TUI surface,
  shown only where native TUI launch is relevant.

Internal compatibility rules:

- Keep existing serialized provider ids and resume state readable.
- Avoid changing protocol shapes for a display-label-only rename.
- If a new adapter key is needed, add aliases rather than rewriting persisted
  history:
  - `claude-p` resolves to the current stream-json adapter.
  - `claude-headless` resolves to the new interactive hidden PTY adapter.
  - Legacy `claude` should continue to load old sessions. The default mapping
    for new launches must be explicit in the waiting room/provider catalog.
- Workflow definitions should persist explicit provider ids after this change;
  imported legacy workflows using `claude` should be normalized at load time and
  surfaced with a migration notice only if behavior would change.

Protocol rule:

- If provider ids or launch request shapes change in serialized protocol, bump
  the shared local daemon protocol version, update protocol snapshot/hash tests,
  and add a focused drill for cross-client launch compatibility.

## Current State

Current standard Claude provider:

- Planned in `apps/kernel/src/provider/claude.rs`.
- Uses process label `claude:stream-json`.
- Launch args are assembled in `apps/kernel/src/provider/claude/launch_args.rs`.
- Uses `claude -p --input-format stream-json --output-format stream-json
  --verbose --include-partial-messages --replay-user-messages`.
- Provides clean structured stdout/stderr protocol, abort via stream-json
  `control_request`, prompt queueing, MCP config, permission mode mapping, and
  provider output parsing.

Current Claude native TUI:

- Uses interactive `claude` without `-p`.
- Launches through `apps/cli/src/native-tui/claude.ts` for local native TUI and
  the kernel native launch path for remote/slice native TUI.
- Injects Chariox-origin prompts by typing into the Claude PTY/screen.
- Uses Claude hooks for `UserPromptSubmit`, `Stop`, `StopFailure`,
  `SessionEnd`, `PermissionRequest`, `PreToolUse`, and `PostToolUse`.
- Sends hidden context through `UserPromptSubmit` `additionalContext`.
- Bridges provider-native permission prompts into Chariox runtime interactions.
- Current transcript ingestion is enough for native TUI observation, but not yet
  enough for full standard provider parity.

Happy reference:

- `happy-main` local mode runs interactive `claude` with inherited stdio.
- It discovers Claude session ids through a `SessionStart` hook.
- It tails Claude JSONL transcript files to publish assistant/tool/session
  messages.
- When remote control is needed, Happy switches to its programmatic remote path;
  it does not keep a hidden interactive local Claude process as the fully
  controllable remote agent backend.

The useful takeaways are transcript tailing and session-id discovery, not the
mode-switching architecture.

## Target Architecture

`Claude headless` is a kernel-owned provider runtime with these parts:

1. Hidden interactive Claude process
   - Launch `claude` without `-p`.
   - Use a kernel-managed PTY, `script`, or existing PTY manager path.
   - No visible Claude TUI is attached to the user.
   - Keep the process owned by the kernel/provider run, not by the web app or
     TUI client.

2. Prompt input driver
   - Type Chariox prompts into the hidden PTY.
   - Maintain the same prompt origin distinctions as native TUI:
     - Chariox-origin prompt.
     - Native/provider-origin prompt, if ever observed.
   - Do not expose a raw hidden terminal as the normal user interface.

3. Hook bridge
   - Reuse the native Claude hook handler shape.
   - Keep `UserPromptSubmit` `additionalContext` as the only hidden context path.
   - Keep permission hooks for provider-native approval.
   - Add `SessionStart` if needed for deterministic Claude session id discovery.

4. Transcript tailer
   - Tail Claude's session JSONL transcript continuously.
   - Convert transcript entries into Chariox provider output blobs.
   - Deduplicate entries across relaunch, resume, compaction, and old transcript
     writes.
   - Use hook `Stop` as one completion signal, not as the only output source.

5. Provider run integration
   - Mark the run as a standard Chariox agent from the client perspective.
   - Internally, the run uses provider-native PTY and transcript observation
     rather than structured stdio.
   - Keep kernel as the authority for prompt queue, status, interactions,
     history, workflow state, and permissions.

## Implementation Workstream

### 1. Provider Catalog And Labels

Files to inspect/update:

- `apps/kernel/src/provider/catalog.rs`
- `apps/kernel/src/provider/claude.rs`
- `apps/cli/src/provider-command-catalog.ts`
- waiting room provider model/catalog code in OSS and Cloud
- workflow provider selection code in OSS and Cloud

Tasks:

- Add provider catalog entry for `Claude headless`.
- Rename existing Claude display label to `Claude -p`.
- Preserve old ids/aliases for resume, history, and workflows.
- Make provider cards and command center show both Claude modes distinctly.
- Ensure default provider selection is explicit and does not silently migrate
  existing saved workflows.

Validation:

- Unit test catalog contains `Claude headless` and `Claude -p`.
- Existing serialized `claude` session still loads.
- Waiting room and workflow provider pickers display both modes.

### 2. Launch Planning

Files to inspect/update:

- `apps/kernel/src/provider/claude.rs`
- `apps/kernel/src/provider/claude/native_tui.rs`
- `apps/kernel/src/provider/launch_contract.rs`
- `apps/kernel/src/provider/service/run_lifecycle.rs`
- `apps/kernel/src/provider/service/runtime_io.rs`
- `apps/kernel/src/runtime/state/provider_launch_owned_state.rs`

Tasks:

- Split Claude launch planning into three explicit modes:
  - `Claude -p`: `claude -p` stream-json.
  - `Claude headless`: interactive Claude in hidden PTY.
  - `Claude native TUI`: interactive Claude with visible provider-native UI.
- Share native hook file generation between headless and native TUI.
- Add headless-specific process label, for example `claude:headless`.
- Add headless process state that records:
  - events file
  - context file
  - context response dir
  - permission response dir
  - settings file
  - transcript path or Claude session id once discovered
  - transcript cursor/dedupe keys
- Do not route headless through `structured_endpoint: stdio://claude`.
- Keep provider env cleanup for Anthropic API env vars, matching current Claude
  provider behavior.

Validation:

- Launch plan unit tests for all three Claude modes.
- `Claude -p` launch args still include `-p`.
- `Claude headless` launch args do not include `-p`, `--input-format`, or
  `--output-format`.
- Headless launch includes hook settings and MCP config.

### 3. Hidden PTY Driver

Files to inspect/update:

- `apps/kernel/src/pty/manager.rs`
- `apps/kernel/src/provider/run_actor/*`
- `apps/kernel/src/runtime/state/provider_process_runtime_state.rs`
- `apps/cli/src/native-tui/claude-tui-launcher.ts` as reference only

Tasks:

- Add a kernel-side prompt driver for headless Claude.
- It must support:
  - wait for initial prompt readiness
  - submit visible prompt text
  - submit Enter/return
  - stop/terminate process
  - capture rendered PTY output for debugging and permission fallback
- Keep PTY output out of normal provider transcript unless mapped through the
  transcript tailer or explicit fallback.
- Add debug log paths for failed drills.

Validation:

- Live smoke drill starts headless Claude, submits one prompt, observes output,
  and terminates cleanly.
- Drill artifact cleanup removes PTY logs and temp hook directories on success.

### 4. Prompt Assembly And Hidden Context

Files to inspect/update:

- `apps/kernel/src/provider/service/runtime_io.rs`
- `apps/kernel/src/provider/workspace_live_sync_policy.rs`
- `apps/cli/src/native-tui/claude-bridge.ts`
- `apps/cli/src/native-tui/claude-attachments.ts`
- centralized prompt injection service used by provider prompt assembly

Tasks:

- Use the centralized prompt injection service for all hidden headless context.
- Visible text typed into Claude must exclude hidden instructions.
- Hidden instructions, metaagent event notifications, workflow context, and
  attachment context must be delivered via `UserPromptSubmit` additionalContext.
- Preserve existing attachment rendering:
  - visible file mentions where Claude needs them
  - hidden inline context where Chariox owns the attachment payload
- Ensure prompt metadata and history show user-visible prompt text correctly.

Validation:

- Prompt injection drill for `Claude headless`:
  - hidden token appears in Claude response when requested
  - hidden token does not appear in visible PTY log
  - hidden token does not appear in user-visible prompt history
- Attachment drill:
  - text attachment is understood
  - image attachment is understood where Claude supports it
  - remote/slice attachment transfer works or fails loudly with a tracked gap

### 5. Transcript Tailer And Output Mapping

Files to inspect/update:

- `apps/kernel/src/provider/claude_runtime.rs`
- `apps/kernel/src/app/provider_output_claude_native.rs`
- `apps/cli/src/native-tui/claude-transcript.ts`
- Happy reference:
  - `happy-main/packages/happy-cli/src/claude/utils/sessionScanner.ts`
  - `happy-main/docs/session-protocol-claude.md`

Tasks:

- Implement a Rust transcript tailer for Claude JSONL files.
- Track:
  - file path
  - line offset
  - message uuid or stable dedupe key
  - current Chariox prompt id
  - Claude session id
  - provider subagent/task mapping if available
- Parse and map transcript entries:
  - assistant text -> provider output text blob
  - assistant thinking -> thinking blob if present and permitted
  - assistant tool use -> tool call start blob with name/title/args
  - user tool result -> tool call result/end blob
  - system/session events -> debug/service blob only when useful
  - summary -> history summary metadata, not assistant output
  - unknown entries -> ignored with debug logging
- Preserve order across lines, hooks, and PTY status.
- Do not emit duplicate blobs after resume/relaunch.
- Handle Claude writing to old transcript files after resume/compaction.

Validation:

- Unit tests with fixture JSONL covering:
  - assistant text
  - tool call and result
  - Bash output
  - Edit/Write output
  - Task/subagent entries
  - summary lines
  - unknown internal entries
  - duplicate uuid/line replay
  - writes to old and new transcript files
- Live drill validates that UI sees assistant text and tool call cards before
  final completion, not only at `Stop`.

### 6. Turn Completion And Status

Files to inspect/update:

- `apps/kernel/src/runtime/state/prompt_activity_owned_state.rs`
- `apps/kernel/src/runtime/state/provider_output_runtime_tests.rs`
- `apps/kernel/src/provider/run_actor/*`
- `apps/kernel/src/provider/service/runtime_io.rs`

Tasks:

- Start Chariox turn on prompt submission.
- Mark provider run as working/thinking while Claude is processing.
- Complete prompt on Claude `Stop` hook after transcript tailer drains new
  output.
- Mark failed on `StopFailure`, process exit, or hook error.
- Mark cancelled on user abort.
- Ensure workflow runtime sees normal provider turn completion.

Validation:

- Prompt status transitions:
  - queued -> active -> completed
  - queued -> active -> failed
  - queued -> active -> cancelled
- Web and TUI status indicators match existing providers.
- History outline includes Claude headless turns.

### 7. Midturn Prompt Steering

Tasks:

- Characterize interactive Claude behavior before locking the design:
  - type a second prompt while Claude is active
  - type escape/control-C while active, then prompt
  - use slash interrupt if available
  - compare hidden PTY, visible native TUI, and `Claude -p`
- Decide deterministic policy:
  - if interactive Claude queues typed prompts safely, use queueing;
  - if it ignores or corrupts input, implement interrupt-and-reprompt;
  - if neither is reliable, reject midturn steering for headless and document
    the provider limitation.
- For metaagent event delivery, do not rely on polling. The kernel should still
  inject event prompts deterministically. If headless cannot steer midturn
  safely, it must interrupt and reprompt rather than silently queue invisible
  state.

Validation:

- Provider steering drill includes:
  - Codex
  - OpenCode
  - `Claude -p`
  - `Claude headless`
  - Claude native TUI
- Drill output records whether behavior is active-turn fold, queued next turn,
  interrupt-and-reprompt, or unsupported.

### 8. Permissions

Files to inspect/update:

- `apps/kernel/src/app/provider_output_claude_native.rs`
- `apps/kernel/src/provider/claude/native_tui.rs`
- `apps/kernel/src/provider/native_permission_instructions.md`
- `apps/kernel/src/runtime/state/tool_dispatch/*`
- permission UI in TUI and web

Tasks:

- Reuse Claude native permission hooks:
  - `PermissionRequest`
  - `PreToolUse`
  - `PostToolUse`
- Surface one kernel-owned `RuntimeInteraction` to all Chariox clients in the
  session.
- Resolve allowed/denied decisions by writing hook response files.
- Support required, plan, yolo/bypass modes.
- Ensure yolo does not block on Claude's interactive bypass confirmation screen;
  launch args/settings must make bypass deterministic or force a clear user
  setup step.
- Ensure metaagent permissions can resolve regular agents but not self.

Validation:

- Required Bash permission drill from TUI.
- Required Bash permission drill from web product frontend.
- Edit/Write permission drill.
- Deny path drill.
- Yolo path drill.
- Plan/exit-plan drill.
- Remote worker permission drill.
- Slice permission drill where Claude auth is available.

### 9. MCP, Scripts, Skills, And Runtime Tools

Files to inspect/update:

- `apps/kernel/src/provider/claude/launch_args.rs`
- `apps/kernel/src/provider/claude/native_tui.rs`
- runtime MCP registration and filtering code
- script extension grant code
- skill grant/sync code

Tasks:

- Pass MCP config to Claude headless exactly as native TUI does.
- Grant runtime MCP, user MCPs, scripts, and skills using existing grant policy.
- Preserve metaagent-only MCP tool filtering.
- Preserve slice-related tool filtering.
- Keep runtime MCP prompt exposed only to providers/agents that should see it.
- Validate `ToolSearch` behavior and avoid exposing conflicting tool search
  surfaces.

Validation:

- Runtime MCP echo drill.
- Script extension drill with Python and TypeScript scripts.
- Skill grant drill.
- Metaagent-only tool visibility drill.
- Slice tool visibility drill.

### 10. Workspace Live Sync And Files

Files to inspect/update:

- `apps/kernel/src/provider/workspace_live_sync_policy.rs`
- `apps/kernel/src/provider/workspace_write_fence.rs`
- workspace live sync runtime state/tests
- Claude native attachment code

Tasks:

- Preserve managed workspace live sync instructions.
- Ensure direct native writes inside protected roots are either fenced or routed
  through Chariox MCP tools according to the existing policy.
- Make transcript tool mapping show file modifications coherently.
- Validate external-directory writes and protected-root writes.

Validation:

- Workspace live sync drill with `Claude headless`.
- Protected root direct write denial or fence enforcement drill.
- Chariox MCP read/write/edit artifact drill.
- Remote worker and slice variants where available.

### 11. Workflows

Tasks:

- Make `Claude headless` selectable in workflow canvas.
- Keep `Claude -p` selectable in workflow canvas.
- Ensure hidden/native prompt assembly is compatible with workflow hidden
  instructions.
- Ensure workflow output validation tools work with headless MCP config.
- Ensure workflow completion uses normal provider completion events.

Validation:

- Single-node Claude headless workflow.
- Claude headless -> Codex -> OpenCode chain.
- Codex -> Claude headless -> OpenCode chain.
- Workflow validated output drill.
- Workflow cancel/resume drill.
- Workflow publication/deployment smoke if provider choice is persisted into
  published package metadata.

### 12. Remote Worker And Slice Support

Tasks:

- Remote worker:
  - launch headless Claude on the worker kernel;
  - use worker-local Claude Code login;
  - relay only Chariox protocol events, not Claude credentials or transcript
    contents through Cloud.
- Home-managed slice:
  - allow headless Claude if Claude auth is available/imported into the slice;
  - reuse existing Claude credential transfer rules for Linux runners;
  - clean up temporary credentials after drills.
- Standard home-worker:
  - do not copy MCPs/skills/credentials automatically outside existing rules.

Validation:

- Same-host relay headless drill.
- Actual remote worker headless drill.
- Home-managed Docker slice headless drill.
- Stale worker target selection drill.
- Worker missing Claude auth drill fails loudly with actionable message.

### 13. UI Work

OSS TUI:

- Waiting room provider picker shows `Claude headless` and `Claude -p`.
- Agent footer/provider badge shows selected mode.
- Command center provider commands show both modes.
- Native TUI command remains `chariox claude`.

Web/Cloud:

- Waiting room provider picker shows both modes.
- Side panel spawn controls show both modes.
- Freeform spawn controls show both modes.
- Workflow canvas provider picker shows both modes.
- Agent pane footer/provider badge shows selected mode.
- No hidden Claude terminal is shown as the default agent UI.

Validation:

- Product frontend drill starts a Claude headless agent from waiting room.
- Product frontend drill starts a Claude headless agent from side panel/freeform.
- Product frontend workflow drill includes Claude headless node.
- TUI drill starts Claude headless from waiting room/command flow.

## End-to-End Live Drill Matrix

All drills should clean up temp dirs, provider processes, screen/tmux sessions,
relay processes, worker processes, and large logs on success. Each drill should
keep artifacts only behind an explicit `--keep-artifacts-on-failure` or
equivalent flag.

### Local Provider Parity Drill

Use the existing Claude provider drill for provider-specific behavior, and run
it once per scenario for both Claude modes.

```bash
node apps/cli/scripts/live-claude-provider-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario echo \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-claude-provider-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario attachment \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-claude-provider-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario selection \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-claude-provider-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario abort \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure
```

Must validate:

- launch
- prompt response
- status transitions
- history outline
- assistant output
- tool call rendering
- cancellation
- relaunch/resume where supported; if Claude headless cannot resume through a
  stable interactive Claude session id, the drill must record that as an
  explicit provider limitation and keep `Claude -p` resume coverage passing

### Prompt Assembly And Hidden Injection Drill

```bash
pnpm --filter @chariox/cli run prompt-assembly:drill -- \
  --providers codex,opencode,claude-p,claude-headless \
  --provider-model claude-p=sonnet \
  --provider-model claude-headless=sonnet
```

Must validate:

- hidden instructions affect model output
- hidden markers are absent from visible prompt history
- hidden markers are absent from PTY logs
- attachments are visible to Claude headless
- centralized prompt injection service is the only source of hidden context

### Provider Steering Drill

```bash
pnpm --filter @chariox/cli run provider-steering:drill -- \
  --providers codex,opencode,claude-p,claude-headless \
  --provider-model claude-p=sonnet \
  --provider-model claude-headless=sonnet \
  --timeout-ms 420000 \
  --keep-artifacts-on-failure
```

Must validate:

- active-turn steering behavior
- queued prompt behavior
- interrupt-and-reprompt behavior if used
- provider-specific result classification is recorded

### Permission Drill

```bash
node apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs \
  --provider claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-native-tui-permission-drill.mjs \
  --providers claude \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure
```

Must validate:

- required Bash approval from TUI
- required Bash approval from web product frontend
- deny path
- Edit/Write approval
- yolo/bypass path
- plan approval path
- no provider-native approval bypasses the kernel-owned `RuntimeInteraction`

### Workspace Live Sync Drill

```bash
node apps/cli/scripts/live-workspace-live-sync-drill.mjs \
  --provider claude-headless \
  --provider-model claude-headless=sonnet \
  --mode managed \
  --timeout-ms 360000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-workspace-live-sync-drill.mjs \
  --provider claude-headless \
  --provider-model claude-headless=sonnet \
  --mode tracked \
  --tracked-target-count 2 \
  --tracked-bidirectional \
  --target-branch live-sync-claude-headless-tracked-target \
  --timeout-ms 700000 \
  --keep-artifacts-on-failure
```

Must validate:

- read artifact
- write artifact
- edit artifact
- protected direct write policy
- external write policy
- transcript output for file edits

### Runtime MCP And Extensions Drill

```bash
node apps/cli/scripts/live-runtime-register-extension-drill.mjs \
  --providers claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-script-extension-agent-drill.mjs \
  --providers claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-connector-extension-agent-drill.mjs \
  --providers claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure

node apps/cli/scripts/live-agent-vault-credential-drill.mjs \
  --providers claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 300000 \
  --keep-artifacts-on-failure
```

Must validate:

- runtime MCP call
- user MCP grant
- script extension grant
- skills where applicable
- metaagent-only tool filtering when combined with M23

### Workflow Runtime Drill

```bash
node apps/cli/scripts/live-workflow-runtime-drill.mjs \
  --spawn-daemon \
  --scenario validated-increment-chain \
  --providers claude-headless,codex,opencode \
  --provider-model claude-headless=sonnet \
  --provider-model codex=gpt-5.4 \
  --provider-model opencode=opencode/gpt-5.4 \
  --workspace-live-sync-mode managed \
  --poll-limit 240 \
  --poll-interval-ms 2000

node apps/cli/scripts/live-workflow-runtime-drill.mjs \
  --spawn-daemon \
  --scenario mcp-echo-workflow \
  --providers claude-headless \
  --provider-model claude-headless=sonnet \
  --workspace-live-sync-mode managed \
  --poll-limit 180 \
  --poll-interval-ms 2000
```

Must validate:

- workflow prompt assembly
- workflow output validation
- intermediate and final outputs
- cancellation
- retry/resume
- history output

### Web Product Frontend Drill

Run through the actual web frontend, not direct protocol shortcuts. The current
Cloud product drills that need extension are:

```bash
cd ../chariox-cloud

pnpm run build

node scripts/terminal-badge-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario simple

node scripts/terminal-badge-drill.mjs \
  --provider claude-headless \
  --model sonnet \
  --scenario permission \
  --permission-hold-ms 1000

CHARIOX_WORKFLOW_CANVAS_PROVIDER=claude-headless \
CHARIOX_WORKFLOW_CANVAS_MODEL=sonnet \
node scripts/local-workflow-canvas-visual-drill.mjs
```

Add or extend web drill coverage to validate:

- waiting room spawn of Claude headless
- side panel/freeform spawn of Claude headless
- provider badge/footer says `Claude headless`
- prompt submission through web terminal
- permission popup in web
- workflow canvas excludes native TUI but includes headless and `Claude -p`
- workflow run with Claude headless node
- relay target freshness and browser/kernel `RelayStatus` are verified before
  web terminal prompt submission

### Remote Worker Drill

```bash
node apps/cli/scripts/live-remote-machine-runtime-drill.mjs \
  --provider claude-headless \
  --provider-model claude-headless=sonnet \
  --timeout-ms 420000
```

Must validate:

- worker-owned Claude headless launch
- worker-local Claude auth detection
- prompt/output/permissions over relay
- no Cloud runtime proxying
- stale worker not selected

### Slice Drill

```bash
node apps/cli/scripts/live-remote-native-tui-drill.mjs \
  --home-managed-slice-local-docker \
  --providers claude-headless \
  --include-permissions \
  --include-attachments
```

If this drill uses a different script after implementation, preserve the
validated behavior:

- home-managed slice launch
- Claude credential import for Linux runner
- prompt and output
- permissions
- attachments
- cleanup of temporary credential material

### Native TUI Regression Drill

```bash
pnpm --filter @chariox/cli run native-tui:prompt-injection-drill -- \
  --providers codex,opencode,claude
```

Must validate that adding Claude headless does not regress the existing visible
native Claude TUI path.

## Acceptance Criteria

- `Claude -p` remains functional and explicitly labeled.
- `Claude headless` launches interactive Claude without `-p`.
- `Claude headless` uses Claude Code login, not SDK/API credentials.
- Prompt input works from TUI, web terminal, waiting room, workflow, remote
  worker, and supported slice topologies.
- Hidden prompt injection is centralized and delivered through Claude hook
  additionalContext.
- Assistant text and tool calls stream into Chariox history/output with parity
  close to `Claude -p`.
- Permissions resolve through Chariox interactions in TUI and web.
- Runtime MCP, scripts, skills, attachments, workspace live sync, workflows, and
  metaagent filtering all work with `Claude headless`.
- All live drills above pass or produce documented provider limitations with
  failing tests marked as intentionally skipped only after explicit acceptance.

## Rollout

1. Land catalog labels and launch-mode scaffolding behind an internal feature
   flag.
2. Land hidden PTY launch and single-prompt smoke drill.
3. Land transcript tailer and provider output mapping.
4. Land permissions, attachments, MCP, and workspace live sync parity.
5. Land workflow, remote worker, and slice support.
6. Enable `Claude headless` in local TUI waiting room.
7. Enable `Claude headless` in web/Cloud UI.
8. Keep `Claude -p` available and document cost/behavior differences.
9. Consider making `Claude headless` the default Claude choice only after the
   full parity matrix passes repeatedly.
