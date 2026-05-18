# Cross-Repo Architecture Boundary Refactor Plan

## Scope

Refactor `arroba` and `arroba-cloud` together while preserving runtime compatibility.

- `arroba`: kernel, relay, CLI/shell clients, provider adapters, shared protocol/client code.
- `arroba-cloud`: hosted auth/control plane, relay token issuance, browser bootstrap, waiting room, browser terminal UI.
- Excluded: iOS. It should follow the stabilized protocol and boundaries after this refactor.

Cloud is auth/control-plane/bootstrap only. The kernel owns runtime sessions, agents, provider runs, workspaces, history, workflows, terminal events, and state transitions. The relay remains opaque transport.

## Current Checkpoint

2026-05-15:

- Cloud API boundary split is in place: `server.ts` is route composition; `cloud-api-service.ts` delegates to focused use cases; dependency construction lives in `cloud-api-service-dependencies.ts`; route/helper/contract files are domain-owned.
- Cloud web responsibility modules now cover browser kernel transport, waiting-room state/projection/rendering/cache persistence/connection display/refresh scheduling/refresh controller/kernel event policy/kernel-directory refresh, app sidebar/context/resize/workflow/history/prompt/output/workspace/capabilities, terminal lifecycle/transport/target/launch/provider profile, freeform policy, and session projection fingerprinting. `client.ts` is still the main coordinator and is 9,893 lines.
- OSS runtime now has responsibility-owned boundaries for router composition, runtime state views, prompt/provider/workflow/session/capability ownership, managed I/O, projection policy, relay/cloud bridges, and session actors. `runtime/capability_executor.rs` is 295 lines after moving context, health/admission, and transferred-file artifact recording into submodules.
- Remote managed-I/O dispatch now separates composition, outbound leased-worker forwarding, home-kernel admission/routing, forwarded reads, forwarded text edit/write, and forwarded patch/delete/move mutations. History requests now separate session transcript, prompt-input history, archive query/search, and semantic search mapping. Workspace repo files now separate listing projection, file content loading, and shared timing.
- Latest verified batch: BrowserKernelClient transport, browser kernel request builders, history kernel bridge, and CLI Cloud command/worktree placement policies are responsibility-owned modules; Cloud web and CLI tests pass.
- Latest verified batch: CLI Cloud session collaboration command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI relay-cloud profile command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI theme file-source discovery is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI theme color utilities are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI theme contracts and definition parsing are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI built-in theme catalog is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider auth/process command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI model/variant/view selection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI user config command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI remote machine command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI MCP/skill capability command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI slice command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workspace/worktree command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent command handling and focus cycling are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command coordinator, automation flow, overlays, interaction strips, status badges, and prompt chrome rendering are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI split-pane footer rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI status indicator chrome rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt/footer chrome summary rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workspace shell submission is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent activity/busy latch and focused busy derivation policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal record agent routing policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment pending-state policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider-native command submission policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow endpoint prompt admission is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt submission projection/status policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI interaction choice selection/reply policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI focused interaction keyboard policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt history and prompt-turn navigation policies are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room key navigation/lifecycle policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser key policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser list/index policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-browser controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI dialog overlay state priority is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment UI state controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment intake/controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI clipboard selection controller is responsibility-owned; CLI tests pass.
- Latest verified batch: Cloud API route-adapter guardrail prevents direct service/repository imports; API build/tests pass.
- Latest verified batch: Cloud API relay facade owns token, target, browser bootstrap, and kernel presence wiring; API build/tests pass.
- Latest verified batch: Cloud API pairing facade owns pairing, revocation, and machine runtime profile wiring; API build/tests pass.
- Latest verified batch: Cloud API browser session facade owns browser session and device-login wiring; API build/tests pass.
- Latest verified batch: Cloud API session-invite facade owns collaboration invite wiring; API build/tests pass.
- Latest verified batch: Cloud API service is thin composition; account, admin, billing, dashboard, and managed-history wiring live in domain facades; API build/tests pass.
- Latest verified batch: Cloud API service composition guardrail prevents direct domain use-case imports/regrowth; API build/tests pass.
- Latest verified batch: Cloud API repository contract is a compatibility aggregate over domain repository interfaces; API build/tests pass.
- Latest verified batch: Cloud API Prisma repository factory composes focused domain facets with guardrails; API build/tests pass.
- Latest verified batch: Cloud API service contract is a compatibility aggregate over domain service interfaces; API build/tests pass.
- Latest verified batch: Tool-display public types, language inference, and transcript-update utilities are responsibility-owned; tool-display tests and CLI tests pass.
- Latest verified batch: Tool-display patch/managed-I/O parsing and shared string/status/tool-name helpers are responsibility-owned; tool-display tests and CLI tests pass.
- Latest verified batch: Tool-display read/grep rendering and shared code-block projection are responsibility-owned; tool-display tests and CLI tests pass.
- Latest verified batch: OSS workflow gateway contracts and kernel publication IPC are responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway auth and connector verification are responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway request parsing and schema validation are responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway publication config loading and TLS/env policy are responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway WebSocket invocation transport is responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway HTTP response projection is responsibility-owned; server tests pass.
- Latest verified batch: OSS workflow gateway raw-body parser registration is responsibility-owned; server tests pass.
- Latest verified batch: CLI command-center selection/submission policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI dialog overlay focus capture/restore policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt content-change/drop policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt draft persistence scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI turn-completion scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal output record batching is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI shared prompt input history refresh scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI footer flash timing is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session chrome update scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI response-pane focus repaint scheduling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI connection health watchdog is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prepended-history scroll restoration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent focus transition tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: Cloud API route schemas are domain-owned with a generic-helper guardrail; API build/tests pass.
- Latest verified batch: CLI older-transcript history loading is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history hydration stale-result guard is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript-history auto-load triggers are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript render deferral is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt cancellation in-flight handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI ambient interval ownership is split by transcript, working-animation, and waiting-room refresh responsibility; CLI tests pass.
- Latest verified batch: CLI prompt focus retention and prompt-surface mouse handling are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workspace shell context sync is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow selection sync moved to its owned module; CLI tests pass.
- Latest verified batch: CLI workflow screen/canvas controls are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow session-state refresh is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow topology endpoint/node/edge requests are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow runtime invoke/queue/run requests are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow watchdog requests are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow settings requests are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow definition metadata requests are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history attachment hydration lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow node instructions editor lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider recovery relaunch/reapply flow is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel event subscription scope tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel restart reattachment/backoff recovery is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI attached-kernel resync/catch-up flow is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI exit cleanup and force-quit retry handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room transition cleanup is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI terminal restore/process-exit teardown is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt input history append/record/sequence tracking is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI submitted-prompt UI reset/restore is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history keyboard navigation is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt content-change/drop side effects are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt attachment token highlighting is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt text snapshot/mutation state is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center state and keyboard controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room control activation policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room lifecycle confirmation state is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider/model/variant selection controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room inventory refresh controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI dialog overlay lifecycle controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt slash-command submission controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow endpoint prompt submission controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider-native namespace submission controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI normal prompt submission controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI focused interaction choice controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room lifecycle action controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt submission coordinator is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room key controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-turn navigation controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI global keyboard shortcut controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI stdin key routing controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt keydown routing controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI poller degradation controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI automation server lifecycle controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI background poller startup controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room intro animation controller is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow terminal panel opening is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow inspector projection and agent label formatting are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command agent reference resolution is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt surface projection and placeholder sync are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow queue command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow run lifecycle command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow terminal command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow run/start invocation handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow settings commands are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow endpoint commands are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow edge commands and shorthand are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow node instructions commands are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow watchdog commands are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow node commands and node runtime settings are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow lifecycle/list/show/new/alias commands are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI workflow reference resolution policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room terminal row rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room remote inventory row rendering and remote delete/attach policy are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room session row and menu projection are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room slice selection policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room focus target navigation is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room start/config row projection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room choice/model projection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room focused value cycling policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room state creation/normalization is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room row composition is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room intro art projection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room shared types are responsibility-owned with compatibility exports; CLI tests pass.
- Latest verified batch: CLI waiting-room value-cycling wrapper is owned by the value-cycling policy module; CLI tests pass.
- Latest verified batch: CLI command-center item type and search/ranking policy are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center tree traversal/projection is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center static slash command tree is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center dynamic provider/model/view item builders are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center root projection and scoped matching are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center selection and exact-submit policy are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent substitute command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent spawn command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent alias/config/profile command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent lifecycle/focus command handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript render color policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript render-mode classification is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI apply-patch transcript rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript text/span rendering is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript interaction controls are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript retention/cleanup is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI launch preference defaults and agent location labels are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI primary transcript state mutations are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI streamed transcript ingestion is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI response pane grid layout application is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript user/notice/error event appends are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript turn expansion state is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI auxiliary agent-pane transcript interactions are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent-pane streamed transcript ingestion is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent-pane direct transcript entries/previews are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI auxiliary agent-pane transcript render lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI primary transcript render lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prepended transcript history stitching is owned by transcript history; CLI tests pass.
- Latest verified batch: CLI primary transcript entry replace/prepend lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI deferred bootstrap hydration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI attached-session initial history/prompt hydration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI authoritative-idle local reset policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI assistant-message completion handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI provider activity and terminal-output side-effect adapters are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI prompt-history restore policy is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript viewport scroll-to-bottom behavior is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI runtime session snapshot application is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI hotkey-toggle shortcut handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room activation side effects are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room reconciliation side effects are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI loading and hydration side effects are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI detached-kernel waiting-room connect flow is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI agent-pane history refresh orchestration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI command-center slash-command execution is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI fallback polling loops are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel session snapshot application is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI kernel event dispatch routing is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session-unavailable recovery is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session projection and agent-pane refresh decision policy are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI launch bootstrap orchestration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI response-pane layout orchestration is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI session chrome render application is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI runtime debug logging is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI UI batch depth and deferred-flush ownership is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript parser registration idempotency is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI runtime tuning constants and prompt queue type ownership are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI process logging and error formatting are responsibility-owned; CLI tests pass.
- Latest verified batch: Cloud API browser-auth route option wiring is responsibility-owned; API build/tests pass.
- Latest verified batch: CLI hotkey debug reporting is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI transcript history autoload follow-up scheduling is controller-owned; CLI tests pass.
- Latest verified batch: Cloud API Node process entrypoint is separated from Fastify server composition; API build/tests pass.
- Latest verified batch: CLI provider/prompt projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI response-pane projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt chrome projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI process/stdin lifecycle wiring is controller-owned; CLI tests pass.
- Latest verified batch: CLI agent runtime projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI agent-pane entry/preview store is controller-owned; CLI tests pass.
- Latest verified batch: CLI automation snapshot projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI daemon activity recovery state is controller-owned; CLI tests pass.
- Latest verified batch: CLI command-center/session-browser projections are controller-owned; CLI tests pass.
- Latest verified batch: CLI terminal resize policy is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt session history refresh/draft policy is controller-owned; CLI tests pass.
- Latest verified batch: CLI status indicator badge/logging policy is controller-owned; CLI tests pass.
- Latest verified batch: CLI interaction choice mutable state is store-controller-owned; CLI tests pass.
- Latest verified batch: CLI agent interaction strip render wiring is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt meta render wiring is controller-owned; CLI tests pass.
- Latest verified batch: CLI history loading render wiring is controller-owned; CLI tests pass.
- Latest verified batch: CLI response-pane render scheduling is controller-owned; CLI tests pass.
- Latest verified batch: CLI visible activity label sync is controller-owned; CLI tests pass.
- Latest verified batch: CLI split-pane footer render projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI primary transcript runtime store is controller-owned; CLI tests pass.
- Latest verified batch: CLI auxiliary agent-pane runtime store is controller-owned; CLI tests pass.
- Latest verified batch: CLI mounted primary transcript agent state moved into the transcript runtime store; CLI tests pass.
- Latest verified batch: CLI transcript turn id progression is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt submission target-agent state is controller-owned; CLI tests pass.
- Latest verified batch: CLI process closing state is controller-owned; CLI tests pass.
- Latest verified batch: CLI waiting-room hidden-kernel preference state is controller-owned; CLI tests pass.
- Latest verified batch: CLI transcript syntax-style lifecycle is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt-meta render refs are owned by the prompt-meta render controller; CLI tests pass.
- Latest verified batch: CLI status-indicator render ref is owned by the status indicator controller; CLI tests pass.
- Latest verified batch: CLI session chrome render refs are owned by the session chrome controller; CLI tests pass.
- Latest verified batch: CLI history-loading render refs are owned by the history loading controller; CLI tests pass.
- Latest verified batch: CLI command-center overlay ref is owned by the command center controller; CLI tests pass.
- Latest verified batch: CLI dialog overlay ref is owned by the dialog overlay controller; CLI tests pass.
- Latest verified batch: CLI response-pane render refs are owned by a response-pane ref store; CLI tests pass.
- Latest verified batch: CLI prompt input ref is owned by a prompt input ref controller; CLI tests pass.
- Latest verified batch: CLI transcript scrollbox ref is owned by a transcript scrollbox ref controller; CLI tests pass.
- Latest verified batch: CLI agent-pane streaming commit policy is owned by a streaming commit controller; CLI tests pass.
- Latest verified batch: CLI agent-pane live transcript retention is owned by a retention controller; CLI tests pass.
- Latest verified batch: CLI prompt session draft/history persistence is owned by a persistence controller; CLI tests pass.
- Latest verified batch: CLI SIGINT stop/exit policy is owned by the global keyboard shortcut controller; CLI tests pass.
- Latest verified batch: Cloud active README no longer names the deleted WEB_CLI bridge; active Cloud docs scan passes outside archive/web work.
- Latest verified batch: CLI agent-pane runtime reset is owned by a reset controller; CLI tests pass.
- Latest verified batch: CLI agent-pane session-change refresh decisions are owned by the agent-pane refresh controller; CLI tests pass.
- Latest verified batch: CLI focused interaction and workflow-inspector projections are controller-owned; CLI tests pass.
- Latest verified batch: CLI renderer focus/debug projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI workspace-shell submission dependency wiring is controller-owned; CLI tests pass.
- Latest verified batch: Cloud API server composition imports domain dependencies directly instead of the public barrel; API build/tests pass.
- Latest verified batch: CLI focused status badge projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI transcript entry visibility/renderability projection is controller-owned; CLI tests pass.
- Latest verified batch: CLI prompt-meta ref callbacks and chrome update hook are controller-owned; CLI tests pass.
- Latest verified batch: Cloud compatibility HTTP adapter delegates relay routes to a domain adapter handler; API build/tests pass.
- Latest verified batch: Cloud compatibility HTTP adapter delegates device-login and session-invite routes to domain handlers; API build/tests pass.
- Latest verified batch: Cloud compatibility HTTP adapter now composes account, managed-history, pairing, relay, auth, and invite handlers; API build/tests pass.
- Latest verified batch: Cloud compatibility HTTP adapter composition and handler-scope guardrails are in place; API build/tests pass.
- Latest verified batch: Cloud active API modules no longer import through the public barrel; guardrail added; API build/tests pass.
- Latest verified batch: Cloud account-control persistence is split from admin persistence; API build/tests pass.
- Latest verified batch: Cloud admin content counting/deactivation/audit metadata is split from admin repository orchestration; API build/tests pass.
- Latest verified batch: Cloud admin query include/projection/search-limit policy is split from admin repository orchestration; API build/tests pass.
- Latest verified batch: Cloud admin read/search persistence is split from admin mutation persistence; API build/tests pass.
- Latest verified batch: Cloud shared-session collaborator listing/contact persistence is split from invite lifecycle persistence; API build/tests pass.
- Latest verified batch: Cloud shared-session invite state policy is split from lifecycle orchestration; API build/tests pass.
- Latest verified batch: Cloud shared-session member listing/projection is split from invite lifecycle persistence; API build/tests pass.
- Latest verified batch: Cloud managed-history policy persistence is split from record/search/export persistence; API build/tests pass.
- Latest verified batch: Cloud managed-history record append, search, and export persistence are responsibility-owned modules; API build/tests pass.
- Latest verified batch: Cloud admin paired-identity revocation mechanics are split from admin lifecycle orchestration; API build/tests pass.
- Latest verified batch: Cloud machine runtime profile shaping/merge policy is split from profile persistence; API build/tests pass.
- Latest verified batch: Cloud approved device-login provisioning is split from authorization polling state; API build/tests pass.
- Latest verified batch: Cloud runtime credential validation is split from runtime token issuance and relay target writes; API build/tests pass.
- Latest verified batch: Cloud pairing token creation is split from relay client/machine pairing persistence; API build/tests pass.
- Latest verified batch: Cloud browser dashboard projection/freshness sorting is split from dashboard persistence; API build/tests pass.
- Latest verified batch: Cloud pairing token consumption state is split from relay identity persistence; API build/tests pass.
- Latest verified batch: Cloud account bootstrap persistence is split from repository composition, and browser dashboard reuses account resolution; API build/tests pass.
- Latest verified batch: CLI silent-poll threshold is owned by runtime policy instead of the app coordinator; CLI lint/build/tests pass.
- Latest verified batch: Cloud shared-session invite acceptance/membership persistence is split from invite creation/display/revocation; API build/tests pass.
- Latest verified batch: Cloud relay identity revocation persistence is split from active identity upsert/lookup helpers; API build/tests pass.
- Latest verified batch: Cloud repository composition now has a delegation-only guardrail; API build/tests pass.
- Latest verified batch: Cloud admin browser permission policy is split from general browser security helpers; API build/tests pass.
- Latest verified batch: Cloud dev browser/device auth cookie and secret mechanics are split from general browser security helpers; API build/tests pass.
- Latest verified batch: Cloud account/email normalization is split from account bootstrap persistence; API build/tests pass.
- Latest verified batch: Cloud account bootstrap now delegates identity upsert, relay realm creation, and subscription seeding; API build/tests pass.
- Latest verified batch: Cloud browser session creation and logout revocation are separate repository modules; API build/tests pass.
- Latest verified batch: Cloud browser security registration is split from browser identity/session helpers; API build/tests pass.
- Latest verified batch: Cloud web/admin shell serving is split by route owner with shared shell-auth and dist-path helpers; API build/tests pass.
- Latest verified batch: Cloud relay subscription, paired identity, and runtime-token admission policies are separate modules; API build/tests pass.
- Latest verified batch: Cloud admin content counts and audit metadata are split from deactivation maintenance; API build/tests pass.
- Latest verified batch: Cloud admin deactivate and purge mutations are separate repositories; API build/tests pass.
- Latest verified batch: Cloud relay identity upsert, paired lookup, hosted-account check, and subject-kind mapping are separate modules; API build/tests pass.
- Latest verified batch: Cloud pairing operational event shaping is split from pairing service orchestration; API build/tests pass.
- Latest verified batch: Cloud pairing service depends on a narrow repository contract and pairing persistence input module; API build/tests pass.
- Latest verified batch: Cloud relay services depend on narrow repository contracts and relay persistence input modules; API build/tests pass.
- Latest verified batch: Cloud device-login service depends on a narrow repository contract and device-login persistence input module; API build/tests pass.
- Latest verified batch: Cloud browser-session service depends on a narrow repository contract and browser-session persistence input module; API build/tests pass.
- Latest verified batch: Cloud session-invite service depends on a narrow repository contract and session-invite persistence input module; API build/tests pass.
- Latest verified batch: Cloud managed-history service depends on a narrow repository contract and managed-history persistence input module; API build/tests pass.
- Latest verified batch: Cloud billing, machine-runtime, and admin services use explicit narrow repository interfaces; API build/tests pass.
- Latest verified batch: Cloud account-control, browser-dashboard, and browser-relay-kernel services use explicit narrow repository interfaces; API build/tests pass.
- Latest verified batch: CLI native-TUI capability grants are shared across provider launchers; CLI tests pass.
- Latest verified batch: CLI native-TUI session/agent lifecycle is shared across provider launchers; CLI tests pass.
- Latest verified batch: CLI native-TUI provider-run launch/get IPC is shared across provider launchers; CLI tests pass.
- Latest verified batch: CLI Codex/OpenCode native launch environment helpers are shared; CLI tests pass.
- Latest verified batch: CLI Claude native attachment/context handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native hook-handler generation is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native permission bridge is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native TUI process launch is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native transcript reading and skill-context generation are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI OpenCode native prompt extraction and selection policy are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI OpenCode native permission-response handling is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI OpenCode native process launch is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI OpenCode native event-stream redaction and refresh injection are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex native app-server lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex native prompt and attachment extraction is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI native permission interaction resolution is shared and Codex choice parsing is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex kernel-output projection to native TUI JSON-RPC is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex JSON-RPC parsing and message classification are responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex native turn submission is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI native kernel pump loop is shared by Codex/OpenCode and responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native launch-environment parsing uses shared helpers; CLI tests pass.
- Latest verified batch: CLI Claude remote-rendered terminal I/O is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI native kernel pump supports Claude local TUI without a provider-local loop; CLI tests pass.
- Latest verified batch: CLI OpenCode native HTTP proxy/interception is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Codex native WebSocket proxy/thread binding is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native hook bridge is responsibility-owned; CLI tests pass.
- Latest verified batch: CLI Claude native provider-run lifecycle is responsibility-owned; CLI tests pass.
- Latest verified batch: OSS local provider catalog loading/auth helpers and relay blocking are responsibility-owned; focused kernel provider-request tests pass.
- Latest gates for owned files: kernel-client tests, Cloud API build/API tests, focused router test, file-level rustfmt, and scoped diff checks pass. Cloud web and full-kernel gates remain pending where they touch dirty unrelated slices.

## Responsibility Rule

Do not split large files by line range. A new module must own a named responsibility with a stable dependency direction.

Allowed:

- Move complete responsibilities such as command admission, session mutation, relay bootstrap, event replay, prompt lifecycle, browser session state, waiting-room projection, or external adapter I/O.
- Extract pure state/projection/request policy with tests before moving side effects.
- Keep compatibility barrels only to preserve imports during migration.

Disallowed:

- Bucket files such as `client-part-1.ts`, `router-helpers.rs`, or `server-utils.ts`.
- Moving private helpers without changing ownership.
- Modules that require broad unrelated state access.
- Mixing render, state mutation, network I/O, and policy in one new module.

## Protocol Rule

No protocol shape changes are intended. If `LocalDaemonRequest`, `LocalDaemonResponse`, relay terminal events, browser/kernel transport semantics, or serialized client protocol shapes change:

1. Bump the shared local daemon protocol version.
2. Update protocol snapshot/hash tests.
3. Update client minimum protocol versions only when needed.
4. Add a focused drill for the changed behavior.

## Workstreams

### OSS Runtime

- Keep shrinking `CommandRouter` into responsibility executors:
  - session lifecycle/membership/focus
  - cloud relay login/token/session invite calls
  - provider auth/catalog/process controls
  - workspace/git/worktree/file requests
  - relay status/remote inventory projections
  - semantic/agent utilities
- Add a kernel composition boundary that owns runtime state, stores, projections, actors, transport health, provider lanes, workspace coordination, and schedulers.
- Remove production `app.lock()` command-path dependencies outside bootstrap/composition. If a slice cannot remove one, document the blocker and avoid new call sites.
- Keep request/response behavior and wire shapes unchanged.

### Shared Protocol And Clients

- Keep `apps/kernel/src/local/api/types.rs` as the wire source of truth.
- Keep browser-safe request/event/protocol helpers separate from Node-only socket/crypto code in `packages/kernel-client`.
- CLI and shell remain clients, not runtime authorities.
- Split CLI composition/state/event handling after kernel-client and runtime boundaries are stable.

### Cloud API

- Keep `apps/api/src/server.ts` as Fastify composition only.
- Keep route modules domain-owned: browser relay bootstrap, browser session, relay, admin, billing, device login, pairing, managed history, account control, session invites.
- Keep `CloudApiService.bootstrapBrowserRelayKernel(input)` as the browser bootstrap use case.
- Keep `contracts.ts` as a compatibility barrel over domain contract files until imports are migrated.
- Preserve `/browser/relay-kernel/bootstrap`, `/dashboard`, `/relay/token`, and browser terminal route compatibility.

### Cloud Web

- Turn `apps/web/src/client.ts` into app bootstrap/coordinator only: route mount, dependency wiring, global event registration, and app start.
- Continue extracting by responsibility:
  - terminal app/container wiring
  - waiting-room controller and background refresh orchestration
  - terminal session lifecycle/connect/reattach
  - freeform agent config/dialog controllers
  - prompt/history/workspace/capabilities controllers
  - render/projection modules or React mounts for HTML/view ownership
- Controllers should not build large HTML strings. Rendering belongs in render modules or React mount components.
- Waiting-room refresh must not overwrite active transcript, focus, prompt draft, selected session, or local reconnect state.

### Naming, Docs, Cleanup

- Active browser runtime storage keys use `arroba:terminal:*` with one-time legacy `arroba:web-cli:*` read fallback.
- Active drills use `terminal-*` or `browser-relay-kernel-*` names.
- Active docs/code/scripts should not reference live `/web-cli` routes except archived historical notes with explicit archive wording.
- Keep this plan concise. Add one checkpoint line per coherent verified batch, not one entry per tiny helper.
- Commit and push verified docs/code batches together; do not leave plan/docs dirty as assumed user work.

## Execution Order

1. Maintain architecture guardrails and line-budget checks for `client.ts`, `server.ts`, `waiting-room-kernel.ts`, and `runtime/router.rs`.
2. Finish Cloud web coordinator extraction around real responsibilities, starting with remaining terminal session/freeform/waiting-room orchestration.
3. Continue OSS router extraction around runtime responsibilities, then introduce the kernel composition boundary.
4. Split remaining CLI composition/state/controller code after shared protocol/runtime seams are stable.
5. Remove stale compatibility barrels/helpers once imports have moved.
6. Run cross-repo smoke/drill gates before resuming feature work.

## Test Gates

Per OSS slice:

- `cargo fmt --manifest-path apps/kernel/Cargo.toml`
- `cargo test --manifest-path apps/kernel/Cargo.toml --lib -- --test-threads=1`
- `pnpm --filter @arroba/kernel-client run test` when shared TypeScript client code changes
- `pnpm --filter @arroba/cli run test` or shell tests when client code changes

Per Cloud slice:

- `pnpm --filter @arroba-cloud/api test` when API changes
- `pnpm --filter @arroba-cloud/web test` when web changes
- `pnpm -r --if-present lint`
- `git diff --check`

Architecture/drill gates:

- browser relay kernel prompt flow
- stale relay target denial
- managed relay smoke
- local/remote freeform and workflow relay drills
- reconnect/replay-gap/session snapshot recovery
- waiting-room refresh preserves active terminal state
- staging retail strict smoke for deployment-sensitive changes

## Done Criteria

- Cloud has no runtime terminal proxy behavior and remains bootstrap/control plane only.
- Relay remains opaque transport.
- Kernel-owned runtime state is not forked in Cloud or clients.
- `client.ts`, `server.ts`, `waiting-room-kernel.ts`, and `runtime/router.rs` are coordinators/composition files rather than domain owners.
- Protocol shapes are unchanged, or the protocol rule has been followed.
