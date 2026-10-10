# TUI copy and selection (MP-08 / MP-10)

Drag across transcript text to select it. Releasing the mouse copies the
selection and keeps its highlight. Press **F6** to copy it again. F6 works with
legacy terminal keyboard input, including Terminal.app and tmux without extended
keys; a Mac keyboard may require Fn+F6. **Meta+C** also copies a selection and
otherwise retains its queued-prompt cancel action.

**Ctrl+C stops the active agent or exits an idle CLI.** Terminal.app and other
terminals without extended keyboard support send that same byte for Ctrl+Shift+C,
so use F6 instead. The TUI accepts a separately encoded Ctrl+Shift+C when the
terminal supplies it, but does not advertise it as a portable copy key.

Over SSH, OSC 52 clipboard delivery is unconfirmed and some terminals ignore it.
Use the terminal's native Copy action after Shift-dragging (terminal dependent),
or set `CHARIOX_TUI_MOUSE=off` to select text with the terminal itself.
