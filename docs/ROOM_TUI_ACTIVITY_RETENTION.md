# Room activity across provider-history refresh

The local Web + Codex + both-TUI drill on 2026-09-04 completed the attributed
desktop click and displayed its physical result in Web, but failed the remote
TUI assertion for action #2 after the provider turn completed. The final TUI
snapshot retained later human actions but not the earlier provider click.

Provider-history refresh replaced the pane's locally rendered Room notices.
Room events belong to the kernel Environment ledger, not provider history, so
reloading the latter cannot reconstruct them. Full and targeted pane refresh
regressions reproduce this loss without a running slice.

The TUI now marks Environment notices with the existing client-only merge key,
derived from Room, Environment, replay cursor and notice index. Refresh keeps
at most 128 such notices and 64 KiB of their text, outside provider turn
grouping. It reads current notices after history I/O so events arriving during
the request survive too. Ordinary notices and provider text are not retained
by text matching. Session identity checks prevent applying a late full refresh
to another Room. This is a bounded display projection, not a second history
authority or a durable replacement for `/room actions`.

Validation:

```sh
bun test apps/cli/src/agent-pane-refresh-controller.test.ts \
  apps/cli/src/room-environment-activity-controller.test.ts \
  apps/cli/src/transcript-event-controller.test.ts \
  apps/cli/src/room-activity-notice-state.test.ts
```

The two refresh regressions failed before the fix. The focused suite has 25
passing tests after it. Live Web + real-provider + both-TUI rerun remains
required. No serialized protocol shape or minimum client version changes.
