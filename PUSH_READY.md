# Seatbelt lane ready

Branch: `am/seatbelt`, based on `ca23d8677` from OSS PR #911. Ready for coordinator review. No push or GitHub CI run.

Implemented the macOS Seatbelt compiler backend, descriptor-relative safe schema imports, bounded serialized I/O and compiler supervision. Caller-selected Node paths remain ignored. Missing Seatbelt fails closed with an actionable error. Windows remains fail-closed and is documented.

The 32-schema regression failed at the resolution limit before the fix and now passes. Compilation gets its final attempt after 32 permitted resolution rounds; an unresolved 33rd request is still refused. Runtime dependencies and the private compiler boundary are prepared once per compilation, rather than once per replay.

Validation on this Apple Silicon Mac, macOS 26.5:

- Workflow/compiler suite: 140 passed, zero failed.
- Focused workflow API suite: 59 passed, zero failed.
- Built kernel and rendered TUI drill: eight checks passed. The owner validated an ordinary workflow and the 32-schema source, ran the ordinary workflow to completion through the official Codex CLI, and a room agent's compile request was refused.
- Formatting, whitespace and drill syntax checks passed.

The live workflow binds the room's existing native Codex agent and disables context flushing. Completion is asserted against the live TUI projection. The drill uses metadata-only native-account registration and reports verdicts without guard source, provider prompts or credential contents.

Evidence: `/Users/miguel/.codex/evidence/coordinator-seatbelt/focused-tests.json` and `macos-live-drill.json` in the same directory. These record test names, counts and verdicts. The built kernel is retained outside the worktree; large Cargo outputs and disposable drill files were removed. Owned kernel, TUI and provider processes exited. Protected state and shared services were retained.

No serialized protocol shape changed; protocol version 450 and client minimum versions remain unchanged. Rust 1.94 required raising the existing kernel crate's recursion limit to 256 to build its runtime async type on this Mac.

macOS memory enforcement combines the V8 heap limit with five-millisecond resident-memory and physical-footprint sampling. A live child whose usage cannot be measured is refused. Sampling allows a transient overshoot. Seatbelt permits loader ancestor metadata and root-directory access, without granting reads of root children. Linux runtime validation and Windows execution were not run in this Mac lane.

See `docs/WORKFLOW_COMPILER_ISOLATION.md` for the boundary and focused commands. The live drill is `apps/cli/scripts/live-macos-workflow-compiler-drill.mjs`; supply absolute kernel, state, fixture-parent and evidence paths as documented.
