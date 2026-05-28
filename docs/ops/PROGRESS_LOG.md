# Arroba Progress Log

Chronological notes to preserve execution context between contributors/agents.

## 2026-05-28

### Workspace live sync session-mode contract

- Split Workspace Live Sync configuration into global launch policy and session-scoped runtime mode. `/config workspace-live-sync` and shell `config workspace-live-sync` now write only `providers.workspace_live_sync = required|unrestricted`; `workspace sync mode managed|tracked|unrestricted` sends `SetWorkspaceLiveSyncMode { session_id, mode }` and receives `WorkspaceLiveSyncModeUpdated { session }`.
- Bumped the local daemon protocol to 62 and refreshed kernel, TypeScript CLI/shell, iOS, and Cloud web request/response shapes. Provider launch paths now resolve Workspace Live Sync mode from the session override first, then the global launch policy.
- Local validation passed after the split: `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture` (124 tests), `pnpm --filter @arroba/kernel-client test` (76 tests), `pnpm --filter @arroba/cli test` (1065 tests), `swift test --package-path apps/ios/ArrobaPackage` (65 tests), `/Users/miguel/arroba-cloud pnpm test` (Cloud API/worker/web suites), and syntax checks for changed live-drill scripts.
- Local Codex live drills passed after the split: `workspace-live-sync:managed-drill` with two managed targets, `workspace-live-sync:tracked-drill` with two cross-branch tracked targets plus bidirectional fanout/resolver convergence, and `workspace-live-sync:permission-drill` with approval-gated write resumption. Scalingo/staging hosted drills remain intentionally deferred.

### Workspace live sync local/client validation closure

- Completed the remaining non-Scalingo validation sweep for Workspace Live Sync after tracked-mode parity work. `pnpm --filter @arroba/kernel-client test` passed with 76 tests, `pnpm --filter @arroba/cli test` passed with 1065 tests, `swift test --package-path apps/ios/ArrobaPackage` passed with 65 tests, and `/Users/miguel/arroba-cloud pnpm test` passed through the Cloud API, worker, package, and web app suites, including the Workspace Live Sync side-panel/enrollment coverage.
- Rechecked the old managed-I/O naming surface across OSS and Cloud; the remaining references are mode-specific "managed" wording or historical progress-log notes rather than the feature name. Scalingo/staging hosted drills are intentionally deferred until the hosted platform issue is cleared.

### Workspace live sync Hetzner validation closure

- Extended `apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs` with `--hetzner-worker`, matching the remote workspace live sync drill's actual Hetzner topology: relay and worker kernel run on the configured Hetzner host while the home kernel remains local, with Codex auth synchronized and fixture workspaces mirrored before leased provider launch.
- Updated `apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs` so wrappers can provide an isolated root directory and a post-fixture copy command, and so the drill pumps terminal output while waiting for the workspace live sync permission interaction and final file write.
- Confirmed Codex `gpt-5.2` remote workspace live sync permission validation in both same-host local relay mode and actual Hetzner worker mode. The remote agent's `write_artifact` request surfaced as a home-kernel permission interaction, approval resumed the same turn, and the expected file landed in the coordinated workspace.
- Confirmed full Codex workspace live sync validation against the actual Hetzner worker in both tracked and managed modes. Tracked mode covered two targets, explicit cross-branch binding, bidirectional propagation, `.arrobaignore`, outside-turn ignore, no commits, conflict detection, and resolver convergence. Managed mode covered two targets, structured text/opaque writes, move/delete fanout, direct-write blocking, collision behavior, non-overlap rebase, and overlap rejection.
- Current OpenCode live workspace sync drills are blocked before workspace live sync behavior by provider auth (`Token refresh failed: 401`). Treat current OpenCode live validation as an environment gap until auth is refreshed.

### Workspace live sync drill entrypoint cleanup

- Changed the `@arroba/cli` workspace live sync drill aliases to the currently green Codex `gpt-5.2` path, with OpenCode treated as an explicit add-on while provider auth is failing before runtime behavior.
- Made the local and remote workspace live sync drill argument parsers tolerate the conventional `pnpm run <script> -- <args>` separator, and verified both aliases print help through that path.
- Re-ran `pnpm --filter @arroba/cli test` and the focused remote workspace live sync membership authorization test. CLI tests passed with 1065 tests; the kernel test verified non-member denial plus member workspace-link create/attach/status identity recording.

### Workspace live sync protocol contract refresh

- Updated `docs/PROTOCOL.md` section 5.0.1 from managed-only wording to the current Workspace Live Sync contract: managed and tracked modes, turn-end tracked fanout, `.arrobaignore` plus force-excludes, explicit workspace-link requirements, relay apply/status shapes, no auto commits, conflict surfacing, and resolver-entry convergence.
- Re-ran `cargo test --manifest-path apps/kernel/Cargo.toml workspace_live_sync -- --nocapture`; 124 focused kernel tests passed, including protocol shape/version hashes, relay apply shape, ignore initialization/force-excludes, journal sequencing, tracked snapshots, rebase/conflict handling, membership auth, and relay peer application.

### Workspace live sync validation aliases

- Added explicit `@arroba/cli` aliases for Codex managed, tracked, permission, remote managed, remote tracked, and remote permission Workspace Live Sync drills so the validated local/relay matrix can be rerun without reconstructing long command lines.
- Made the local and remote permission drill parsers accept the normal `pnpm run <script> -- <args>` separator; the local permission drill also accepts `--provider-model PROVIDER=MODEL`, matching the other Workspace Live Sync wrappers.
- Verified all Workspace Live Sync aliases reach `--help`, and re-ran `pnpm --filter @arroba/cli test`; 1065 tests passed.

## 2026-05-15

### M14B actual Hetzner native TUI worker validation

- Added the actual Hetzner worker path to `live-remote-native-tui-drill.mjs`.
  The drill starts the relay and worker kernel on the Hetzner host, reaches the
  relay through an SSH local forward, and bridges Codex/OpenCode worker-local
  provider endpoints back to local native TUIs through an SSH provider endpoint
  bridge.
- Confirmed OpenCode against the Hetzner worker for prompt/turns, provider
  permissions, prompt attachments, two native TUIs plus one Arroba observer in
  one session, no cross-agent marker contamination, and badge transitions back
  to idle.
- Confirmed Codex against the Hetzner worker for the same prompt/turn,
  permission, attachment, separation, and badge-status matrix.
- Confirmed Claude Code prompt/turns against the Hetzner worker through the
  remote-rendered PTY path. The first extended run exposed a credential-transfer
  gap: macOS stores Claude Code credentials in the Keychain, while the Linux
  worker expects `~/.claude/.credentials.json`, so copying `.claude.json` alone
  left the worker in `Not logged in`.
- Confirmed the credential-transfer path for Claude Code by exporting the local
  `Claude Code-credentials` Keychain payload to the worker's
  `~/.claude/.credentials.json`; `claude auth status` then reported
  `loggedIn: true` on the worker.
- Confirmed Claude Code extended Hetzner validation after credential transfer:
  image prompt attachments and native-origin/Arroba-origin permission prompts
  pass through the remote-rendered PTY path.
- Fixed OpenCode native TUI projection for cross-host multi-TUI runs by adding a
  periodic transcript refresh while the provider event stream is active.
- Updated remote permission assertions to check execution files on the worker
  when the provider run is Hetzner-backed.

## 2026-05-14

### M14B native TUI validation reset

- Replaced the stale M14 split-native plan with `docs/M14B_NATIVE_TUI_VALIDATION_PLAN.md`, focused on Arroba-managed native TUI validation across local, standard remote home-worker, and slice scenarios.
- Removed the misleading native-TUI workspace live sync artifact checks from `live-remote-native-tui-drill.mjs`. Native TUI attachment validation now explicitly means prompt files/images, while workspace live sync remains covered by dedicated workspace live sync drills.

### M14 remote native TUI same-host relay

- Confirmed `apps/cli/scripts/live-remote-native-tui-drill.mjs --providers opencode,codex,claude --keep-artifacts-on-failure` against the same-host relay topology.
- The drill launches two native TUI agents plus one Arroba observer CLI in one Arroba session per provider, sends prompts from both Arroba and native TUIs, verifies provider responses appear without cross-agent marker contamination, and checks agent badges move from idle to working/thinking and back to idle.
- Fixed Claude Code remote-rendered auth in the drill by keeping Arroba state isolated through explicit runtime env vars while launching the kernel/provider process with the real user `HOME`, which lets Claude Code see its normal authenticated configuration.
- Fixed Claude Code Arroba-origin prompt submission reliability by staging visible prompt typing and Enter submission instead of sending the prompt plus carriage return in one PTY write.

### M14B standard home-worker native TUI

- Added the Claude Code standard home-worker remote-rendered PTY path. The home
  kernel owns the Arroba session, the worker kernel owns the Claude provider
  process and PTY, and the local native TUI launcher renders/controls that PTY
  through the existing kernel and relay paths.
- Added relay peer support for leased native provider PTY input, plus worker-to-
  home projection of native prompts from active prompt state and worker history.
- Confirmed the Claude standard home-worker prompt/turn drill with two Claude
  native TUIs plus one Arroba observer CLI in one Arroba session, separated
  agents, no marker contamination, and badge transitions returning to idle.
- Confirmed Claude standard home-worker image prompt attachments in both
  directions. Claude-origin local `@path` image prompts are intercepted by the
  remote-rendered wrapper, transmitted as inline prompt attachments, materialized
  on the worker, and injected into Claude Code as worker-local native `@path`
  mentions; Arroba-origin image attachments use the same worker materialization
  path.
- Confirmed Claude standard home-worker permissions in both directions. Native-
  origin and Arroba-origin prompts both surface permission approval in the
  remote-rendered Claude TUI, and the approval is sent back through kernel-owned
  PTY input to the worker provider run.

## 2026-04-25

### OSS iOS app planning baseline

- Added `docs/ios/IOS_APP_PLAN.md` for the native OSS iOS client sub-project.
- Captured the key boundary: iOS is a client surface like the TypeScript CLI and Cloud browser terminal, while the kernel remains the runtime authority for sessions, agents, workflows, provider runs, permissions, workspace live sync, and relay membership.
- Recommended native SwiftUI under `apps/ios`, direct kernel WebSocket transport through `URLSessionWebSocketTask`, Keychain for relay/cloud credentials, XCTest/XCUITest for committed tests, XcodeBuildMCP for the default build/test/run validation loop, and iOS Simulator MCP for explicit QA/dogfooding passes when requested or confirmed.
- Documented that Maestro is a candidate tool future agents should suggest when useful, but they must ask Miguel before adding it to the repo, installing it as a project dependency, or making it part of the official QA gate.
- Added `IOS-001` to the repo-native task board.
- Installed SwiftUI/iOS implementation skills into `~/.codex/skills`: `swiftui-expert-skill`, `swiftui-ui-patterns`, `swiftui-view-refactor`, `swift-concurrency-expert`, `swiftui-performance-audit`, and `ios-debugger-agent`.
- Added Codex MCP server entries for `XcodeBuildMCP` and `ios-simulator`.

## 2026-04-24

### M12 provider-native permissions and mode defaults

- Added provider-native `mode` (`build|plan`) and `permissions` (`required|yolo`) as first-class runtime launch state in the kernel, with effective resolution from session defaults plus agent overrides.
- Added session config keys `agents.mode` and `agents.permissions`, plus agent-local override mutation through the new `UpdateAgentConfig` local daemon request/response.
- Mapped the effective values into provider launch behavior: Codex now derives approval/sandbox policy from `mode + permissions`, while OpenCode now sets `default_agent` and native permission rules for `edit`, `bash`, and `task`.
- Added shell commands `session mode`, `session permissions`, `agent mode`, and `agent permissions`, including `inherit` for clearing agent overrides and `context` output that shows the effective current values.
- Added matching CLI slash commands `/session mode`, `/session permissions`, `/agent mode`, and `/agent permissions`.
- Extended split-pane agent footers to show effective mode and permissions alongside identity/provider/model metadata.
- Verified the slice with `cargo test --manifest-path apps/kernel/Cargo.toml --no-run`, `pnpm --filter @arroba/kernel-client run test`, and `pnpm --filter @arroba/cli run lint`.

### M12 popup blocking spike validation

- Extended the existing controlled-exec spike with a blocking `request_popup` path in the interaction gateway, including timeout/default-on-timeout handling and externally-resolved late answers.
- Added fake drills proving that the popup request really blocks until timeout or later resolution, then resumes with a structured reply in the same turn.
- Re-ran live Codex and OpenCode drills with a forced delayed popup response. Both providers waited on the popup tool call and then completed the same turn after the delayed answer arrived.
- Verified with `cd experiments/controlled-exec-spike && npm run check`, `npm test`, `npm run drill:fake`, and `npm run drill:providers`.
- Latest live artifacts: `experiments/controlled-exec-spike/artifacts/2026-04-24T11-19-09-661Z-provider-drill/`.

### M12 popup + native permission closure

- Added the production popup interaction layer to Arroba proper: `request_popup` now blocks the current turn until the user answers or a timeout/default resolves.
- Added always-injected shared runtime instructions so Arroba runtime MCP tools are advertised independently of workspace live sync mode.
- Surfaced provider-native permissions through the same interaction model:
  - Codex `item/commandExecution/requestApproval`
  - OpenCode native permission events
- Routed CLI interaction-strip answers back to provider-native approval channels and resumed the same turn after approval.
- Fixed transient interaction lifecycle bugs by moving pending interaction / pending MCP continuation stores onto shared process-wide state.
- Fixed Codex native approval handling by restoring the correct unrestricted `required -> untrusted` mapping and the correct app-server decision vocabulary.
- Added live drill coverage:
  - `apps/cli/scripts/live-native-permission-drill.mjs`
  - `apps/cli/scripts/live-popup-drill.mjs`
- Confirmed live native permission drills for both Codex and OpenCode.
- Confirmed live Codex popup execution on the real-home auth path and retained artifacts proving `request_popup` completed and resumed with `USER_FEEDBACK_RESULT:green`.
- Closed the initial local M12 scope. Shell popup queue UX stayed outside M12.

## 2026-04-27

### Workspace live sync permission follow-on

- Added workspace live sync permission gating for mutating Arroba runtime tools. When effective permissions require approval, `write_artifact`, `edit_artifact`, `apply_patch`, `move_artifact`, and `delete_artifact` now block on an Arroba interaction before the mutation applies.
- Split prompt assembly by execution path: all structured runs get the shared runtime instructions, unmanaged runs get the native-permissions block, and workspace live sync runs get the workspace live sync block.
- Added `apps/cli/scripts/live-workspace-live-sync-permission-drill.mjs` and confirmed live workspace live sync permission drills for Codex and OpenCode.

### Remote permission UX follow-on

- Extended the local native-permission and workspace live sync permission drills so they can be driven through an already-running home kernel and a leased remote worker.
- Added relay wrapper drills:
  - `apps/cli/scripts/live-remote-workspace-live-sync-permission-drill.mjs`
  - `apps/cli/scripts/live-remote-native-permission-drill.mjs`
- Confirmed remote workspace live sync permission drills for Codex and OpenCode. Home-kernel interaction strips now surface remote workspace live sync approval requests and resume the same remote turn after approval.
- Fixed leased-worker provider launch propagation so remote backing provider runs now inherit the leased agent's `execution_mode` and `permission_level` when the worker launches or reloads a provider run.
- Fixed the remote native permission completion projection issue by refreshing the home session snapshot after leased prompt settlement, then confirmed remote native provider permission drills for Codex and OpenCode.

### Remote popup UX follow-on

- Added remote support to `apps/cli/scripts/live-popup-drill.mjs` so it can reuse an existing home kernel, spawn a leased remote agent, and drive the home CLI interaction strip against that remote agent.
- Added `apps/cli/scripts/live-remote-popup-drill.mjs` and `pnpm --filter @arroba/cli run remote-popup:drill`.
- Fixed non-permission `request_popup` forwarding for leased worker agents by routing worker-side runtime popup requests through the existing relay native-interaction channel to the home kernel, with timeout/default handling owned by the home interaction.
- Confirmed remote non-permission popup drills for Codex and OpenCode. Both providers passed feedback choice, warning-level choice, and timeout/default popup paths, and each resumed the same remote turn with the selected/default reply.

## 2026-04-19

### M7.5 Arroba Shell core skeleton

- Added the first `arroba-shell` implementation slice: shell command parsing, slash-command normalization, shell context/result types, `as <name>` bindings, variable substitution, TUI-only command classification, and result rendering helpers.
- Verified the slice with the focused kernel-client shell-core test path.

### M7.5 minimal shell executor

- Added a minimal `arroba-shell` executor over normalized shell commands, returning structured shell results for shell-local commands and low-risk kernel-backed session/agent commands.
- Covered `session list`, `session new --dir|--worktree`, `session attach|use`, `agent list`, `agent spawn --dir|--worktree|--machine`, `agent focus`, and `agent cycle`, with variable binding and context update behavior.
- Verified with the focused kernel-client shell-core and shell-executor test path.

### M7.5 standalone arroba-shell entrypoint

- Added `apps/shell/src/shell.ts` as the standalone `arroba-shell` REPL entrypoint, wired to the existing local kernel IPC client and the shared shell parser/executor.
- Added shell package wiring for `arroba-shell` and `pnpm --filter @arroba/shell run start`, with options for kernel endpoint, workspace/worktree, provider, model, and effort.
- Verified with focused shell tests plus a built `node apps/shell/dist/shell.js --help` smoke check.

### M7.5 arroba-shell script runner

- Added `arroba-shell run <file>` for line-oriented Arroba command scripts with comments, blank lines, variable bindings, and stop-on-error behavior.
- Added mocked IPC fixture coverage proving a script can create a session, bind its id, spawn an agent in that current session, and stop before later commands on failure.
- Verified with the focused kernel-client shell tests, shell app tests, and a built `node apps/shell/dist/shell.js run <tmpfile>` smoke check.

### M7.5 shell app split

- Refactored the shell out of the TUI package into `apps/shell`, keeping it as a sibling app to `apps/cli`.
- Extracted shared kernel-facing client code into `packages/kernel-client`, including IPC transport, request builders, minimal kernel runtime types, shell parser, and shell executor.
- Left the TUI-specific CLI types and UI command handling in `apps/cli`; the CLI imports shared IPC/request code through narrow compatibility re-export files.

### M7.5 workspace shell pane

- Added a right-side `arroba-shell` pane to the workflow workspace screen while preserving the workflow outline/canvas on the left.
- Routed `@ <command>` prompt submissions on the workflow screen through the shared shell parser/executor and rendered input/output transcript entries in the pane.
- Kept TUI-only commands outside the shell path and added focused workspace-shell unit coverage.

### M7.5 shell executor read/status coverage

- Added shared `arroba-shell` executor support for `machine list|kernels`, `relay status`, `config show`, `mcp list|show`, `skill list|show`, and `provider status`.
- Added kernel-client runtime types for relay status, remote machines, and remote kernel presence so shell and TUI surfaces can share the same response model.
- Updated standalone shell usage examples and covered the new command families with focused kernel-client tests.


### M7.5 shell executor MCP/skill mutations

- Added shared `arroba-shell` executor support for `mcp install|update|uninstall|import|grant|revoke|grants` and `skill install|update|uninstall|import|grant|revoke|grants`.
- MCP install/update parsing now covers stdio transports with command/args/env vars and streamable HTTP transports with optional bearer-token env vars, matching the provider-facing registry shape.
- Covered install/update/import/grant/revoke/grants flows with focused kernel-client executor tests.


### M7.5 shell executor workflow coverage

- Added shared `arroba-shell` executor support for core workflow commands: `workflow list|new|show|alias|run|runs|run-show|cancel|resume`.
- Added graph-management coverage for `workflow node add|remove`, `workflow edge add|remove`, and `workflow endpoint new|alias|bind`, including current workflow context updates and variable binding for created workflows/nodes.
- Covered workflow list/create/show/alias, run lifecycle, graph, and endpoint flows with focused kernel-client executor tests.


### M7.5 shell executor config mutations

- Added shared `arroba-shell` executor support for `config path`, `config set`, `config unset`, and `config workspace-live-sync`.
- `config workspace-live-sync` accepts `required|unrestricted` and writes the same user-config key as the TUI command, while reporting that shell changes apply on the next provider launch. Tracked mode is selected from `workspace sync`.
- Covered config path/set/unset/workspace-live-sync flows with focused kernel-client executor tests.


### M7.5 shell executor Slice 6 closure

- Added remaining shared `arroba-shell` executor coverage for workflow advanced config, node runtime flags, watchdogs, workflow queue management, provider login/logout/reauth/process inspection/teardown, and active prompt cancellation.
- `stop` and `cancel` resolve the current session attachment from session state before sending `CancelActivePrompt`, matching the kernel authorization model without adding TUI-only state to the shell context.
- Covered workflow advanced, provider auth/process, and cancellation flows with focused kernel-client executor tests. Slice 6 command-family coverage is now closed.

### M7.5 shell scriptability hardening

- Added script runner ergonomics for repeated `--var NAME=VALUE` seed bindings, `--continue-on-error`, and line-numbered failure diagnostics.
- `arroba-shell run <file>` still stops on first error by default, while validation/drill scripts can continue after structured command failures or thrown transport/kernel errors and return non-zero if any command failed.
- Added standalone shell session attachments so `stop` and attachment-scoped session config commands can run from `arroba-shell`, not only from the TUI.
- Added and passed `live-shell-scriptability-drill.mjs` through `pnpm --filter @arroba/cli run shell:drill` against an isolated local kernel.
- Added `source <file>` / `run <file>` support inside `arroba-shell` and nested scripts, preserving context and variable bindings after loading scripts from disk.

### Session/agent git worktree placement

- Added local session and agent placement commands for existing directories and git worktree creation: `/session new [DIR]`, `/session new --worktree DIR --branch BRANCH [--from REF]`, and `/agent spawn ... --worktree DIR --branch BRANCH [--from REF]`.
- Extended remote agent spawn so `/agent spawn ... --machine MACHINE --worktree REMOTE_DIR --branch BRANCH [--from REF]` forwards a worktree placement spec to the worker kernel, which runs `git worktree add` on the remote machine before creating the leased backing session/agent. Git/repo/configuration failures are surfaced as worker errors.
- Verified local placement with a real temporary git repo/worktree command-action drill and remote placement with a worker-side remote-lease git worktree materialization drill, plus focused CLI and kernel tests.

## 2026-04-18

### M4.6 workspace live sync Codex hardening and live drills

- Root-caused the local workspace live sync drill failure to Arroba's Codex app-server permission approval path: managed Codex turns used the read-only sandbox, but Arroba was approving Codex `item/permissions/requestApproval` requests wholesale, allowing native shell to acquire filesystem write permission.
- Hardened managed Codex runs so permission approvals preserve non-write requests but never grant filesystem write upgrades, while unrestricted Codex runs keep the previous permissive behavior.
- Split the local workspace live sync drill's positive provider phases into smaller serialized prompts so the drill validates each managed read/write/edit/apply-patch/move/opaque step deterministically before entering the direct-write negative checks.
- Verified with `cargo check --manifest-path apps/kernel/Cargo.toml`, focused Codex permission/runtime tests, `node --check apps/cli/scripts/live-workspace-live-sync-drill.mjs`, the full local workspace live sync drill for OpenCode `openai/gpt-5.3-codex` plus Codex `gpt-5.2`, and the local runtime MCP reattach drill for both providers.

## 2026-04-15

### M4.5 production ownership closure status

- Closed the seven ownership points: direct-cutover baseline, session ownership, prompt ownership, provider process/output ownership, workflow/runtime-tool ownership, transport/relay ownership, and runtime fallback deletion now route command/runtime behavior through owned runtime ports.
- Completed the M4.5 dead-code purge by deleting now-unused app-backed session/projection/remote-lease/workflow-console helpers, the obsolete app-backed runtime-tool dispatcher and its tests, and stale test-only calls into compatibility helpers.
- Verified with clean `cargo check`, daemon lib tests, runtime integration tests, kernel websocket integration tests, relay-client tests, and daemon bin tests. Recommendation: treat M4.5 ownership as closed and move next to final docs/invariant alignment before the final I/O-coordination slice.

### M4.5 live drill gate alignment

- Added [LIVE_DRILLS.md](/Users/miguel/arroba/docs/ops/LIVE_DRILLS.md) as the gate before new tasks. It covers local freeform multi-agent mode, local workflow drills, remote freeform relay drills, and remote workflow relay drills.
- Adopted **freeform multi-agent mode** as the name for normal non-workflow multi-agent sessions, replacing the working phrase "unscheduled mode".
- Next recommendation: run the live drill matrix and fix only drill blockers before opening new feature work.

### M4.5 prompt ownership hard-center update

- Moved the single-agent slow-structured local prompt submit path onto owned `CompatibilityRuntimeState` stores, including user history append, prompt-owner submit/mirror mutation, queued notice emission, prompt echo, and dispatch preparation without waiting on the compatibility app mutex.
- Added owned queued prompt advancement for the same single-agent slow-structured path, including expected queue-front activation, provider claim acquisition, structured submit enqueue, prompt activity tracking, and session/agent-runtime projection refresh.
- Kept multi-agent structured prompt scheduling, PTY prompt delivery, provider launch-on-submit, remote prompt submit, workflow-owned prompt progression, and broad provider dispatch semantics on the existing compatibility/provider/workflow paths until points 4 and 5 own those side effects.
- Verified the slice with `cargo test` in `apps/kernel`: 338 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 structured local prompt cancellation ownership update

- Moved structured local prompt cancellation admission and prompt-owner mutation onto owned `CompatibilityRuntimeState` stores for non-remote, non-workflow prompts, including attachment validation, provider-run validation, cancelling-state mirroring, cancellation notices, prompt-activity settlement markers, structured abort dispatch creation, and projection refresh.
- Kept PTY Ctrl-C cancellation, remote prompt cancellation, workflow-owned prompt cancellation, and abort dispatch execution side effects on existing compatibility/provider-runtime paths for follow-up slices.
- Verified the slice with `cargo test` in `apps/kernel`: 336 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 simple local prompt completion ownership update

- Moved the simple local prompt-completion path onto owned `CompatibilityRuntimeState` stores when there is no queued prompt to advance and the active prompt is not remote-backed or workflow-owned, including prompt-owner mutation, session prompt-state mirroring, completion notification recording, prompt-activity cleanup, and projection refresh.
- Kept queued prompt advancement, workflow completion, remote prompt completion, and claim-release retry side effects on the compatibility fallback until those owners are cut over in follow-up slices.
- Verified the slice with `cargo test` in `apps/kernel`: 335 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 local agent destroy ownership update

- Moved local agent destroy onto the owned `CompatibilityRuntimeState` session and agent stores when the target agent is not remote-backed, preserving the compatibility fallback for remote execution lease/relay teardown.
- Added a no-app-lock regression proving local destroy removes the agent from both session and agent-runtime projections while the compatibility app mutex is held.
- Verified the slice with `cargo test` in `apps/kernel`: 334 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 local agent spawn ownership update

- Moved local session-scoped agent spawn onto the owned `CompatibilityRuntimeState` session and agent stores when runtime-owned state is available, keeping remote machine spawn on the compatibility fallback because it still owns relay/lease side effects.
- Fixed session-runtime projection refresh for `AgentSpawned` and `AgentDestroyed` responses so session and agent-runtime projections update from agent lifecycle responses instead of depending on a separate compatibility snapshot path.
- Verified the slice with `cargo test` in `apps/kernel`: 333 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 session alias ownership update

- Moved session alias updates onto the owned `CompatibilityRuntimeState` session store when runtime-owned state is available, with a no-app-lock regression covering alias mutation and projection refresh while the compatibility app mutex is held.
- Verified the slice with `cargo test` in `apps/kernel`: 332 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 session-runtime ownership update

- Moved session create/default-agent bootstrap and session config updates onto the owned `CompatibilityRuntimeState` session, agent, attachment, terminal, history, and projection stores when runtime-owned state is available. These session-runtime paths no longer wait for the compatibility app lock in the normal router-owned configuration, with app-lock fallback kept only for no-owned-state legacy tests.
- Fixed session creation responses to return the refreshed focused-agent session after default-agent bootstrap, so session/focus projections are populated from the authoritative post-bootstrap state.
- Verified the slice with `cargo test` in `apps/kernel`: 331 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 runtime integration bridge update

- Restored the daemon integration suite after direct facade retirement by routing the stale test-facing session, prompt, terminal, provider-output, and structured-runtime-state helpers through the explicit `KernelSessionService`, `KernelAgentService`, provider-output pump, provider terminal-input, and provider-run actor boundaries instead of the deleted generic local request dispatcher.
- Verified the slice with `cargo test` in `apps/kernel`: 329 daemon unit tests, 15 kernel WebSocket integration tests, and 30 runtime integration tests passed.

### M4.5 direct-cutover direction

- Updated the M4.5 plan to stop treating compatibility preservation as ongoing work. The next slices should replace app-backed runtime ports with owned services and delete the old helper path in the same slice once tests cover the owner path.
- Current code has moved more state and side effects behind owned seams: provider launch state, prompt dispatch/abort enqueue paths, structured prompt I/O reads, provider-output fanout/history writes, prompt-activity updates, workflow prompt progression, workflow console helpers, watchdog pumping, and blocked-claim retry now enter explicit runtime boundaries.
- The remaining M4.5 blockers are still the app-lock ports inside `CompatibilityRuntimeState`: session lifecycle/config/alias/agent mutations, prompt submit/cancel/complete settlement and queue advancement, provider PTY spawn/remove/poll/drain and provider-output prompt settlement, workflow service mutation/runtime tools, transport subscription/replay snapshots, and relay peer/lease state.
- Final I/O coordination remains separate. Keep coarse `WorkspaceCoordinator` claims active during the direct cutover, then design file-level claims, port claims, shell/harness sandboxing, coordinator-owned patch application, and transactional rebase/repair after the actor/projection runtime no longer depends on hot `DaemonApp` paths.

## 2026-04-12

### M4.5 kernel runtime refactor progress

- Landed the first substantial kernel responsiveness slices:
  - `KernelCommand` / `KernelEvent` envelopes and command routing
  - bounded `EventLog` replay with explicit replay-gap behavior
  - session snapshot projection metadata
  - bounded interactive routing and inbound WebSocket admission
  - safe command-id retry handling with in-flight fanout and conflict rejection
  - typed CLI replay-gap handling and user-visible refresh notice
  - local IPC compatibility routing through the same command normalization path
- Moved structured provider submit, abort, and output polling through provider-run actors.
- Added runtime slot tombstones/generations so cleanup racing with slow provider I/O cannot restore stale runtime state.
- Removed provider-family global locks from structured output polling while provider I/O is in progress.
- Fixed the websocket integration harness port race by reserving listeners through server startup.
- Added responsiveness/race coverage for slow history, provider catalog, shell capability, provider launch, structured submit/cancel/poll, slow consumers, replay gaps, duplicate command ids, and runtime cleanup races.
- Introduced `KernelSessionService` and moved session attach, detach, end, delete-by-ref, focus/cycle, and terminal resize behavior behind that service while keeping `DaemonApp` as a compatibility facade.
- Introduced `KernelAgentService` and moved prompt submit, kernel submit acknowledgement/dispatch preparation, cancel, runtime cancel, completion, queue advancement, and cancellation finalization behind that service while keeping `DaemonApp` as a compatibility facade.
- Added `AgentRuntime` per-agent mailboxes for prompt submit/cancel admission so agent prompt commands no longer wait behind the generic interactive queue.
- Added `SessionRuntime` per-session mailboxes for attach, detach, focus/cycle, resize, end, and delete admission so session UI/lifecycle commands are isolated from the generic interactive queue and from unrelated sessions.
- Added session mailbox deregistration after successful end/delete so closed sessions do not leave stale mailbox registrations behind.
- Added projected session lane-key lookup for session delete and detach admission, so delete-by-ref and detach-by-attachment can route to the right session mailbox from warmed projection state before falling back to the compatibility app lock.
- Added `DaemonHealthProjection` snapshots for session/agent command mailboxes, provider runtime operation lanes, projected session counts, active/queued prompt counts, and provider-catalog cache state exposed through `GetDaemonHealth`.
- Added a session-owned focused-agent projection shared by `SessionRuntime`, `AgentRuntime`, and the router's agent-lifecycle response path, so untargeted prompt submit/cancel routing can resolve the focused agent without taking the compatibility `DaemonApp` lock once focus is warmed by session commands or local agent spawn/destroy responses.
- Added the first shared session projection store. `GetSessionState`, successful `ResolveSession`, and warmed `ListSessions` now serve projected data without taking the compatibility `DaemonApp` lock; the router refreshes the projection from session-bearing responses and list responses. List responses also hydrate the per-session projection map, so follow-up state and successful resolve reads can return from projection without a separate compatibility-store read.
- Added the first agent runtime projection store. Session projection refreshes now materialize per-agent active and queued prompt counts, direct agent/session-scoped reads are available, and `GetDaemonHealth` uses those counts as the canonical prompt-work health source while mirroring them into the legacy session-projection counters for compatibility.
- Extended the agent runtime projection with each agent's front queued prompt and made local/remote queue advancement inspect that projected candidate before falling back to compatibility session queue reads.
- Updated detach prompt cleanup to republish session and agent-runtime projections after active/queued prompt removal, preventing stale queue-front reads during follow-up advancement.
- Moved provider idle settlement's active-prompt status check to the agent-runtime projection first, with compatibility session inspection retained as fallback when no projected active prompt is warm.
- Moved agent-runtime prompt projection publication into the per-agent mailbox execution path for prompt submit/cancel, so the mailbox runtime now updates its own active/queued prompt read model while compatibility session projections remain mirrored.
- Routed kernel `CompletePrompt` through the per-agent mailbox, resolving the active prompt owner from warmed session projection before falling back to compatibility session state and publishing completion state into the agent-runtime projection from that mailbox worker.
- Moved `CompletePrompt` owner lookup to the agent-runtime active-prompt projection first, so completion routing can stay off the compatibility app lock even when the warmed session projection is stale.
- Moved kernel `CancelActivePrompt` owner lookup to the same agent-runtime active-prompt projection resolver before per-agent mailbox dispatch, with mailbox execution still enforcing attachment/session and active prompt validation.
- Removed the duplicate router-side agent-runtime projection refresh for `PromptSubmitted`; the router still refreshes the session projection from the response while the agent mailbox owns the agent-runtime prompt projection.
- Extended untargeted prompt routing to use warmed session projection focused-agent state before falling back to the compatibility app lock when the dedicated focused-agent projection is cold.
- Added a shared session-history projection. `GetSessionHistory` can now load from disk using a warmed session snapshot without taking the compatibility `DaemonApp` lock, and repeated warmed transcript reads are served from memory while successful history appends keep the warmed projection current.
- Added a shared provider-run projection. Warmed `GetProviderRun` reads now return without taking the compatibility `DaemonApp` lock, and launch/start/finish/fail/park/resume/ended lifecycle updates refresh the warmed projection.
- Added a warmed provider-process projection. Repeated `ListProviderProcesses` reads now return without taking the compatibility `DaemonApp` lock, while provider-run and session lifecycle changes invalidate the projection so teardown-safety metadata is not served stale.
- Added prompt lifecycle publication into the shared session projection. Prompt submit, complete, cancel, dispatch failure, and queue advancement now update warmed prompt-state snapshots so `GetSessionState` can reflect those transitions without taking the compatibility app lock.
- Removed redundant router-side session snapshots for prompt complete/cancel. Those paths now rely on prompt lifecycle projection publication, keeping follow-up warmed `GetSessionState` reads projection-first without an extra compatibility-store refresh.
- Trimmed router-side session snapshots for non-state terminal control commands: `PollRuntimeNotices` and `ResizeTerminal` no longer perform post-response session projection snapshots, while `PumpTerminalOutput` still does because provider output pumping can settle prompt state.
- Added a TTL-bound provider-catalog projection. Warmed `GetProviderCatalog` reads now return without taking the compatibility app lock, and provider logout/configuration changes invalidate the projection.
- Fixed projection correctness gaps: provider-process projection now stores a canonical unfiltered snapshot and refreshes after teardown, warmed OpenCode `GetProviderRun` no longer bypasses selection-sync side effects, relay reconfiguration invalidates provider catalog projection state, and agent lanes are removed on agent/session cleanup.
- Added warmed session-projection reads for agent and workflow inspection: `ListAgents`, `ListWorkflows`, `ResolveWorkflow`, `ListWorkflowRuns`, `GetWorkflowRun`, `ListWorkflowWatchdogs`, and `ListQueuedWorkflowPrompts` can now return without taking the compatibility app lock once the session projection is warm.
- Added transport health projection counters for kernel websocket pressure: active connections/subscriptions, incoming requests, emitted events, replay gaps, inbound overload rejections, outgoing queue overflows, and slow-consumer closes are now exposed through `GetDaemonHealth`.
- Added a workspace coordination health baseline: active worktree claims and same-workspace worktree collisions are now reported from the warmed session projection through `GetDaemonHealth`.
- Added initial `WorkspaceCoordinator` enforcement for explicit file-writing capabilities. `EditFile` and `StoreTransferredFile` now acquire scoped worktree write claims, reject overlapping same-workspace/worktree writes with a retryable workspace-claim conflict, publish active operation claims through daemon health, and release claims on operation completion.
- Removed provider prompt lifecycle worktree claims. Active local provider prompts no longer acquire whole-worktree claims, so independent sessions can dispatch prompts in the same workspace/worktree; explicit write capabilities and workflow node dispatch remain claim-coordinated.
- Promoted workspace claims into the workflow scheduler. Claims now expose `read`/`write` mode metadata, workflow node dispatch acquires an exclusive `workflow_node_dispatch` write claim before provider submission, blocked nodes move to `BlockedOnWorkspaceClaim`, and claim release retries blocked workflow nodes instead of failing temporary contention.
- Clarified the claim strategy after review: current claims should remain a coarse safety/scheduler layer while M4.5 finishes actor/projection ownership. Deeper I/O coordination, including file-level claims, port claims, harness sandboxing, coordinator-owned patch application, and automatic patch rebase loops, is intentionally deferred to the final coordination slice.
- Removed the duplicate `AgentRuntimePromptStateStore` shadow after review. `AgentRuntimeProjectionStore` is now the single warm prompt-state read model for active-owner routing, queue-front preview, daemon health, and projection-first reads while `PromptStateOwner` is the mutation authority and compatibility session state remains the mirror.
- Changed structured provider submit/abort/output-poll/selection-sync enqueue failures to propagate as daemon errors instead of being logged and swallowed. This keeps prompt dispatch cleanup, claim release, notices, and retryable failures on the normal error path when a provider actor does not accept work.
- Added a per-provider-run structured output return buffer so globally drained background output still comes back from the later direct pump for that provider run, without delaying terminal fanout.
- Introduced `PromptRuntimeState` inside the compatibility session mirror. It is now the only writer for per-agent active/queued prompt state and the legacy session-level prompt/scheduler projections, while serialization remains flattened to the existing wire fields.

## 2026-04-13

### M4.5 kernel runtime refactor progress

- Added `PromptStateOwner` as the kernel write owner for per-agent active prompts and queued prompt backlogs. Prompt submit, complete, cancel, cancellation finalization, dispatch-failure cleanup, queue advancement, detach cleanup, provider settlement, and workflow prompt submission now mutate the owner first and then mirror into compatibility `RuntimeSession` prompt fields.
- Demoted `PromptRuntimeState` to the flattened compatibility mirror/projection boundary. It still preserves the existing wire shape for active prompt, queued prompts, per-agent prompt states, and scheduler state, but it is no longer the hot prompt lifecycle authority.
- Removed projection-based prompt submit admission as an authority. Agent-runtime projections still provide warm queue-front previews and health/read models, but stale projection state cannot force an otherwise idle prompt owner to queue.
- Added regression coverage that deliberately corrupts the compatibility session mirror and verifies completion still succeeds from the prompt owner.
- Promoted `PromptStateOwner` from a private compatibility-app sidecar to a cloneable kernel service shared with `AgentRuntime`. Active-prompt owner resolution and complete queue-front preview can now consult the owner without taking the app lock when a session projection is warm, and regression coverage locks the stale-mirror/no-app-lock path.
- Moved `PromptRuntimeState` into `session/prompt_runtime.rs` as a dedicated compatibility prompt mirror boundary. `RuntimeSession` still flattens and forwards it for wire compatibility, but scattered prompt mutation is no longer embedded in the shared session type.
- Hardened `WorkflowRuntime` lane admission and projection refresh. Warmed missing-session workflow mutations now fail from `SessionStateProjectionStore` without creating a workflow lane or waiting on the compatibility app lock, and workflow workers refresh session/agent-runtime projections directly from session-bearing workflow responses before falling back to a compatibility snapshot.
- Hardened `SessionRuntime` projection publication. Session mailbox workers now publish session-bearing create/config/alias/end responses directly, remove deleted-session projections at the mailbox boundary, and only fall back to compatibility snapshots for responses that do not carry enough session state.
- Trimmed prompt-completion app-lock work in `AgentRuntime`. The agent mailbox now consumes the session projection published by the prompt lifecycle service instead of taking a second compatibility session snapshot after completing a prompt.
- Deferred kernel prompt-submit side effects out of the agent-mailbox acknowledgement path. User-prompt history appends now run through spawned blocking persistence with projection refresh after success, and remote relay prompt submit now returns a dispatch object that is spawned after owner mutation; remote dispatch failure cancels the active prompt, refreshes projections, and records a notice.
- Added projection publication after workflow runtime-tool calls. Direct MCP/relay runtime-tool mutations now republish the session and agent-runtime projections after recording the tool call, so workflow turn acknowledgements and output submissions do not leave warmed workflow inspection reads stale when they bypass the router workflow lane.
- Routed relay-client daemon/workflow requests through `CommandRouter` as `KernelCommandSource::RelayClient`. Proxied relay clients now share the same actor admission, projection refresh, and overload behavior as local IPC/kernel transport requests, regression coverage verifies relay list requests warm the shared session projection, and workflow validation requests no longer die at the relay transport gate.
- Consolidated workflow command handling behind the workflow-runtime boundary. `WorkflowRuntime` workers now invoke the explicit workflow request handler instead of the generic local compatibility handler, and local IPC delegates workflow requests to that same handler so workflow mutation logic is not doubled while `DaemonApp` remains the compatibility mirror.
- Consolidated session command handling behind the session-runtime boundary. `SessionRuntime` workers and local IPC now delegate lifecycle, focus, resize, notice, config, alias, end, and delete commands to one explicit session request handler, removing the duplicate session mutation implementations from the actor and local API paths.
- Consolidated agent prompt command handling behind the agent-runtime boundary. Local IPC, the legacy interactive fallback, and `AgentRuntime` now share one explicit agent request handler for prompt submit, completion, and cancellation while prompt mutation still goes through `KernelAgentService` and `PromptStateOwner`.
- Closed the legacy generic interactive fallback. The fallback lane now accepts only explicit session/agent commands and rejects unsupported commands immediately instead of re-entering the full local API or waiting on the compatibility app lock.
- Routed agent lifecycle mutations through the session runtime lane. `SpawnAgent` and `DestroyAgent` now normalize as interactive session-scoped commands, share the explicit agent request handler with local IPC, and publish session/focus projections from the runtime path instead of entering the normal local fallback.
- Routed runtime MCP through `CommandRouter`. The MCP HTTP server now binds from the router-owned config projection, authenticated local workflow runtime tools dispatch through the router, and forwarded relay workflow runtime tools also enter through the router instead of locking `DaemonApp` directly from transport handlers.
- Added explicit router handlers for relay configuration and remote-machine registry mutations. `ConfigureRelay`, `ApproveRemoteMachine`, `ForgetRemoteMachine`, and `RenameRemoteMachine` now invalidate provider-catalog projections from the router path instead of falling through the generic local compatibility request handler.
- Removed the normal/background generic local compatibility fallback from `CommandRouter`. Every non-interactive request now has an explicit router branch: warmed projection reads return early, cold session/provider/capability paths are named, workflow requests enter `WorkflowRuntime`, and provider auth/login/logout no longer wait on the app lock before running provider-side work.
- Split the remaining router cold-read/provider-sync paths away from the public local API facade. `CommandRouter` now calls named session/list/resolve/state/agent-list and provider-run helpers directly, leaving `DaemonApp::handle_local_request` out of production router dispatch.
- Ran the post-slice live drill set: daemon smoke harness, focused-agent multi-agent prompt routing, shared-endpoint multi-agent prompt routing, workflow progression without terminal pumps, downstream workflow scheduling, join-node scheduling, workflow workspace-claim retry, and CLI workflow graph/outline drill catalogs all passed.
- Removed the unused direct complete-and-auto-advance prompt mutation API from `RuntimeSession` and `PromptRuntimeState`, leaving completion on the kernel lifecycle path that reconciles against the agent-runtime queue-front preview before explicit queue advancement.
- Aligned direct compatibility complete/cancel owner resolution with the agent runtime rule: prefer the focused agent only when it is active, otherwise resolve the single active agent and reject ambiguous multi-active ownership.
- Narrowed compatibility prompt mutation visibility. `RuntimeSession` prompt mutators are now private to the session module tree, and provider dispatch failure cleanup now calls back into `KernelAgentService` instead of reaching into `SessionService` directly.
- Moved public session creation and default-agent bootstrap behind `KernelSessionService`, leaving `DaemonApp::create_session` as a compatibility facade instead of a direct lifecycle owner.
- Added daemon-health projection invariant reporting for session/agent-runtime prompt drift, with regression coverage that detects stale agent-runtime queue-front and queued-count projections.
- Routed public `CreateSession` through the session runtime mailbox boundary, including projection and focused-agent publication for the created session, so creation is not rejected behind the generic interactive lane.
- Collapsed `CreateSession` response construction into one compatibility helper shared by local API and session runtime dispatch, avoiding duplicate create/logging components while the runtime migration is still in progress.
- Added `docs/ops/IMPLEMENTATION_INVARIANTS.md` as the explicit M4.5 gate for ownership, projection refresh, cleanup, overload, health, and tests before final I/O coordination starts.
- Recorded the full A+ sequence for the rest of M4.5: prompt ownership, session ownership, projection correctness, workflow hardening, provider/terminal hardening, hot app-lock removal, docs/invariant lock, and final I/O coordination last.

### Remaining M4.5 work

- Move `KernelSessionService` session state into the new `SessionRuntime` mailbox owner, then finish removing the remaining prompt claim/mirror side effects from the shared app lock.
- Expand actor-owned projections beyond focused-agent routing and warmed session/list/history/provider-run/process/prompt-state/provider-catalog snapshots so remaining provider/read models no longer require synchronous compatibility-store access.
- Keep current workspace claims bounded until actor/projection ownership is complete; return to file-level scopes, port claims, harness enforcement, and transactional mutation/rebase semantics in the final I/O-coordination slice.
- Retire remaining hot request paths that depend on `Arc<Mutex<DaemonApp>>`.
- Run live multi-agent and workflow drills only after all non-I/O ownership/runtime slices above are complete.

## 2026-03-31

### Kernel transport hardening follow-up

- Landed resumable kernel WebSocket transport hardening for the TypeScript CLI: durable event ids, resumable subscribe, reconnect/resubscribe, heartbeat events, and bounded slow-consumer handling.
- Added layered coverage for the hardened transport:
  - TypeScript client transport contract tests
  - daemon kernel-WebSocket integration tests
  - live forced-disconnect and slow-consumer drills
- The remaining transport drills are narrower now:
  - deeper live replay/catch-up validation during active streaming output
  - long-idle heartbeat/liveness validation
- Extracted CLI live-event application and transcript-history seams so incremental pushed-event behavior and reattach catch-up can be tested directly instead of only through manual PTY runs.

### Manual multi-agent session runtime slice

- Landed the first real M4 runtime slice instead of keeping agent handling as footer/chrome-only plumbing.
- Added daemon-owned top-level agent runtime services under `apps/kernel/src/agent/`.
- Direct prompt submission now targets `focused_agent_id`.
- Provider runs are now associated with top-level agents, and the daemon parks/resumes runs as focus changes or the session returns to idle.
- Session history entries now carry `agent_id`, so provider output, notices, and user prompts can be partitioned by agent in the local runtime.
- The TypeScript CLI now supports `individual` and `split` multi-agent response modes plus visible per-agent transcript panes/previews.
- Added shared domain and Prisma updates for focused agents, agent-owned provider runs, and prompt queue targeting.

### Docs alignment update

- Updated roadmap/status docs to reflect that manual multi-agent session runtime is now in progress and no longer just planned plumbing.
- Updated local-running/protocol notes so they describe focused-agent prompt routing, agent-scoped history/provider-run ownership, and the current split-pane CLI behavior.
- Re-sequenced the spec/roadmap around an OpenCode-first development cycle: close one provider deeply first, then polish the CLI, then add multi-platform clients, and only after that expand provider support.

### Known follow-up from current code state

- The OpenCode-backed multi-agent runtime path is not fully stable yet.
- The current daemon integration suite still reports failures around:
  - provider-run launch health checks in the OpenCode event-stream path
  - delayed local-response handling through the local transport
- The current split-pane CLI is still a first slice centered on the primary transcript plus up to two auxiliary panes.

## 2026-03-30

### CLI TUI repaint skill

- Added `docs/CLI_TUI_REPAINT_SKILL.md` as a repo-native repaint playbook for future agents working on OpenTUI/JVX visual update bugs.
- Captured the main lesson from split-pane focus bugs: proactive multi-pass repainting and child-renderable rebuilds matter more than only changing parent pane colors.

## 2026-03-29

### Multi-agent docs alignment update

- Reviewed the current daemon and TypeScript CLI agent plumbing after reproducing that focused-agent changes currently affect footer/chrome state more than actual runtime routing.
- Updated `README.md`, `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, `docs/PROTOCOL.md`, `docs/RUNNING_LOCAL.md`, `docs/ROADMAP.md`, and `docs/ops/TASKS.md` so they distinguish three things clearly:
  - current single-agent-effective runtime behavior
  - already-landed session-agent metadata/focus plumbing (`/agent ...`, `Ctrl+A`, focused-agent state)
  - the intended next milestone: manual multi-agent sessions with per-agent context/history and split-pane CLI rendering before workflow automation
- Reframed the roadmap so manual multi-agent session execution is the next step ahead of daemon-scheduled workflow topology work.

## 2026-03-22

### CLI transcript highlighting update

- Added `docs/CLI_TRANSCRIPT_HIGHLIGHTING_PLAN.md` to define transcript syntax highlighting as an M3 TypeScript CLI subphase separate from LSP.
- Implemented markdown-aware assistant/reasoning transcript rendering in the TypeScript CLI.
- Implemented syntax-highlighted fenced code blocks in the TypeScript CLI transcript using OpenTUI parser/code rendering infrastructure.

## 2026-03-16

### Context

- M0 assessed as not complete yet.
- M0 implementation direction clarified and accepted:
  - include Rust daemon bootstrap
  - use GitHub Actions now
  - Option A baseline structure
  - smoke tests + minimal domain contract tests
  - include Prisma schema now

### Changes made in this update

- Added `docs/M0_IMPLEMENTATION_CHECKLIST.md` with concrete M0 task breakdown and DoD.
- Added `docs/ops/TASKS.md` lightweight board for backlog/in-progress/done tracking.
- Added this `docs/ops/PROGRESS_LOG.md` for chronological handoff notes.

### Next recommended execution order

1. M0-001 workspace root and scripts
2. M0-004 server stub
3. M0-002 + M0-003 domain + contract tests
4. M0-005 daemon rust crate
5. M0-006 prisma schema
6. M0-007 CI workflow
7. M0-008 docs alignment and status update

### 3.1 implementation progress

- Completed workspace/package bootstrapping (`M0-001`, `M0-002`, `M0-004`, `M0-005`).
- Added root workspace scripts for `build`, `lint`, `test`, and daemon test invocation.

### M0 completion update

- Expanded `packages/domain` to cover the full M0 entity baseline and added contract tests.
- Added `prisma/schema.prisma` for the initial persistence model.
- Added `.github/workflows/ci.yml` for pnpm and Rust verification.
- Updated `README.md`, `docs/CONTRIBUTING.md`, `docs/ROADMAP.md`, and `docs/M0_IMPLEMENTATION_CHECKLIST.md`.
- M0 verification now consists of `pnpm lint`, `pnpm build`, `pnpm test`, and `cargo test --manifest-path apps/kernel/Cargo.toml`.
- M0 is considered complete once those commands pass on the repository state produced in this update.

### M1 planning update

- Added `docs/M1_IMPLEMENTATION_CHECKLIST.md` to break M1 into concrete runtime, PTY, attachment, provider, and test workstreams.
- Seeded `docs/ops/TASKS.md` with `M1-001` through `M1-008`.
- Recommended M1 execution order:
  1. daemon runtime skeleton
  2. session lifecycle service
  3. attachment and shared-session interaction logic
  4. provider adapter baseline
  5. PTY manager and terminal fan-out
  6. local harness/API
  7. runtime tests
  8. docs/protocol alignment

### M1-001 implementation update

- Added the daemon runtime skeleton in `apps/kernel` with:
  - `app.rs` for bootstrap and shutdown handling
  - `config.rs` for daemon configuration loading/validation
  - `error.rs` for structured daemon runtime errors
  - a lean application container that owns only real runtime services
- Switched the daemon binary to a Tokio-based async entrypoint and documented Tokio as the M1 async runtime baseline.
- Added crate tests to verify config validation and top-level runtime wiring.

### M1-002 implementation update

- Implemented an in-memory session lifecycle service in `apps/kernel/src/session/`.
- Added runtime session records for workspace/worktree/host ownership, active provider run, and attachment membership state. This was later extended to prompt-queue and config-state ownership.
- Added explicit session transition validation for `created`, `active`, `parked`, and `ended` states.
- Added Rust unit tests for create/get/list/end flows, invalid transitions, and unknown-session lookup behavior.
- Refined the session model to remove duplicated derived state, keep host metadata out of the in-memory store, and encapsulate session mutation behind methods.

### M1-003 implementation update

- Initial implementation added a real `attachment` runtime module with in-memory attachment records and daemon-facing event recording.
- The original implementation used controller-style semantics, which were later superseded by the shared-attachment prompt-queue/config-state model.
- Current runtime behavior is governed by the later shared-attachment refactor notes in this log.

## 2026-03-17

### Scope clarification update

- Multi-agent workflow execution is explicitly in scope for v1.
- Circular topology is the earlier implementation priority inside v1.
- Hierarchical topology remains in scope for v1, but is planned for a later stage of v1 after lower-level runtime, capability, control, and protocol foundations stabilize.

### Documentation alignment update

- Updated `README.md` to distinguish current implementation status from planned v1 scope.
- Updated `agents/AGENTS.md`, `docs/spec-v1.md`, `docs/ARCHITECTURE.md`, `docs/ROADMAP.md`, and `docs/PROTOCOL.md` so workflow execution is clearly in v1 scope, with circular earlier and hierarchical later within v1.
- Corrected planning/status drift by marking `M1-004` as pending again in `docs/M1_IMPLEMENTATION_CHECKLIST.md` and `docs/ops/TASKS.md`.
- Updated `docs/CONTRIBUTING.md` so testing guidance and baseline-command wording match the current repository state and the new workflow-oriented scope.

### Runtime review update

- Reviewed the current daemon runtime against the new workflow-oriented specifications before continuing M1 work.
- Added explicit session execution mode metadata so the session model now distinguishes current single-agent behavior from future multi-agent workflow mode.
- Kept PTY ownership and terminal stream handling keyed by provider run so future node-scoped provider runs can reuse the same runtime surfaces.

### M1-004 implementation update

- Added a provider adapter trait, registry, and deterministic `dev-stub` adapter for local runtime tests without depending on external provider CLIs.
- Added in-memory provider run lifecycle management for launch, park, resume, terminate, and session active-run ownership.
- Added provider runtime tests covering first launch, automatic parking on active-run replacement, and inconsistent active-run rejection.

### M1-005 implementation update

- Integrated `portable-pty` as the PTY baseline for the daemon runtime.
- Added a PTY manager for spawn, write, resize, output draining, and process cleanup keyed by provider run.
- Added terminal stream records for attachment-driven input routing and multi-attachment output fan-out.
- Added daemon-level tests covering PTY spawn, terminal input/write path, resize behavior, and output fan-out to multiple attachments.

### Runtime hardening update

- Hardened PTY lifecycle ownership so the daemon now retains child-process handles and performs explicit PTY cleanup on provider/session teardown instead of only dropping in-memory records.
- Updated failed provider-switch handling to resume the previously active run automatically when the replacement PTY cannot be established, and record a user-facing runtime notice for that recovery path.
- Expanded `packages/domain` with workflow-oriented v1 entities and enums so shared contracts no longer stop at the earlier single-agent baseline.

### M1-006 implementation update

- Added a local daemon request/response API in `apps/kernel/src/local/` covering create, attach, detach, provider launch, session state reads, notice polling, prompt submit/complete, config updates, terminal output polling, terminal resize, and session end flows.
- Added a local smoke harness binary in `apps/kernel/src/bin/arroba-kernel-harness.rs` plus runtime tests proving a managed-session path through the PTY and terminal fan-out surfaces.
- Updated `docs/PROTOCOL.md` to record the local-first daemon API baseline for M1 flows.

### Domain and schema alignment update

- Expanded `packages/domain/src/index.ts` and `packages/domain/src/index.test.ts` to reflect workflow-oriented runtime naming, richer workflow entities, handoff/completion fields, worktree-isolation modes, and delivery statuses.
- Updated `prisma/schema.prisma` to add workflow-oriented enums, execution-mode/session fields, and baseline models for workflow definitions, runs, nodes, edges, node messages, worktree assignments, and aggregation state.

### M1-007 implementation update

- Added daemon integration tests in `apps/kernel/tests/runtime_integration.rs`.
- Covered session lifecycle cleanup, prompt queue/notification behavior, provider run switching with PTY-backed terminal flow, and the local managed-session smoke harness path.
- Marked the M1 testing/verification checklist items complete now that daemon integration coverage passes and the documented JS workspace verification plus dedicated daemon verification commands both pass.

### Shared-attachment refactor update

- Replaced the earlier controller/observer runtime model with shared attachment participation in the daemon runtime.
- Added daemon-owned prompt queue state, active-prompt completion/advancement, and queued-message notices for the other attachments in a session.
- Added canonical session config state with versioned updates plus propagation notices to the rest of the session attachments.
- Updated local daemon APIs, domain types, Prisma schema, and daemon tests to match the shared-attachment queue/config model.

### M1-008 documentation alignment update

- Aligned `docs/PROTOCOL.md` with the current local daemon API: session state reads, notice polling, prompt submit/complete, and config update responses now match the implemented runtime surface.
- Aligned `docs/ARCHITECTURE.md`, `docs/spec-v1.md`, `agents/AGENTS.md`, and `docs/CONTRIBUTING.md` with the shared-attachment prompt/config model and the current client/daemon responsibilities.
- Reconciled the M1 checklist and task board with the now-complete runtime, integration coverage, and documentation work for M1-001 through M1-008.

### M1 closure update

- Added explicit scheduler-state ownership and primary worktree-assignment-compatible session state so the remaining workflow-compatibility guardrails are satisfied without redesigning the current runtime.
- Closed the remaining M1 checklist items and marked M1 complete in the project status docs.

### M2 planning update

- Added `docs/M2_IMPLEMENTATION_CHECKLIST.md` to break M2 into concrete capability workstreams, local API alignment, testing, and documentation requirements.
- Seeded the task board with initial M2 planning and implementation tasks.

### M2 shell capability baseline update

- Added a new `capability` module in the daemon runtime and implemented a structured shell command capability service.
- Exposed shell command execution through the local daemon API with structured stdout/stderr/exit-code results.
- Added daemon tests and local API tests covering successful shell execution, non-zero exits, and working-directory scoping.

### M2 shell hardening update

- Added timeout bounds and worktree-boundary validation to the shell capability so long-running or escaped commands do not silently bypass daemon safety expectations.
- Added attachment-aware authorization for shell execution through the local daemon API.
- Tightened prompt lifecycle UX by emitting notices when queued prompts are dropped because an attachment detached.

### M2 filesystem and git capability update

- Added structured directory tree capability support scoped to the session worktree.
- Added file read and file edit capabilities with structured results and worktree-boundary validation.
- Added structured git/worktree inspection capability for branch and status reporting.
- Exposed the new capabilities through the local daemon API and added daemon/local API tests for each baseline capability.

### M2 screenshot baseline update

- Added a screenshot capability contract and local runtime baseline with structured unavailable fallback when no capture backend is available.
- Exposed screenshot capture through the local daemon API and added daemon/local API tests for the baseline unavailable path.

### M2 transfer baseline update

- Added a daemon-owned file transfer storage baseline that copies source files from the session worktree into a session artifact root.
- Exposed transfer storage through the local daemon API and added daemon/local API tests for the stored-artifact path.

### Roadmap reprioritization update

- Reordered the near-term roadmap around one end-to-end local success path before broader platform scope.

### Workflow runtime phase 1 update

- Added daemon-owned workflow runtime entities for `WorkflowRun`, `WorkflowNodeRun`, and `WorkflowMessage` on the current session runtime path.
- Added local API invoke/list/get/cancel flow for workflow runs, keyed off existing workflow endpoints.
- Added workflow-run daemon tests plus a local IPC socket round-trip covering create -> list -> get -> cancel on the new transport surface.
- Kept this slice intentionally narrow: endpoint invocation now persists runnable workflow state, but it does not yet schedule provider turns or execute graph handoffs.
- New immediate priority: local daemon + CLI + OpenCode integration with prompt submission and live output streaming.
- Deferred broader local capabilities, additional providers, multi-agent workflows, relay/web surfaces, provider switching, memory, compaction, and per-agent extension management to later milestones after that baseline is proven.

### Workflow runtime scheduler slice update

- Added daemon-owned entry-node scheduling for endpoint-triggered workflow runs on top of the existing prompt queue and provider runtime.
- Workflow-owned prompts now carry workflow run/node run context so prompt start, completion, cancellation, and unexpected provider exits reconcile back into `WorkflowRun` and `WorkflowNodeRun` state.
- Entry-node scheduling can auto-launch a provider run for the bound agent when one is not already active, then dispatch the workflow prompt through the same top-level agent runtime.
- Kept this slice intentionally narrow: there is still no CLI `/workflow run` surface yet, and downstream node handoffs are not executed. Runs currently become `Completed` when the entry node has no outgoing edges, or `Waiting` when downstream edges exist.

### Workflow runtime handoff slice update

- Added a daemon-owned structured handoff payload for downstream routing, including workflow run id, workflow id, source node run id, source node id, source agent id, target node id, and the root invocation prompt.
- Node completion now creates one workflow message per outgoing edge, creates one downstream node run per routed message, and schedules those downstream node prompts through the same prompt/provider runtime.
- Queued workflow prompts can now auto-launch the target agent's provider run when they reach the front of the session queue, so chained workflow execution no longer depends on pre-launched runs.
- Added daemon tests plus a local IPC socket round-trip covering entry execution -> downstream routing -> downstream completion for a simple chained workflow.

### Workflow join gating slice update

- Workflow handoffs are now buffered on the target side instead of immediately creating one node run per incoming edge.
- Join nodes default to `all_inputs` gating when their indegree is greater than one, so one downstream node run starts only after all required parent messages are present.
- Workflow messages now record which node run consumed them, making aggregated activations and later audit/replay possible without forwarding transcript history.
- Fixed a queue-advancement bug where completing one workflow prompt could overwrite an already-started downstream prompt instead of only advancing when no active prompt remained.
- Added daemon coverage for service-level join gating and a local API round-trip proving that join nodes do not start early and do start exactly once after the final parent completes.

### Workflow runtime CLI slice update

- Wired `/workflow run`, `/workflow runs`, and `/workflow cancel` into the TypeScript CLI on top of the existing daemon workflow-run API.
- Added command-center entries and CLI help text for the new workflow runtime commands.
- Updated the workflow canvas to show the selected workflow's display run id/status and per-node status derived from the newest active run, falling back to the newest run overall.
- Added CLI tests covering workflow runtime commands plus graph-layout tests for runtime status rendering.

### M2 checklist realignment update

- Rewrote `docs/M2_IMPLEMENTATION_CHECKLIST.md` so it now matches the new M2 milestone instead of the earlier capability-first ordering.
- Broke the M2 task board into concrete sub-workstreams: daemon transport, CLI app, OpenCode adapter, and end-to-end smoke coverage.
- Explicitly marked the already-implemented local capability work as preserved but deferred relative to the new OpenCode-first critical path.

### M2 closure update

- Closed M2 formally after landing the real local daemon IPC transport, minimal local CLI, real `opencode` adapter, and end-to-end delayed-output smoke coverage through the daemon.
- Updated `README.md`, `docs/ROADMAP.md`, `docs/M2_IMPLEMENTATION_CHECKLIST.md`, `docs/PROTOCOL.md`, `docs/ARCHITECTURE.md`, and `docs/ops/TASKS.md` so repository status now reflects M2 as complete and M3 as the next milestone.
- Recorded shipped M2 implementation work against commit `727a97f`.

### TypeScript CLI migration update

- Promoted `apps/cli` to the primary local CLI implementation using TypeScript + OpenTUI.
- Kept `arroba-cli` as a Rust compatibility launcher that builds and starts the TypeScript client.
- Removed the previous Rust-only CLI after the TypeScript client became the only supported local CLI implementation.

### TypeScript CLI hardening update

- Added retry/backoff policy for transient local IPC polling failures in the TypeScript CLI instead of treating the first poll error as immediately fatal.
- Changed TypeScript CLI exit semantics so cleanup failures remain visible and require a second explicit exit attempt before forcing shutdown.
- Added initial TypeScript CLI behavior tests around retry/exit policy helpers and updated the roadmap/checklist so M3 now explicitly calls out TypeScript CLI hardening before slash-command expansion.

### M3 observability priority update

- Raised a project-wide logging/debugging system ahead of the remaining M3 tasks after the TypeScript CLI migration.
- Documented the intended baseline as one shared machine-local log root with per-process structured log files and shared session/provider/client correlation fields.
- Reprioritized the next M3 slice toward persistent session management: detached sessions should remain resumable, deletion should be explicit, the CLI should support a no-session state after deletion, and session references should move toward commit-like ids plus optional aliases.
- Marked privacy policy, retention, and content-capture scope as explicit design decisions to resolve before implementation.

### M3 logging foundation update

- Added a shared NDJSON logging baseline for the daemon, the Rust `arroba-cli` launcher, and the primary TypeScript CLI.
- Standardized log-root resolution around `ARROBA_LOG_DIR`, `XDG_STATE_HOME/arroba/logs`, `~/.local/state/arroba/logs`, then `./.arroba/logs`.
- Added built-in local log inspection through `arroba-cli logs`.
- Removed the previous ad hoc CLI debug-file hook and daemon IPC debug stderr hook in favor of the shared logger.
- Updated contributor and agent guidance so future debug work must extend the shared logging system instead of introducing separate mechanisms.

### Workflow runtime completion snapshot update

- Extended workflow node completion so the daemon now derives a summary-only completion payload from persisted provider output for the exact provider run that settled the node.
- Persisted that summary-only payload on completed `WorkflowNodeRun` records and forwarded it in downstream workflow handoff payloads, while keeping the full transcript only in session history for audit rather than as workflow output.
- Added daemon coverage proving that downstream handoffs retain the upstream node summary when provider output exists before prompt completion.

### Workflow runtime artifact reference update

- Added optional artifact refs to the workflow completion payload so a completed node can forward `summary + artifacts` without forwarding transcript data.
- Namespaced session artifacts by attachment/workflow source under the daemon artifact root so workflow-owned artifacts can be discovered without sweeping unrelated session files.
- Added daemon coverage proving that a workflow-owned artifact appears on the completed node run and in the downstream handoff payload.

### Workflow explicit output contract update

- Changed the workflow runtime contract so `summary` remains human-facing while downstream routing uses an explicit `output.message` plus optional artifact refs.
- Updated workflow-owned prompts to request a structured JSON completion envelope with separate `summary` and `output`.
- Reframed the docs around graph-derived execution and per-node gating/release policy instead of user-declared circular vs hierarchical workflow modes.

### M4.5 prompt dispatch and terminal cleanup update

- Removed provider-prompt worktree claim admission from prompt submit, so cross-session same-worktree prompts are no longer rejected before `PromptSubmitted`.
- Moved local provider prompt PTY writes and provider actor enqueue work into the spawned provider-run operation dispatch after owner-backed prompt mutation, reducing work done inline by the per-agent mailbox response path.
- Added session-runtime terminal cleanup on session end/delete so terminal input, pending output, notices, completions, and terminal backlog health do not retain stale records for removed sessions.
- Added CLI request helpers for `CompletePrompt` and `AckWorkflowTurn` so deterministic live drills can use the public kernel API once the non-I/O runtime slices are complete.

### M4.5 compatibility facade retirement update

- Added an explicit facade-retirement checklist to the M4.5 plan, separating public facade retirement, router independence, actor ownership, runtime service extraction, and final compatibility handler deletion.
- Added a router-backed in-process local daemon client for tests and smoke harnesses that need to send `LocalDaemonRequest` without calling `DaemonApp::handle_local_request` directly.
- Moved the local smoke harness and external daemon integration test off direct `handle_local_request` calls.
- Demoted `DaemonApp::handle_local_request` to crate-private compatibility surface. It remains only for internal compatibility tests and transitional service code until later ownership slices remove the remaining facade-only handlers.

### M4.5 session ownership cutover update

- Completed the point-2 cutover for the local session lifecycle: attach, detach, focus, cycle focus, resize validation, end, and delete now run through owned `SessionRuntime` state instead of `KernelSessionService<&mut DaemonApp>` mutation paths.
- Kept resize and provider process teardown behind narrow app side-effect ports while owned stores perform session/provider validation, queue cleanup, prompt-owner cleanup, provider-run projection updates, and session projection cleanup.
- Added no-app-lock regressions for attach/detach, focus/cycle, end/delete, resize validation, and router delete behavior.

### M4.5 prompt ownership and provider advancement update

- Broadened the point-3 owned prompt path beyond the earlier single-agent structured case: local multi-agent prompt submit, PTY prompt submit, PTY cancellation acknowledgement, and queued prompt advancement now mutate prompt owner/session mirrors through owned runtime state.
- Added an explicit prompt-abort dispatch source attachment so PTY cancellation can acknowledge immediately and send `Ctrl-C` through the side-effect port asynchronously.
- Started point 4 by moving provider post-launch queued prompt advancement out of `DaemonApp::advance_next_queued_prompt`; provider launch completion now activates the next prompt and builds dispatch from owned runtime state.

### M4.5 prompt ownership closure update

- Closed point 3 by removing the production `CompatibilityRuntimeState` fallbacks to `KernelAgentService` prompt submit/cancel/complete methods.
- Moved remote prompt submit/cancel/complete owner mutation into owned runtime state while keeping relay I/O as an explicit side-effect port.
- Restored workflow prompt start/completion handoffs after owned prompt mutation and refreshed owned session projections so workflow run reads observe completed/downstream state.

### M4.5 provider output ownership update

- Completed point 4 for production runtime paths by moving active provider-output pumping, structured output batch application, structured prompt job reaping, prompt activity settlement, and provider-exit prompt cleanup onto owned runtime stores.
- Removed the app-level provider-exit prompt settlement helper; provider liveness now closes/cancels owned prompts and advances queued work without routing through `DaemonApp` prompt lifecycle helpers.
- Kept remaining PTY spawn/poll/drain/write calls as narrow process I/O side-effect ports while provider run state, output fanout/history, prompt claims, and projections are owned by runtime state.

### M4.5 workflow command ownership update

- Cut workflow command mutation over to the runtime-state owner: workflow definition, endpoint, graph, schema, watchdog, queued-prompt, validation, and ack commands now mutate cloneable session/workflow stores directly from `CompatibilityRuntimeState`.
- Deleted the transitional `KernelWorkflowService` module and removed `WorkflowRuntimeStore`'s per-method app-backed service delegation.
- Kept invoke/cancel/resume workflow progression behind explicit runtime-state scheduler ports while the scheduler internals remain the remaining point-5 work before transport/relay cleanup.

### M4.5 workflow runtime-tool ownership update

- Moved owned workflow resume, workflow prompt start/cancel bookkeeping, workflow provider-run ensure, blocked workspace-claim retry, and local/forwarded workflow runtime-tool mutation into `CompatibilityRuntimeOwnedState` store operations.
- Runtime MCP calls authenticated by provider tokens now resolve local workflow turns from owned provider/prompt/session state before recording ack, validation, output-submission, and console tool effects.
- Kept workflow prompt completion scheduling on the existing scheduler renderer because that path still owns mailbox rendering, outgoing edge contracts, join-node readiness, and queued downstream prompt text fidelity; this is the remaining hard center of point 5 rather than point 6.

### M4.5 workflow progression ownership closure update

- Moved workflow prompt completion scheduling, invoke admission/start, cancel cleanup, queued-prompt start, provider-run ensure, and blocked-claim retry behavior onto owned runtime-state operations for the production local workflow path.
- Preserved workflow join-node readiness by preventing incomplete joins from being dispatched while still retrying the blocked sibling branch after workspace-claim release.
- Left the old app workflow-runtime helpers only as compatibility/test-facing surfaces; router/workflow-lane invoke and cancel no longer use the app-backed workflow launch or cancellation helpers.

### M4.5 transport and relay ownership update

- Closed point 6 for the production relay transport path by routing relay registration/config/key reads, subscription authorization/watch snapshots, peer lease commands, leased prompt settlement, and remote projection events through `CommandRouter` relay/runtime ports instead of direct relay-client `DaemonApp` state access.
- Kept connector bootstrap and the unused relay test helper as the only direct `relay_client.rs` app-lock reads; production daemon/workflow relay requests, runtime MCP forwarding, subscription replay, and peer prompt handling now enter the router/runtime boundary.
- Hardened workflow cancellation cleanup by releasing session workflow-dispatch workspace claims when a run is cancelled, preventing subsequent queued or manual workflow launches from inheriting stale claim conflicts.

### M4.5 compatibility deletion update

- Started point 7 as deletion, not quarantine: removed the test-only no-owned `CompatibilityRuntimeState::new` constructor and moved the remaining actor/executor regressions onto owned runtime-state construction so the old generic fallback can no longer be instantiated by tests.
- Tightened the owned workflow claim lifecycle by blocking entry-node dispatch on workspace-claim conflicts, releasing workflow-node claims on completion/cancellation/validation-stop, and retrying blocked workflow work after workflow-node claim release.
- Verified the deletion slice with the full daemon lib suite plus runtime, WebSocket, and relay transport integration suites.

### M4.5 runtime fallback deletion closure

- Closed point 7 for production command/runtime ownership by replacing `CompatibilityRuntimeState` with mandatory `KernelRuntimeState`/`KernelRuntimeOwnedState`; runtime construction no longer carries optional owned state or a no-owned compatibility fallback.
- Deleted the unreachable app-backed runtime fallback branches for session reads/mutations, prompt dispatch/abort cleanup, provider launch mutation, terminal-output pumping, capability authorization, and workflow runtime-tool dispatch. Remaining `DaemonApp` access is limited to named side-effect ports for PTY/process/relay operations and remote-agent lifecycle.
- Removed the obsolete app-backed prompt-dispatch runtime helper and kept the legacy relay helper test-only, leaving `DaemonApp` as bootstrap/composition plus explicit side-effect services rather than the command mutation owner.

### M4.5 live drill gate update

- Added the live drill matrix and adopted **freeform multi-agent mode** as the name for normal non-workflow multi-agent sessions.
- Local freeform OpenCode + Codex passed after fixing router bootstrap lock contention during concurrent router construction.
- Local workflow drills are blocked on entry-node live provider completion: downstream workflow turns can ack and emit output, but entry-node turns that launch a provider lazily can complete without a drained structured output payload. The reproducible hard-fail command is `node apps/cli/scripts/live-workflow-runtime-drill.mjs --spawn-daemon --scenario validated-increment-chain --providers codex,opencode --model gpt-5.4 --poll-limit 120 --poll-interval-ms 2000`.
- Current drill fixes in progress: workflow prompts rendered by owned runtime now include node instructions and validation/tool rules, structured provider completion ignores stale lifecycle events, queued workflow prompts advanced after provider launch receive workflow-start bookkeeping, and terminal output pumping drains all running session provider runs rather than only the focused active run.

### M4.5 local workflow live drill closure

- Closed live-drill point 2 locally: the full workflow catalog passes with a spawned local daemon and `opencode,codex` providers, including validated handoff, workflow console tools, final run output, cyclic final output, budgeted cyclic final output, and cyclic intermediate-output/final-output flows.
- Fixed the hard runtime issues found by the catalog: invoke now enqueues owned entry dispatches, workflow console write no longer deadlocks on session-store lock ordering, resolved workflow failures are cleaned after successful retry/final submission, node turn budgets ignore no-payload recovery attempts, and final-output turns no longer emit stale downstream handoff failures before pending final output is committed.
- Tightened the live workflow drill prompts where the intended first-turn payload was ambiguous for live providers, using exact fenced workflow output examples for entry turns and explicit console-output payload contracts.

### M4.5 remote freeform relay live drill closure

- Closed live-drill point 3: `node apps/cli/scripts/live-remote-multi-agent-relay-drill.mjs --providers opencode,codex --model gpt-5.4 --timeout-ms 240000 --poll-ms 1000` passed with a relay, home daemon, worker daemon, direct local client, and relayed client.
- The drill spawned OpenCode and Codex local sidecar agents plus OpenCode and Codex worker-machine leased agents in the same freeform multi-agent session. Prompts submitted from both local and relayed clients completed, both clients observed all four completions, the relayed client observed relay `transport_closed` and `transport_resumed`, and worker-machine agents completed prompts after relay restart.
- Updated the remote freeform drill automation to accept `--providers opencode,codex` so point 3 is a single mixed-provider session instead of two independent single-provider passes.

### M4.5 remote workflow relay live drill closure

- Closed live-drill point 4: the remote workflow catalog passes through relay with `opencode,codex`, a home daemon, a worker daemon, worker-machine leased agents, forwarded workflow runtime tools, cyclic progression, validated intermediate output, final workflow output, and clean final workflow projections.
- Fixed the remote workflow hard center found by the drills: remote entry turns now start and settle leased prompts, forwarded worker runtime-tool calls complete worker-side leased workflow prompts after validated handoff/final-output submission, and workflow completion can fall back to the last successful validation tool payload when a live provider validates but does not emit the final fenced block.
- Tightened the live cyclic drill scenarios to exercise one deterministic cycle rather than long model-dependent loops, keeping coverage for cyclic handoff, turn budget, intermediate output, and final workflow-output submission without making the gate depend on repeated live-provider instruction compliance.

### M7.5 embedded shell scriptability update

- Moved line-oriented Arroba shell script execution into `packages/kernel-client/src/shell-script.ts`, so standalone `arroba-shell` and the embedded workflow-pane shell share `source`/`run`, nested script loading, variables, line diagnostics, and context propagation.
- Updated the workflow-pane shell to select the workflow returned in shell context after workflow creation/show/load commands, keeping the visible workflow graph synchronized with shell-driven mutations.
- Manual TUI drill confirmed direct `@` workflow graph creation and `@ source <file>` graph loading update the workflow pane to the expected node/edge/endpoint counts. Dev-stub workflow execution still fails with `missing_structured_output`; provider-backed runtime behavior remains covered by the workflow runtime drill suite.

### M7.5 shell context and hardening update

- Added shell-local `context` and `pwd` commands for standalone and embedded shells; they render the active workspace, worktree, session, attachment, agent, workflow, provider/model/effort, and shell variables without making a kernel request.
- Extended parser/executor/shell usage tests plus the live shell scriptability drill to cover `context`/`pwd` and documented the remaining shared-executor command gaps against the TUI slash surface.
- Revalidated standalone shell scriptability with `pnpm --filter @arroba/cli run shell:drill`. A provider-backed embedded TUI runtime drill was attempted manually, but the PTY input path is still unreliable for slash/workflow-mode automation; the committed validation remains the shared executor tests plus the scriptability drill rather than a flaky TUI automation artifact.

### M7.5 CLI automation drill update

- Added `--automation-socket` to the CLI so live drills can drive embedded UI integration through semantic JSONL actions rather than raw PTY keystrokes.
- The automation API supports `ping`, `switch_screen`, `workspace_shell_exec`, `snapshot`, `wait_for`, and `exit`, returning structured snapshots for screen mode, selected workflow/node, workflow graph counts, workflow runs, shell context, shell transcript, and footer state.
- Added `pnpm --filter @arroba/cli run embedded-shell:drill`, which launches the real CLI under a PTY, sources a workflow script through the workflow-pane shell, and validates selected-workflow graph/source updates through automation snapshots.

### M7.5 shell prompt command update

- Added shared shell `prompt [agent-ref] <prompt> [--wait] [--show-reply|--show-summary]` support for standalone `arroba-shell` and the embedded workflow-pane shell.
- No-wait prompt submission returns the prompt id immediately; wait/show modes poll prompt completion, read session history, and render prompt-id-headed output blobs with aligned content.
- Changed `context` from a purely local shell command into a kernel-aware shared command when a session is set, so it can show the current agent as `(busy)` after a no-wait shell prompt.

### M7.5 Arroba Shell milestone closure

- Closed M7.5 after landing `arroba-shell` as a sibling app to the TUI CLI, shared shell execution in `packages/kernel-client`, line-oriented scripts, embedded workflow-pane shell support, reliable CLI automation snapshots, and freeform prompt submission from shell surfaces.
- Validated closeout with `pnpm --filter @arroba/kernel-client test`, `pnpm --filter @arroba/shell test`, `pnpm --filter @arroba/cli test -- workspace-shell.test.ts command-actions.test.ts`, `pnpm --filter @arroba/cli run shell:drill`, `pnpm --filter @arroba/cli run embedded-shell:drill`, and `git diff --check`.
- Deferred remaining shell parity gaps to later hardening: session/agent deletion aliases, machine approval/rename/forget, relay configuration, workflow node instruction editor actions, workflow endpoint removal, and provider-native namespace passthrough.

### M8 history and persistence plan kickoff

- Added the M8 plan for operational transcript history, optional archive adapters, durable kernel state, manual remote restart reconciliation, provider resume descriptors, and Git observation events.
- Locked in the v1 split: operational history stays Arroba-owned/local for active UX, while archive history is optional and adapter-backed; if archive is disabled, retained operational history remains searchable and expired/deleted transcript content can disappear.
- Added TOML-backed config structs for history operational policy, archive mode/policy, and durable state policy without changing runtime storage behavior yet.

### M8 canonical history event model update

- Added provider-neutral canonical history event types for transcript, workflow, capability, remote-machine, and Git observation events.
- Added turn/provider/model/worktree attribution fields plus candidate attribution lists for ambiguous Git and multi-agent cases.
- Added a compatibility conversion from existing `SessionHistoryEntry` transcript records into canonical `HistoryEvent` records while leaving the current JSONL runtime storage path unchanged.

### M8 operational SQLite store foundation

- Added the first operational history SQLite store behind a new `OperationalHistoryStore` API without cutting over `GetSessionHistory` yet.
- The store creates its schema, enables SQLite WAL mode, appends canonical events idempotently by `event_id`, and can load ordered events by session with optional agent filtering.
- Added focused coverage for opening the store, idempotent append, and session/agent event loading.

### M8 operational history dual-write update

- Wired the operational history store into `DaemonApp` and owned runtime state while keeping the existing JSONL session history path as the read source.
- Prompt, provider-output, and notice transcript appends now dual-write canonical operational history events after the legacy append succeeds.
- Test configs now use isolated operational history database paths, and session history preservation coverage verifies the operational store receives restored prompt transcript events.

### M8 session history read cutover update

- Changed app and router session-history reads to prefer operational history and fall back to legacy JSONL only when no operational entries exist for a pre-cutover session.
- Added canonical-event to `SessionHistoryEntry` conversion so existing transcript pagination and CLI rendering can continue while reading from operational history.
- Added focused coverage proving `session_history_page` reads transcript entries from the operational store.

### M8 history query/search API update

- Added kernel `QueryHistory` and `SearchHistory` requests backed by the operational SQLite store.
- The query path returns canonical `HistoryEvent` rows with filters for session, agent, provider, model, workflow, machine, repo/worktree, event kind, text, sequence cursor, and bounded limits.
- Added TypeScript kernel-client history event/query types and request builders, plus focused store and router coverage for operational history queries.

### M8 archive adapter client foundation

- Added a history archive adapter client for disabled and external archive modes.
- External adapters can now receive canonical `HistoryEvent` batches at `/arroba/history/events`, expose `/arroba/history/capabilities`, and optionally require bearer-token auth through the configured token environment variable.
- Archive append responses are validated for durable acceptance of every event before callers can treat the archive write as successful.

### M8 archive outbox checkpoint update

- Added a durable `history_archive_outbox` table to the operational SQLite store.
- The store can idempotently enqueue canonical history events for archive export, reload pending work after reopening, record failed attempts, and mark adapter-accepted events as archived.
- Extended operational-history store coverage to verify duplicate enqueue handling, retry metadata, acceptance marking, and reopen persistence.

### M8 archive enqueue/exporter update

- External archive mode now enqueues newly appended transcript history events into the durable archive outbox.
- Added a one-shot `HistoryArchiveExporter` that loads pending outbox events, calls the configured archive adapter, marks accepted events archived, and records failed/rejected attempts for retry.
- Added focused coverage for archive-mode prompt enqueue and adapter-backed outbox flushing.

### M8 retention prune safety update

- Added operational-history pruning before a timestamp cutoff, with separate modes for archive-disabled deletion and verified-archive-only deletion.
- Added session markers that disable legacy JSONL fallback after pruning removes all operational history for a session, preventing deleted retained history from being resurrected by the compatibility store.
- Tightened session-history fallback so JSONL is used only for truly pre-cutover sessions with no operational rows and no retention marker.

### M8 durable state store foundation

- Added `DurableKernelStateStore`, a WAL-backed SQLite store for kernel state snapshots and append-only state events.
- Added config path resolution for the durable state database from `[state].path`.
- Added focused coverage for appending ordered state events, saving a snapshot, reopening the store, and loading the latest snapshot.

### M8 durable state app wiring update

- Wired `DurableKernelStateStore` into `DaemonApp` using the configured `[state].path`.
- Session creation now writes a `session.created` durable state event containing the created session and default agent payload.
- Added focused coverage that creates a session through `KernelSessionService` and verifies the durable event is written.

### M8 durable lifecycle event update

- Local agent spawn now writes an `agent.created` durable state event.
- Session end now writes a `session.ended` durable state event with removed attachments, terminated provider run ids, and removed agent references.
- Added focused coverage for session create, agent spawn, and session end event ordering in the durable state journal.

### M8 durable boot restore update

- Kernel bootstrap now replays the durable state journal for the lifecycle events currently emitted by the app: `session.created`, `agent.created`, and `session.ended`.
- Restored sessions and agents are inserted without writing new durable events; ended-session replay clears live agents so ended sessions do not come back with runnable agents.
- Added focused restart coverage for created sessions, default agents, spawned agents, and ended sessions restored from the durable journal.

### M8 durable grant persistence update

- User-triggered MCP/skill grants and revokes now append durable agent mutation events with full agent snapshots.
- Agent-triggered capability requests through the runtime MCP tool path now append the same durable grant events, so discovered grants survive restart too.
- Boot restore replays those grant mutation events by restoring the latest agent snapshot, and focused coverage verifies a skill grant survives kernel restart.

### M8 durable workflow state update

- Workflow runtime commands that return a changed session now append a durable `session.updated` snapshot.
- Boot restore replays `session.updated` snapshots so workflow definitions, nodes, endpoints, run state, queues, and watchdog/session workflow fields can be recovered from the latest session snapshot.
- Added focused restart coverage that creates a workflow, adds a node, restarts the kernel, and verifies the workflow definition is restored.

### M8 durable provider profile update

- Provider launch success now appends a durable `agent.runtime_profile_updated` snapshot after updating an agent's provider/model/effort/resume state.
- Boot restore replays provider profile updates as agent snapshots so provider resume descriptors survive kernel restart.
- Added focused restart coverage using the `dev-stub` provider to verify provider/model profile restoration after a kernel restart.

### M8 runtime lifecycle durability update

- Runtime-state session end now appends `session.ended`, closing the CLI-facing lifecycle path in addition to the app service path.
- Runtime-state session deletion now appends `session.deleted`, and boot restore replays it by removing the session and clearing any live agents/projections.
- Added focused restart coverage for runtime end/delete paths to verify ended sessions stay ended and deleted sessions stay absent after reboot.

### M8 background lifecycle invariant update

- Documented the rule that retention, archive export, snapshots, Git observation, and remote reconciliation must not hold the main app lock while doing filesystem, network, Git, archive-adapter, or long-running SQLite work.
- Moved runtime-state durable event appends off the main `DaemonApp` lock by passing the durable state store into the owned runtime-state facade.
- Foreground commands still write small durable/operational/outbox rows synchronously, but no longer need the app lock just to append M8 durable agent/session mutation events.

### M8 durable snapshot restore update

- Boot restore now loads the latest durable state snapshot, restores sessions/agents from it, and replays only events after the snapshot sequence.
- Added an app-level snapshot writer that captures current sessions and agents against the latest durable event sequence; the future scheduler can call this from a background worker.
- Added focused coverage proving bootstrap restores snapshot state and then replays post-snapshot lifecycle events.

### M8 durable snapshot scheduler update

- Added a background durable snapshot scheduler for websocket and local IPC daemon lifetimes, controlled by `[state].snapshot_interval_events`.
- The scheduler reads event/snapshot sequence checkpoints, captures sessions and agents through cloneable stores under short store locks, then writes the SQLite snapshot outside the main `DaemonApp` lock.
- Snapshot write failures are logged and retried on later ticks without failing foreground kernel commands.
- Added focused coverage for interval gating, checkpointed snapshot writes, and no-main-app-lock snapshot ticks.

### M8 boot recovery reconciliation update

- Added boot-time reconciliation after durable snapshot/event replay so runtime-only work that cannot survive a kernel process restart is not shown as still running.
- Restored sessions now clear stale active provider run ids, interrupt active prompts, and mark in-flight workflow runs stopped with a `RunStopped` failure event explaining the kernel restart interruption.
- Fixed the restore/projection loops to materialize session lists before write-back, avoiding session read-lock retention across reconciliation writes.
- Added focused restart coverage that snapshots stale active provider, prompt, and workflow state, then verifies rebooted state is idle/stopped with the interruption surfaced on the workflow run.

### M8 local restart drill update

- Added `apps/cli/scripts/live-local-restart-drill.mjs` and `pnpm --filter @arroba/cli run local-restart:drill`.
- The drill rebuilds the kernel, runs an isolated daemon/home/config/state/history/workspace, creates durable session, spawned-agent, MCP-grant, skill-grant, provider-profile, completed-history, and active-workflow state, restarts the daemon, then verifies restored state and boot reconciliation end to end.
- Verified the live drill passes with stale active provider/prompt state cleared, the interrupted workflow run marked `Stopped` with the kernel-restart failure event, and the completed prompt marker available through both transcript paging and operational history search.

### M8 CLI restart/reconnect drill update

- Added `apps/cli/scripts/live-cli-kernel-restart-drill.mjs` and `pnpm --filter @arroba/cli run cli-restart:drill`.
- Extended CLI automation snapshots with `daemonDisconnected`, `statusLine`, and `sessionId` checks so PTY-hosted drills can assert restart UX instead of scraping terminal pixels.
- Fixed the runtime actor create-session path to append `session.created`; CLI-created sessions now restore after a kernel process restart instead of only sessions created through the app service path.
- Added CLI recovery for full kernel restarts: on transport close, an attached CLI polls for the previous session, creates a fresh attachment, resubscribes to kernel events, refreshes panes, clears the disconnected state, and keeps the restored agent visible.
- Hardened `session_unavailable` handling during reconnect by verifying the session state before transitioning to no-session, avoiding stale replay events from hiding a successfully restored session.
- Verified the live drill passes with a real PTY-hosted CLI: the CLI shows `Lost connection to the Arroba kernel.` after daemon stop, then reconnects after daemon restart and returns to the restored session with its agent present.

### M8 remote restart/reconcile drill update

- Added `apps/cli/scripts/live-remote-restart-drill.mjs` and `pnpm --filter @arroba/cli run remote-restart:drill`.
- Remote agent spawn now writes durable home-side agent snapshots, so home can restore remote agents after a kernel reboot.
- Remote prompt dispatch detects stale or missing worker leases, refreshes the remote agent binding through the live worker kernel, and retries the prompt submit. Relay daemon registration now replaces stale peers for the same daemon id, so worker restarts do not leave home routing requests to an old dead peer handle.
- Remote prompt completion now treats worker-side `NoActivePrompt` as an already-settled leased prompt and completes the home-side prompt, matching the dev-stub/fast-provider case where the worker finishes before home asks for completion.
- Verified the live drill passes through baseline prompt, home restart with worker alive, worker restart with stale lease refresh, and both home/worker restart with a final refreshed lease.

### M8 local Git observation update

- Added local provider-turn Git observation for structured prompt dispatch: Arroba captures a pre-turn Git snapshot from the provider working directory, captures a post-turn snapshot after prompt completion, and records operational history events without holding the main app lock during Git commands.
- Operational history now records `git_commit_detected`, worktree dirty/clean/change, and push-detected events with provider, model, prompt id, branch, worktree, prompt summary, commit SHA, commit subject, author metadata, changed paths, before/after HEAD, and attribution candidates.
- Added `apps/cli/scripts/live-git-observation-drill.mjs` and `pnpm --filter @arroba/cli run git-observation:drill`; the live drill passed against an isolated local kernel/dev-stub agent by committing `feature.txt` during a dispatched agent turn and verifying the commit event is searchable by subject/path/provider/model/prompt attribution.
- Remote Git observation follows the same event model with worker-local observation and home-owned persistence. The relay protocol now carries home prompt context to the worker and can return Git observations on leased prompt completion so home operational history gets the searchable commit event with home agent/prompt ids and worker machine/repo/worktree metadata.
- Added `apps/cli/scripts/live-remote-git-observation-drill.mjs` and `pnpm --filter @arroba/cli run remote-git-observation:drill`; the live drill passed against isolated relay/home/worker kernels by committing `feature.txt` in the worker repo during a remote dev-stub turn and verifying home history contains the commit with worker `repo_root`/`worktree_path`.

### M8 Postgres archive adapter drill update

- Added `arroba-history-archive-flush [--limit N]`, an ops binary that loads Arroba config, opens operational SQLite, flushes pending archive outbox events through the configured archive adapter, and prints attempted/accepted/rejected ids as JSON.
- Added `apps/cli/scripts/live-postgres-archive-adapter-drill.mjs` and `pnpm --filter @arroba/cli run postgres-archive:drill`.
- The drill runs a real ephemeral `postgres:16-alpine` container behind an HTTP adapter, creates transcript events through an isolated dev-stub kernel, and validates bearer-token auth, capabilities, append idempotency, HTTP failure retry, durable partial-rejection safety, non-durable rejected-event checkpointing, final retry acceptance, operational-only search when external archive search is disabled, and Postgres-backed archive search through Arroba once the matching operational row is deleted.

### M8 artifact archive store update

- Added a filesystem-backed operational artifact store with a SQLite artifact index/outbox. Transferred files are now registered as content-addressed SHA-256 blobs, emit an `artifact_stored` canonical history event, and can be flushed to external archive storage.
- Extended the archive adapter protocol with `PUT /arroba/artifacts/blobs/:artifact_id` for raw blob upload and `POST /arroba/artifacts/manifest` for durable artifact metadata acceptance.
- Extended `arroba-history-archive-flush` to flush pending artifact blobs/manifests before history events. The JSON output now has separate `artifacts` and `history` sections.
- Extended the Postgres archive drill to use production-shaped archive storage: Postgres stores events/artifact manifests/searchable metadata and MinIO stores S3-compatible artifact blobs. Verified `pnpm --filter @arroba/cli run postgres-archive:drill` passes with one archived transferred artifact and four archived transcript events.

### M9 workflow publication kickoff

- Added `docs/M9_WORKFLOW_PUBLICATION_PLAN.md`, defining publication gateways as transport/auth/parser/control infrastructure that forwards workflow-produced outputs without semantic manipulation.
- Reworked `apps/server` into the first `arroba-workflow-gateway` app. It loads a publication config, exposes HTTP routes, supports anonymous/bearer/API-key auth, built-in JSON/query/header/webhook/regex/path-template parsers, custom command parsers, lightweight input schema checks, sync/async kernel invocation, and passthrough `http_response` workflow output forwarding.
- Extended the M9 plan with paired-sender auth for published workflows and OpenClaw-derived connector/security notes. Pairing is documented as a short-lived bootstrap code that creates kernel-owned trusted sender records, while connector-specific webhook verification and sender policy stay beside each connector.
- Moved Slack, Discord, Telegram, WhatsApp, and Signal connectors into M9 V1 scope and clarified the single Arroba identity model: connector verification proves an external identity claim, then Arroba maps that identity to a registered user/team, paired sender, API token owner, or explicit anonymous caller.
- Added the first workflow-gateway Arroba auth resolver. Publication configs can now use `auth.mode="arroba"` with bearer/API-key principal mappings, external connector identity mappings, per-principal connector restrictions, and explicit anonymous access. Gateway raw-body parsing now supports Slack-style HMAC verification. Gateway tests cover connector-to-principal mapping, disallowed connector rejection, and anonymous opt-in behavior.
- Added gateway coverage for health, auth rejection/acceptance, JSON parsing/schema checks, transport-shaped output passthrough, regex parsing, and path-template parsing.
- Added M9.1/M9.9 kernel-owned workflow publication records in session state with local API variants for create/list/get/disable, TypeScript kernel-client helpers/types, and `workflow publication ...` shell command routing for `arroba-shell` and the CLI shell pane. Added focused session-service coverage for create/list/resolve/disable.
- Added M9.4 gateway kernel lookup by `ARROBA_PUBLICATION_SESSION_ID` plus `ARROBA_PUBLICATION_ID`, preserving file and explicit env fallback modes. Added M9.6 `publication:drill`, which starts an isolated kernel, creates a kernel-owned HTTP workflow publication, starts the gateway from that publication id, and invokes it through HTTP.
- Added M9.5 paired-sender auth. Pairing is optional per publication through `auth.mode="arroba"` plus `paired_senders.enabled=true`; publications without it do not expose the pair endpoint. Kernel session state now owns pairing codes and trusted sender records, shell/client APIs can create/redeem/list/revoke senders, and the gateway redeems `POST /.well-known/arroba/publication/pair`, authenticates sender bearer credentials, and forwards sender identity in invocation metadata. The publication live drill now validates anonymous publication and paired publication reject/redeem/invoke/revoke/reject behavior.
- Added M9.9 publication package export. `workflow publication export <publication> <directory> [--kernel-url <url>]` writes `publication.config.json`, `.env.example`, `run.sh`, and `README.md` for `arroba-workflow-gateway`, preserving auth/parser/method/mode config and paired-sender instructions. The publication live drill now exports a publication package and starts the gateway from the exported config before running paired-sender checks.
- Added M9.10 workflow-to-workflow publication drill. `pnpm --filter @arroba/cli run workflow-to-workflow-publication:drill` starts two isolated kernels and gateways, publishes workflow B over HTTP, publishes workflow A over HTTP with a custom parser that calls workflow B's published endpoint, and verifies both workflows return accepted async run metadata.
- Added M9.6 HTTPS/TLS support for the HTTP publication gateway. TLS can be supplied through publication config or `ARROBA_PUBLICATION_TLS_*` env vars, exported gateway packages document the HTTPS env shape, parser/schema failures now return HTTP 400, and the publication live drill covers HTTP success, parser failure, and self-signed HTTPS invocation.
- Added M9.12 Slack connector coverage. The gateway now handles signed Slack URL verification without workflow invocation, accepts signed slash-command form payloads through the Arroba connector identity model, and the publication live drill creates a Slack-shaped publication covering challenge and signed invocation.
- Added M9.13 Telegram connector coverage. Telegram webhook-secret verification now has unit and live drill coverage, sender ids map through the Arroba connector identity model, and the publication live drill covers invalid-secret rejection plus accepted Telegram webhook invocation.
- Added M9.14 Discord connector coverage. The gateway verifies Discord Ed25519 signatures over `timestamp + raw_body`, handles signed PING interactions without invoking workflows, maps signed interaction users through the Arroba connector identity model, and the publication live drill covers PING, invalid signature rejection, and accepted signed invocation.
- Added M9.15 WhatsApp and M9.16 Signal connector coverage. WhatsApp now supports Meta webhook verification plus `x-hub-signature-256` raw-body HMAC checks; Signal supports bridge-style `x-signal-webhook-secret` verification. Both connectors map sender identities through the Arroba connector identity model and are covered by unit tests plus the publication live drill.
- Added M9.7 WebSocket/WSS publication support. The gateway now accepts WebSocket upgrades at `/.well-known/arroba/publication/ws`, reuses publication auth and input schema validation, invokes the same kernel workflow endpoint, streams accepted/status/final/error messages where possible, and supports WSS through the existing TLS config. Unit tests cover WS invocation and validation errors; the publication live drill covers WS and self-signed WSS.
- Added M9.8 local IPC publication invocation. `arroba-workflow-call` can invoke exported `publication.config.json` packages or kernel-owned publications by session/publication id, validates the configured input schema, forwards to the same kernel workflow endpoint as HTTP/WebSocket, emits JSON results, and is covered by unit tests plus the publication live drill.
- Added a Docker-backed M9 publication connector drill. `pnpm --filter @arroba/cli run publication:docker-connectors-drill` builds a checked-in Node/curl client image and verifies an external container can kick off workflow runs through HTTP, HTTPS, WS, WSS, Slack, Telegram, Discord, WhatsApp, and Signal ingress paths.
- Added a semantic URL renderer drill. `pnpm --filter @arroba/cli run semantic-url-renderer:drill` drives session/workflow/publication setup through shell commands, uses one Codex `gpt-5.4` workflow agent, publishes an async renderer workflow, and validates that a wrapper site first serves a loading page and then serves workflow-generated HTML for `/about/<prompt>`.

### iOS app implementation kickoff

- Added the native SwiftUI iOS app under `apps/ios` as an OSS Arroba client surface, with a minimal Xcode app shell and `ArrobaPackage` for feature code.
- Implemented the first local-kernel client slice: typed request/response envelopes, `ListSessions`, `CreateSession`, `AttachToSession`, and a `URLSessionWebSocketTask` request path matching the TypeScript kernel-client IPC frame shape.
- Added an `@Observable` app model and a terminal-inspired waiting-room UI with kernel URL/workspace/worktree configuration, session refresh, session creation, selected-session summary, runtime drawer, and global footer.
- Added Swift Testing coverage for protocol encoding/decoding and model session selection, plus XCUITest launch coverage for the waiting-room UI.
- Documented iOS local development, build/test commands, component parity rules, and QA guidance in `apps/ios/README.md`; updated `docs/ios/IOS_APP_PLAN.md` with the first implementation decisions and remaining IOS-M1 transport work.

### iOS kernel event stream update

- Extended the Swift kernel protocol layer with local WebSocket subscribe/unsubscribe frames, transport event frame decoding, and typed handling for `session_snapshot`, `heartbeat`, `session_unavailable`, `transport_resumed`, and `replay_gap` events.
- Added attach-and-subscribe behavior to the waiting-room app model, including a stable iOS client id, attachment state, event cursor tracking, heartbeat timestamp, session snapshot upserts, replay-gap surfacing, and reconnect with the last received event id after stream interruptions.
- Added Attach and Detach actions plus attachment/stream status to the SwiftUI runtime drawer while keeping the app as a kernel client rather than a runtime authority.
- Expanded Swift Testing coverage to 8 tests and XCUITest launch coverage to include the Attach/Detach affordances; verified `swift test --package-path apps/ios/ArrobaPackage` and `xcodebuild -workspace apps/ios/Arroba.xcworkspace -scheme Arroba -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' test` pass.

### iOS prompt composer update

- Added the first single-agent prompt composer to the iOS waiting room. The composer requires an active attachment, sends `SubmitPrompt` to the selected session/focused agent, clears the draft only after kernel acceptance, and exposes a Stop action backed by `CancelActivePrompt`.
- Extended Swift protocol fixtures and app model tests for prompt submission, bringing package coverage to 10 tests. The XCUITest launch smoke now asserts the prompt composer, Send, and Stop affordances exist.
- Re-verified `swift test --package-path apps/ios/ArrobaPackage` and `xcodebuild -workspace apps/ios/Arroba.xcworkspace -scheme Arroba -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' test` pass.

### iOS transcript and agent focus update

- Added recent session-history loading after attach, live transcript rendering for terminal output/runtime notices/completion markers, and replay-aware auto-scroll behavior in the SwiftUI waiting-room surface.
- Added kernel protocol coverage for `GetSessionHistory`, `FocusAgent`, `CycleAgentFocus`, `AgentFocused`, terminal output frames, and session-history responses.
- Added runtime-drawer agent focus controls so iOS can focus a specific kernel-reported agent or cycle focus, matching the TypeScript client prompt-routing model at the request boundary.
- Verified `swift test --package-path apps/ios/ArrobaPackage` passes with 17 Swift Testing cases and `xcodebuild -workspace apps/ios/Arroba.xcworkspace -scheme Arroba -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' build` succeeds.

### iOS command-center subset update

- Added the first native command-center catalog for iOS with `/session`, `/agent`, `/stop`, and `/waiting` discovery from the prompt composer.
- Routed slash-command drafts through the existing `ArrobaAppModel` actions so `/session list`, `/session new|create`, `/session attach [ref]`, `/session detach`, `/agent list`, `/agent focus <ref>`, `/agent cycle`, `/stop`, and `/waiting` use the same typed kernel-backed request paths as the buttons and agent controls.
- Added command feedback into the transcript as notices and kept failed freeform prompt submission behavior intact.
- Expanded Swift Testing coverage to 20 cases for command catalog filtering plus slash command execution; verified `swift test --package-path apps/ios/ArrobaPackage` and `xcodebuild -workspace apps/ios/Arroba.xcworkspace -scheme Arroba -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' test` pass. A simulator text-entry command-center assertion was intentionally not kept because XCTest did not reliably expose the transient SwiftUI suggestion buttons; package/model coverage is the stable gate for that logic for now.

### M14B home-managed slice native TUI update

- Added `--slice <ref>` native TUI placement for Codex, OpenCode, and Claude so `arroba codex/opencode/claude <session> --slice <slice_id>` attaches to a home-kernel session and places provider execution on the home-managed slice worker.
- Local Docker slices now reuse the home relay when available, start only the worker runtime, bind provider endpoints for worker access, import provider auth on demand, install Claude Code, copy Claude credentials from file or macOS Keychain, and trust `/workspace` for Claude Code.
- Fixed slice worker kernel ref resolution so shared-relay home-managed slices keep the recorded home relay endpoint/token instead of being rewritten to the old slice-private Docker relay.
- Extended `live-remote-native-tui-drill.mjs` with `--home-managed-slice-local-docker` and validated Codex, OpenCode, and Claude with two provider-native TUIs plus one Arroba observer in one home session, provider execution on the slice worker, prompt/turns, provider permissions, prompt attachments, cross-agent separation, and badge transitions back to idle.

### M14B native TUI MCP/skills contract update

- Documented the native TUI MCP/skills placement contract: local native TUI reuses agent-scoped grants, standard home-worker remote does not copy/install capabilities across machines, and home-managed slices may receive home skill packages because the home kernel owns the child worker.
- Started native remote MCP propagation for provider-native runs by forwarding grant-derived MCP requirements through `LaunchLeasedNativeProviderRun`, validating worker availability before remote native launch, and rendering the required MCP set into the worker-owned provider run.
- Extended Claude native TUI launch planning so granted MCPs and the Arroba runtime MCP are passed through `--mcp-config`/`--strict-mcp-config`, matching the structured Claude launch path.
