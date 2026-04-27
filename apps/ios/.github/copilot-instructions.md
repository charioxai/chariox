# Copilot Instructions For Arroba iOS

- This is the native SwiftUI iOS client for the Arroba OSS runtime.
- The app targets iOS 17+ in the first slice.
- The kernel is the runtime authority. Do not duplicate session, agent,
  workflow, provider, permission, or relay authority in the client.
- Put feature code in `ArrobaPackage/Sources/ArrobaFeature/`; keep the app
  target as a thin shell.
- Prefer SwiftUI, Swift Concurrency, and `@Observable` state.
- Keep runtime protocol changes covered by Swift tests and aligned with
  `packages/kernel-client`.
- Use XcodeBuildMCP for build/test/run validation when available. If MCP tools
  are not exposed, use the `xcodebuild` commands documented in `README.md`.
- Use iOS Simulator MCP for explicit QA/dogfooding only when Miguel requests or
  confirms it.
- Suggest Maestro only when it would be useful; ask Miguel before adding it as
  a dependency or official QA gate.
