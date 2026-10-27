# CLI TUI Repaint Skill

Use this when OpenTUI/JVX state changes are correct but the screen does not visually update, especially in split multi-agent response panes.

## What usually goes wrong

- Updating parent pane colors is not enough; transcript entries, markdown blocks, diffs, and scrollboxes often carry their own backgrounds.
- Focus changes and split activation are not the same code path. A split view can paint correctly on mount but still miss later focus-driven repaints.
- Layout can be applied before pane refs exist, so an early repaint attempt can silently do nothing.
- Some nested renderables need both `requestRebuild()` and `requestRender()`, not just a top-level renderer refresh.
- OpenTUI layout measurements can settle a tick later, so one immediate repaint is often insufficient.

## Working strategy

1. Treat visual state changes as explicit repaint events.
   - Focus changes, split/individual toggles, pane ownership swaps, and agent-count changes should all trigger the same repaint path.

2. Reapply layout first.
   - Mutate pane borders, widths, visibility, padding, footer colors, scrollbox backgrounds, and any other surface props in one place.
   - In this repo, that place is `apps/cli/src/index.tsx` in `applyResponseLayout()`.

3. Rebuild pane content if child surfaces depend on focus.
   - If transcript entry bodies, markdown containers, or diff backgrounds change by focused/unfocused state, rebuild the affected transcript panes after layout mutation.
   - Do not assume child renderables will inherit the parent background.

4. Force repaint on every affected layer.
   - Request render on the primary pane, secondary pane, layout box, scrollboxes, footers, and finally the renderer.
   - If nested renderables can cache layout or content, walk the subtree and call `requestRebuild?.()` and `requestRender?.()`.

5. Repaint more than once.
   - Run the repaint immediately, then again on `setTimeout(..., 0)`, then again on `setTimeout(..., 16)`.
   - This matches the pattern that helps when OpenTUI/JVX finishes layout a frame later.

6. Re-run after refs mount.
   - If `applyResponseLayout()` runs before the pane refs exist, return early and call it again from the relevant `ref` callbacks.

## Repo-specific playbook

- Layout mutation lives in `apps/cli/src/index.tsx` in `applyResponseLayout()`.
- Split-focus repaint entry point should be triggered from `applySessionState()` whenever split mode is active and any of these change:
  - focused agent id
  - active layout
  - active agent set
- Primary transcript rendering uses `buildTranscriptEntryRenderable()`.
- Patch/diff transcript blocks use `buildApplyPatchTranscriptContent()`.
- If focused/unfocused panes should look different, thread a pane-surface tone through those builders instead of only changing outer pane backgrounds.

## Minimal checklist before declaring repaint fixed

- `Ctrl+A` changes split-pane tint immediately.
- `/agent focus <id>` changes split-pane tint immediately.
- `/view split` paints correctly on first entry into split mode.
- Switching back and forth between one and two panes does not require extra input to repaint.
- Markdown and diff blocks also change surface tone with focus, not just the pane border/footer.

## Anti-patterns

- Do not rely on reactive state alone if the bug is visual-only.
- Do not only call the top-level renderer if nested renderables have their own cached surfaces.
- Do not only repaint the outer pane if transcript entries have explicit backgrounds.
- Do not test only `/view split`; also test `Ctrl+A` after the split is already visible.

## If you are debugging a fresh repaint bug

Start from the focus-change path, not just the split-mount path. If the data routing is correct but the colors are stale, the bug is almost always one of:

- focus state changed without entering the explicit repaint path
- child transcript renderables were not rebuilt for the new surface tone
- repaint happened before refs or final measurements were ready
- only the parent pane was repainted
