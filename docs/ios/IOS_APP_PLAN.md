# Chariox iOS App Plan

Status: IOS-M0/M1 implementation started on 2026-04-25.

## Goal

Build a native iOS client for Chariox inside the open-source `chariox` repository.

The iOS app is a client surface like the TypeScript terminal CLI and the Cloud
WEB_CLI. It must not become a runtime authority. The kernel remains responsible
for sessions, agents, workflows, provider runs, prompt queues, workspace live sync,
relay membership, and permission decisions.

## Source Context

Read before implementing:

- `README.md`
- `docs/spec-v1.md`
- `docs/ARCHITECTURE.md`
- `docs/PROTOCOL.md`
- `docs/RUNNING_LOCAL.md`
- `docs/CONTRIBUTING.md`
- `docs/ROADMAP.md`
- `apps/cli/src/`
- `packages/kernel-client/src/`
- sibling repo: `/Users/miguel/chariox-cloud/docs/WEB_SERVICE_ARCHITECTURE.md`
- sibling repo: `/Users/miguel/chariox-cloud/docs/C2_REMOTE_WEB_CLI_WORKBENCH_MILESTONE.md`
- sibling repo: `/Users/miguel/chariox-cloud/docs/C2_5_PERSISTENT_WEB_CLI_RUNTIME_MILESTONE.md`
- sibling repo: `/Users/miguel/chariox-cloud/docs/CLI_KERNEL_INTEGRATION_CONTRACT.md`

Cloud is a product and relay/onboarding reference, not the authority model for
the iOS app. The OSS iOS app should support direct local-kernel attachment and
optional hosted relay/cloud login.

## Product Rules

- iOS is part of the OSS `chariox` repo, not `chariox-cloud`.
- iOS must share behavior with terminal CLI and WEB_CLI.
- The kernel is the source of truth for runtime state.
- Client-local state is limited to view state, drafts, local preferences,
  selected addresses, cached projections, and secure credentials.
- All runtime-changing actions go through kernel or relay-backed kernel
  requests.
- Cloud owns hosted identity, relay credentials, and account state only.
- User-generated remote payloads must preserve the existing session-scoped
  encryption boundary when crossing relay infrastructure.

## Recommended Stack

### App

Use native SwiftUI in `apps/ios`.

Reasons:

- best fit for iPhone/iPad layouts, accessibility, file picker, share sheet,
  Keychain, scene lifecycle, and background reconnection behavior
- first-class XCTest/XCUITest and simulator tooling
- less risk than shipping a WebView wrapper for a latency-sensitive terminal
  client

Use Swift Concurrency and `Observable`/SwiftUI state where practical. Avoid a
large architecture dependency in the first slice unless app state becomes hard
to reason about. If reducer-style state pays for itself, evaluate The
Composable Architecture before adopting it.

### Transport

Use `URLSessionWebSocketTask` for the direct kernel WebSocket transport.

Required behavior:

- connect to `ws://127.0.0.1:${CHARIOX_KERNEL_PORT:-43118}/kernel` for local
  development
- support request/response frames
- support subscribe/unsubscribe
- track monotonic `event_id`
- reconnect and resubscribe with `resume_from_event_id`
- handle explicit replay-gap responses by refreshing projections
- surface heartbeat/liveness state in the UI

### Shared Contracts

Start by extracting reusable TypeScript client semantics from `apps/cli` into a
client-core package before implementing Swift UI behavior that could drift.

Candidate shared surfaces:

- slash command parsing and command catalog definitions
- session/agent prompt lifecycle reducers
- terminal output normalization rules
- transcript entry model
- response pane selection rules
- waiting-room row derivation
- provider/model/variant selection semantics
- request shape fixtures and protocol conformance fixtures

The Swift app does not need to execute TypeScript at runtime. It should consume
documented protocol fixtures and match the same state-transition behavior.

## Component Parity Contract

The same conceptual components must exist across CLI, WEB_CLI, and iOS:

- `WaitingRoom`
- `CommandCenter`
- `PromptComposer`
- `AgentPane`
- `AgentPaneFooter`
- `GlobalFooter`
- `RuntimeDrawer`
- `TranscriptView`
- `ArtifactTray`
- `InteractionPrompt`
- `ConnectionBanner`

Parity means equivalent behavior and state, not identical source code or visual
chrome. iOS should adapt layout to touch and small screens without turning into
a dashboard.

## Milestones

### IOS-M0 Product Contract And Skeleton

Status: implemented for the first slice.

Deliverables:

- `apps/ios` Xcode project/workspace committed to the OSS repo
- SwiftUI app shell with local-kernel URL entry
- README with local setup and simulator commands
- initial XCTest target and XCUITest target
- no runtime behavior beyond app launch and configuration persistence

Exit criteria:

- `xcodebuild test` runs from the repo root or a documented script
- app launches in an iPhone simulator
- credentials/config are not stored in plaintext user defaults

### IOS-M1 Kernel Transport

Status: mostly implemented for the local direct-kernel path. Request/response
WebSocket transport, typed envelopes, session list/create/attach/detach
requests, subscribe/unsubscribe framing, event frame decoding, heartbeat
visibility, heartbeat stale detection, replay-gap state, recent history
loading, and reconnect with the last event cursor exist. Remaining work: local
mock-kernel fixtures and end-to-end attach verification against a live local
kernel.

Deliverables:

- Swift kernel client for direct WebSocket transport
- typed request/response envelope decoding
- subscribe/unsubscribe support
- heartbeat, reconnect, replay cursor, and replay-gap handling
- local mock-kernel fixture tests

Exit criteria:

- iOS app connects to a local running `chariox-kernel`
- app can list sessions and show connection state
- transport unit tests cover reconnect and replay-gap behavior

### IOS-M2 Waiting Room

Status: implemented for the first native slice. The app has kernel address,
workspace/worktree target state, session refresh/create/select, attach/detach,
selected-session summary, runtime drawer, attachment state, and global footer.
The first `/session` command subset is also wired through the prompt composer:
`/session list`, `/session new|create`, `/session attach [ref]`, and
`/session detach`, plus session default `/session mode` and
`/session permissions`; local `/workspace` commands update future session
creation targets. Remaining work: broader session command parity and deeper
live-kernel UX polish.

Deliverables:

- waiting-room UI matching CLI/WEB_CLI semantics
- session list, selected kernel address, workspace/worktree target state
- create and attach session actions
- command entry for `/session` basics

Exit criteria:

- user can start the kernel locally, launch the app, create a session, and
  attach to it
- no hardcoded fake runtime state appears after a real kernel is connected

### IOS-M3 Freeform Single-Agent Runtime

Status: implemented for the first local freeform slice. Basic prompt composer,
`SubmitPrompt`, `CancelActivePrompt`, recent transcript history, live terminal
output rendering, runtime notices, completion markers, prompt activity
projection display, interaction prompt responses, and replay-aware event
streaming are in place. Remaining work: richer cancellation reconciliation,
more detailed busy affordances, and live-kernel end-to-end prompt drills.

Deliverables:

- prompt composer
- transcript rendering
- prompt submit
- provider output streaming
- stop/cancel
- session detach/delete handling

Exit criteria:

- app can submit a prompt to the focused agent and stream output live
- cancellation is visible and reconciles with kernel state

### IOS-M4 Multi-Agent Freeform

Status: started. The app can render kernel-reported agents in the runtime
drawer, focus a specific agent, cycle focus, spawn and destroy agents, set
per-agent mode/permission overrides, and route prompt submission through the
selected session's focused agent id. Remaining work: per-agent panes/footers,
split views, richer provider/model/variant state parity, and per-agent stop
affordances.

Deliverables:

- agent list/focus/cycle/spawn
- individual and split views
- pane footers with provider/model/variant/mode/permission state
- busy state and per-agent stop affordance

Exit criteria:

- behavior matches current TypeScript CLI focus and prompt-routing rules
- iPhone uses a compact focused-agent layout
- iPad uses a richer split-pane layout

### IOS-M5 Command Center And Capabilities

Status: started. Slash-command discovery exists for the first safe subset:
`/session`, `/agent`, `/stop`, and `/waiting`. Executed commands call the same
typed kernel-backed model actions as the buttons instead of introducing a new
client-side runtime authority. The native subset now includes `/agent spawn`,
`/agent destroy`, `/agent mode`, `/agent permissions`, `/session mode`, and
`/session permissions`, read-only `/provider list` and `/provider auth`, and
read-only `/mcp list` plus `/skill list`. Remaining work:
provider/model/view mutation parity, Cloud/relay, artifact tray, richer
interaction prompt UX, and shared fixture parity.

Deliverables:

- `/` command discovery
- core slash command parity for session, agent, view, provider/model/variant,
  workspace/worktree, relay/cloud status, MCP/skill read-only state where
  useful
- artifact tray with file/image attach support
- interaction prompts for user/provider permission requests

Exit criteria:

- command behavior is verified against shared fixtures and kernel responses
- failed prompt submissions keep the draft intact

### IOS-M6 Cloud And Relay

Deliverables:

- optional Cloud device login
- hosted relay profile persistence in Keychain
- relay target discovery
- remote kernel attachment through the hosted relay path
- logout/revocation behavior

Exit criteria:

- local direct-kernel usage remains available
- Cloud only supplies identity, relay credentials, and target discovery
- runtime actions still route to the kernel authority

### IOS-M7 Hardening And Release Readiness

Deliverables:

- accessibility labels and dynamic type pass
- iPhone/iPad screenshot matrix
- offline/reconnect UX
- log export that avoids prompt/provider content by default
- CI simulator test script
- App Store/TestFlight checklist if distribution is desired

Exit criteria:

- deterministic XCTest/XCUITest coverage gates PRs
- MCP-driven QA drill captures screenshots/logs for critical flows
- security review covers token storage, relay URLs, logs, and artifact handling

## QA Plan

### Build And Validation MCP: XcodeBuildMCP

Use XcodeBuildMCP as the default implementation-loop MCP once
installed/configured.

Use it for:

- build and test execution
- simulator boot/build/run loops
- screenshots
- app logs
- debugger attachment when needed
- quick launch validation after code changes

Current local note from 2026-04-25: Xcode 26.2 is installed and iOS 26.x
simulators are available. XcodeBuildMCP has been configured in Codex as an MCP
server through `npx -y xcodebuildmcp@latest mcp`.

### Explicit QA MCP: iOS Simulator MCP

Use `ios-simulator-mcp` for explicit QA and dogfooding passes, not for the
default code-build-validation loop.

Use it for:

- simulator state inspection
- screenshots
- accessibility-tree/UI inspection
- taps, typing, and gestures
- less frequent visual and interaction walkthroughs

Future agents should use iOS Simulator MCP when Miguel requests QA or when they
propose a QA pass and Miguel confirms it. It should not be treated as a
secondary default for every implementation step.

Current local note from 2026-04-25: iOS Simulator MCP has been configured in
Codex as an MCP server through `npx -y ios-simulator-mcp@latest`.

### Deterministic Tests

Use Swift Testing or XCTest for unit/domain tests. Use XCUITest for committed UI
regression tests because it integrates with Xcode and CI cleanly.

Minimum committed coverage:

- kernel transport framing and reconnect behavior
- reducer/state transition behavior
- waiting-room row/state derivation
- prompt submission acceptance/failure behavior
- cancellation behavior
- replay-gap refresh behavior
- basic app launch and attach/session flow through XCUITest

### Maestro Guidance

Maestro is approved as a candidate tool, not an automatic dependency.

Future agents should suggest Maestro when it becomes useful, especially for:

- readable cross-screen smoke flows
- fast exploratory mobile regression scripts
- flows that benefit from YAML scenarios outside Xcode
- eventual cross-platform iOS/Android parity

Before adding Maestro to the repo, installing it as a project dependency, or
using it as part of the official QA gate, future agents must ask Miguel for
confirmation.

### Appium Guidance

Do not start with Appium. Consider it later only if Chariox needs WebDriver
compatibility, third-party QA infrastructure, or real-device automation beyond
what XcodeBuildMCP/XCUITest/Maestro cover.

## Skills And Review Workflow

Use these implementation helpers when useful:

- `frontend-design`: for the first serious SwiftUI layout pass and any UI
  polish where the app risks becoming a generic dashboard.
- `design-review`: after simulator screenshots exist, to audit spacing,
  hierarchy, touch targets, text fitting, and visual parity with CLI/WEB_CLI.
- `codex` review/challenge: before merging major transport, state-management,
  or security-sensitive changes.
- `cso`: before Cloud/relay login ships, with focus on Keychain storage,
  token lifetime, relay URL handling, screenshots/logs, and artifact payloads.
- `browse`/browser QA skills: only for comparing against Cloud WEB_CLI behavior
  or checking hosted onboarding references; iOS itself should be QA'd through
  iOS simulator tooling.

## Research Notes

Useful references:

- XcodeBuildMCP: https://www.xcodebuildmcp.com/
- XcodeBuildMCP GitHub: https://github.com/getsentry/XcodeBuildMCP
- iOS Simulator MCP: https://github.com/joshuayoes/ios-simulator-mcp
- Apple `URLSessionWebSocketTask`: https://developer.apple.com/documentation/foundation/urlsessionwebsockettask
- Apple XCTest: https://developer.apple.com/documentation/xctest/
- Apple Keychain guidance: https://developer.apple.com/documentation/security/storing-keys-in-the-keychain
- Maestro iOS docs: https://docs.maestro.dev/get-started/supported-platform/ios
- Appium XCUITest driver: https://appium.github.io/appium-xcuitest-driver/latest/
- Swift OpenAPI Generator: https://github.com/apple/swift-openapi-generator
- Swift Composable Architecture: https://github.com/pointfreeco/swift-composable-architecture

## Implementation Decisions

- Project shape: Xcode workspace and app shell plus `CharioxPackage` Swift
  package, scaffolded under `apps/ios`.
- Minimum iOS version: iOS 17 for the first slice.
- State management: native SwiftUI `@Observable`; no TCA dependency yet.
- Protocol typing: hand-written Codable request/response models with fixture
  tests first, schema generation deferred.
- First product scope: local-kernel only; Cloud login deferred to IOS-M6.

## Deferred Decisions

- Whether to introduce The Composable Architecture if state complexity grows.
- Whether to generate Swift protocol types from a schema after more protocol
  surfaces are implemented.
