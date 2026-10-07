# Seatbelt lane status

Status: FINAL, ready for coordinator review. Branch `am/seatbelt`; base `ca23d8677`. Commit uses `[skip ci]`. No push.

Both requested fixes are implemented. The compiler cannot reach host process, files or commands. macOS uses Seatbelt and safe schema-file opens; unsupported hosts fail closed. The schema replay limit now permits the final compile after 32 successful resolution rounds.

Validation passed on this Mac:

- 140 workflow/compiler tests.
- 59 focused workflow API tests.
- Eight built-kernel and rendered-TUI checks, including ordinary Codex workflow completion, 32-schema compilation and refusal of the room agent's compile request.

Evidence is under `/Users/miguel/.codex/evidence/coordinator-seatbelt/`. Temporary fixture workspaces, compiler scratch directories, diagnostic probes and large Cargo outputs were cleaned up. The checked kernel binary and protected runtime state remain outside the repository. All drill-owned kernel, TUI and provider PIDs are gone; reviewer services and existing provider sessions were preserved.

The memory limit uses five-millisecond sampling of resident memory and physical footprint in addition to the V8 heap limit. Seatbelt's loader allowances and the live drill's existing-agent binding are recorded in `PUSH_READY.md` and `docs/WORKFLOW_COMPILER_ISOLATION.md`.
