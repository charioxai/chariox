`@opentui/core@0.1.87` retains detached render trees in
`Renderable._shouldUpdateBefore` when an ancestor is hidden. `remove()` must
remove the child from this pending-layout set even when no subsequent visible
layout pass occurs. Destruction also clears pending and cached child lists.

The patch targets the published JavaScript bundle and is applied by pnpm from
the lockfile. `apps/cli/src/hidden-pane-retention.bun-test.ts` exercises real
hidden auxiliary panes, interaction strips, App consent cards and event-stream
reconnects. Keep these tests when upgrading OpenTUI; remove the patch once the
upstream package releases detached children itself.
