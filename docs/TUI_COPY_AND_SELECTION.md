# TUI copy and links (MP-08 / MP-10 / MP-11)

Drag-select text, then use the terminal's Copy action: **Cmd-C** in
Terminal.app/iTerm/kitty on macOS. No F7, Shift modifier or clipboard extension
is required. The terminal owns the selection and clipboard by default, including
over SSH. Use keyboard navigation for TUI controls.

**Cmd-click** opens a link using the terminal's native handling. Terminal.app
may require **Cmd-double-click**. Links remain on the TUI screen; Chariox does
not switch to a plain terminal buffer. Terminals supporting OSC 8 receive the
complete URL even when the displayed link wraps. Terminals without OSC 8 use
the displayed URL and their native URL detection.

`CHARIOX_TUI_MOUSE=on` opts into app mouse controls. In that mode, transcript
drag-release, **F6** and **Meta+C** use the app clipboard path; OSC 52 delivery
is unconfirmed over SSH. Use the terminal's selection modifier in that mode.
**Ctrl-C** retains its stop-active-agent / exit-idle-CLI action.

Provider sign-in remains in the TUI: open the authorization link in your desktop
browser, paste the returned code into the focused Code field, and press Enter.
The kernel's result reports who signed in and which profile was saved. On a
local desktop, an explicit login start or the **O** action may also ask the
browser to open; SSH links open on the client through the terminal.
