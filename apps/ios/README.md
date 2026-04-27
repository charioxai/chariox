# Arroba iOS

Native SwiftUI client for the Arroba OSS runtime.

The iOS app is a client surface like the terminal CLI and Cloud WEB_CLI. It
does not own runtime authority: sessions, agents, workflow runs, provider
state, permissions, and relay behavior remain kernel-owned.

## Project Shape

```
apps/ios/
├── Arroba.xcworkspace          # Open this in Xcode
├── Arroba.xcodeproj            # Minimal app shell
├── Arroba/                     # App target and assets
├── ArrobaPackage/              # Main SwiftUI feature package
└── ArrobaUITests/              # XCUITest launch and flow coverage
```

Most implementation belongs in:

```
apps/ios/ArrobaPackage/Sources/ArrobaFeature/
```

Current package areas:

- `Kernel/`: local-kernel protocol envelopes and WebSocket client.
- `State/`: observable app state and async runtime actions.
- `Views/`: SwiftUI waiting-room UI and reusable components.

## Local Development

From the repo root, start the local kernel:

```sh
pnpm run start:kernel
```

The app defaults to:

```text
ws://127.0.0.1:43118/kernel
```

The default workspace/worktree path is `/Users/miguel/arroba`; it is editable
from the waiting-room UI.

Current local-kernel flow:

- Refresh sessions from the configured kernel URL.
- Create a session for the configured workspace/worktree.
- Select a session in the waiting room.
- Attach to the selected session.
- Subscribe to kernel events for the attachment.
- Apply session snapshots, heartbeat state, replay-gap state, and reconnect
  with the last event cursor after stream interruptions.
- Mark the event stream stale when a live attachment stops receiving kernel
  heartbeats.
- Detach from the active attachment, cancelling the event stream and sending
  `DetachFromSession`.
- Submit a text prompt to the selected session/focused agent through
  `SubmitPrompt`; successful submissions clear the draft and failed
  submissions keep it intact.
- Request active prompt cancellation through `CancelActivePrompt`.
- Render selected-session prompt activity from `active_prompt`, queue, and
  per-agent prompt-state projections.
- Render kernel-owned interaction prompts and respond through
  `RespondToInteraction`.
- Load recent session history for the focused agent after attach and render
  live terminal output, notices, and assistant-completion markers.
- Focus a specific kernel-reported agent or cycle focus from the runtime
  drawer; prompt submission routes through the selected session's focused
  agent id.
- Spawn a new agent from the selected session, destroy an existing agent with
  confirmation, and set focused-agent mode/permission overrides through
  kernel-backed requests.
- Load provider catalog and provider auth status through read-only kernel
  requests.
- Load installed MCP server and skill inventory through read-only kernel
  requests.
- Show and update local workspace/worktree target paths through `/workspace`
  commands.
- Type `/` in the prompt composer to show the first native command-center
  subset. Supported commands currently include `/session list`,
  `/session new`, `/session attach [ref]`, `/session detach`, `/agent list`,
  `/session mode [build|plan]`, `/session permissions [required|yolo]`,
  `/agent spawn [alias] [model]`, `/agent destroy [ref]`, `/agent focus <ref>`,
  `/agent cycle`, `/agent mode [ref] <build|plan|inherit>`,
  `/agent permissions [ref] <required|yolo|inherit>`, `/provider list`,
  `/provider auth [provider]`, `/mcp list`, `/skill list`, `/stop`, and
  `/waiting`. Local workspace target commands include `/workspace show`,
  `/workspace set <path>`, `/workspace path <path>`, and
  `/workspace worktree <path>`.

## Build And Test

Run package tests:

```sh
swift test --package-path apps/ios/ArrobaPackage
```

Build for the local simulator:

```sh
xcodebuild \
  -workspace apps/ios/Arroba.xcworkspace \
  -scheme Arroba \
  -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' \
  build
```

Run package and UI tests through Xcode:

```sh
xcodebuild \
  -workspace apps/ios/Arroba.xcworkspace \
  -scheme Arroba \
  -destination 'platform=iOS Simulator,name=iPhone 17,OS=26.2' \
  test
```

If simulator names or OS versions differ, list available devices with:

```sh
xcrun simctl list devices available
```

## Component Parity

The app should preserve conceptual parity with CLI and WEB_CLI components:

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

Parity means equivalent behavior and state. iOS can and should adapt visual
layout for touch, small screens, iPad, accessibility, and native navigation.

## QA Guidance

Use XcodeBuildMCP as the default build/test/run validation MCP when available.
It is the normal loop for compilation, simulator launch, test execution, logs,
and debugger-oriented validation.

Use iOS Simulator MCP for explicit QA and dogfooding passes when Miguel asks
for QA or confirms a proposed QA pass. It is useful for simulator state
inspection, screenshots, accessibility tree inspection, taps, typing, and
gesture walkthroughs.

Maestro is a candidate tool, not an automatic dependency. Future agents should
suggest Maestro when readable mobile smoke flows or exploratory regression
scripts would help, but must ask Miguel before installing it, adding it to the
repo, or making it part of the official QA gate.

## Architecture Rules

- Keep runtime-changing actions behind kernel or relay-backed kernel requests.
- Store secure credentials in Keychain when Cloud/relay login is added.
- Keep UserDefaults limited to non-secret local preferences.
- Prefer SwiftUI `@Observable` state and Swift Concurrency.
- Avoid introducing a large state-management dependency until complexity
  justifies it.
- Keep protocol shapes covered by tests and fixtures so Swift does not drift
  from the TypeScript CLI/client behavior.
