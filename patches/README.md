`@opentui/core@0.1.87` retains detached render trees in
`Renderable._shouldUpdateBefore` when an ancestor is hidden. `remove()` must
remove the child from this pending-layout set even when no subsequent visible
layout pass occurs. Destruction also clears pending and cached child lists.

The patch targets the published JavaScript bundle and is applied by pnpm from
the lockfile. `apps/cli/src/hidden-pane-retention.bun-test.ts` exercises real
hidden auxiliary panes, interaction strips, App consent cards and event-stream
reconnects. Keep these tests when upgrading OpenTUI; remove the patch once the
upstream package releases detached children itself.

Repeated empty text updates also allocate native rope nodes without resetting
the text buffer's arena. Split-pane footers repeatedly exercise this path. The
patch skips native replacement and clearing when both the existing buffer and
replacement are empty, for styled text, plain text and direct clears. It preserves
highlight clearing and JS bookkeeping, clears nonempty text and accepts the next
nonempty update. `apps/cli/src/native-text-retention.bun-test.ts` exercises real
TextRenderable updates and verifies bounded native allocation counts. Keep this
guard until upstream empty replacements reuse or reset their native arena.

MP-08 / MP-10: the stdin parser also keeps incomplete CSI reports until their
final byte arrives, including SGR mouse reports delayed by SSH or renderer
work. A new ESC and the existing pending-byte bound still recover malformed
input. Lone ESC gets a 250 ms ambiguity window instead of 10 ms; this avoids
turning the first fragment of a drag into an Escape shortcut. While a mouse
button is held, a pending Escape waits for the next report instead of cancelling the drag on a timer. After release, standalone Escape
keeps its bounded deadline. Both the renderer and raw shortcut parser use the same patched framing. Real-renderer regressions
in `tui-mouse-acquisition.bun-test.ts` cover delayed mouse-down and movement,
including a pause inside CSI parameters. Keep these checks on OpenTUI upgrades.
