# Arroba iOS Agent Notes

This is the native iOS client for the Arroba OSS runtime.

## Product Boundary

- The iOS app is a client surface like the terminal CLI and Cloud WEB_CLI.
- The kernel remains the runtime authority for sessions, agents, workflow
  runs, provider state, permissions, workspace live sync, and relay behavior.
- Runtime-changing actions must go through kernel or relay-backed kernel
  requests.
- Client-local state is limited to view state, drafts, local preferences,
  selected addresses, cached projections, and secure credentials.

## Project Shape

- Open `Arroba.xcworkspace` in Xcode.
- Keep the app shell in `Arroba/` minimal.
- Put feature code in `ArrobaPackage/Sources/ArrobaFeature/`.
- Put package tests in `ArrobaPackage/Tests/ArrobaFeatureTests/`.
- Put simulator UI tests in `ArrobaUITests/`.

## Swift Guidelines

- Target iOS 17+ for the first implementation slice.
- Prefer SwiftUI, Swift Concurrency, and `@Observable` state.
- Keep views small and extract reusable subviews when body complexity grows.
- Do not introduce TCA, SwiftData, or another large dependency without a clear
  need and a documented decision.
- Do not store Cloud/relay credentials in UserDefaults. Use Keychain when those
  features are added.
- Keep protocol shapes covered by tests and fixtures so Swift behavior does not
  drift from `packages/kernel-client` and the CLI.

## Validation

Use XcodeBuildMCP as the default build/test/run validation path when available.
If MCP tools are not exposed in the current session, use documented
`xcodebuild` commands from `README.md`.

Use iOS Simulator MCP only for explicit QA/dogfooding passes when Miguel asks
for QA or confirms a proposed QA pass. Suggest Maestro when it would help, but
ask Miguel before installing it, adding it to the repo, or making it a QA gate.
