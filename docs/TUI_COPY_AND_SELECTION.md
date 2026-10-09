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

## Provider sign-in links (MP-08 / MP-11)

A provider sign-in (for example `/provider login claude <profile>`) appears as
numbered steps above the prompt with a countdown. The authorization link sits
alone on its rows, wrapped only at the terminal edge, and is an OSC 8 hyperlink
where the terminal supports it (`CHARIOX_TUI_HYPERLINKS=off` keeps plain text).
The code field has the cursor: paste the provider's code and press Enter.

Click the link (mouse reporting stays on). On the local desktop Chariox opens
the browser. Over SSH it shows the plain link view: the link is one
terminal-wrapped line for Cmd-click or native selection, and pasting the code
there returns to Chariox with it. With the code field empty, **O** does the
same and **C** copies the link (OSC 52 over SSH is a request, never a
confirmed copy).

After the code is sent the TUI follows the kernel's login status and ends with
`Signed in to <provider> · saved to <profile>` or the reason the sign-in failed
and the command that retries it.
