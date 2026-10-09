# User-domain App text prototype

This removable TUI prototype requires `CHARIOX_USER_APP_VIEWS_PROTOTYPE=1`,
the same opt-in name as Cloud's native-rendering build flag. It is off by default;
no panel, poll timer, App requests or input hooks are created when disabled.
Use a protocol-418 kernel from the multidomain integration branch. Known older
kernels get the shared minimum-version diagnostic before these requests.
Protocol remains 418/70; this client uses the existing 417/418 wire shapes.

## Opening and reading

From the waiting room, `/app open INSTALLATION` opens a **user-domain** view.
`/app view open INSTALLATION` does so explicitly from either the waiting room
or an attached session. Attached `/app open INSTALLATION` and explicit
`--session` commands keep the existing Room behavior. `/app views` lists the
owner's instances; `/app view show VIEW` selects an existing text-capable view;
`/app view close [VIEW]` explicitly closes the selected or named instance.

The TUI cannot run an App's frontend JavaScript natively. It requests the shared
kernel Chromium fallback (`host: kernel_browser`), which loads the verified
bundle and supplies the same App channel as native hosting. No Room, slice or
Session is created. Kernel and App sandbox/permissions/Vault authority remain
unchanged. A native-hosted view appears in the list but cannot be projected;
open a separate TUI instance rather than silently migrating another client.

The panel reads Chromium's accessibility snapshot, including role, name,
value, description, states and focus. It shares the existing `/room read`
formatter. App-provided terminal controls are escaped; output is capped at
500 nodes, 16 indentation levels and bounded field lengths. DOM/HTML/scripts
are never rendered or executed in the terminal. The kernel redacts protected
field values; its existing secret-input checks still apply.

## Keyboard and approvals

While the panel is open, Tab, ordinary typing, Enter, Backspace/Delete,
arrows and Home/End act on the App frontend through KernelBrowser input.
The frontend calls `window.chariox.call` through the existing kernel App channel.
Modifier navigation combinations are deferred in this prototype. PgUp/PgDn
scroll the outline; Escape returns to the prompt **without closing** the App;
Ctrl+W closes it. Each input validates the selected owner-visible view and its
tab/generation before sending. The queue is bounded and stale queued input is
discarded. A read-only refresh updates the visible outline every 750 ms.

From the prompt, `/app view call METHOD 'JSON'` invokes the selected instance's
App channel directly. It supplies neither an actor nor session: the kernel
uses terminal admission. App errors remain kernel/App decisions.

`/app view approvals` shows the kernel's owner-scoped detached interactions;
`/app view answer INTERACTION CHOICE` answers a current noncritical choice.
Critical approvals require F8 and the existing trusted Chariox passkey popup.
The popup accepts session-less *critical App* prompts only behind the flag,
sends AnswerUserDomainInteraction on the original connection and checks the
matching acknowledgment. Passkeys cannot be supplied through App commands,
accessibility content or the App bridge. Opening the trusted popup hides the
App panel and preserves the prompt draft/focus. Existing Room/access/sudo
popup routing stays on its established path.

## Connection and lifecycle

Each selected/created view captures the concrete admitted kernel client,
not the mutable kernel selector. Reads/actions fail after a kernel switch;
no stale view id or approval proof is sent to the new kernel. Cleanup closes
only instances opened by this prototype, through their originating clients.
Views selected from the list remain open unless explicitly closed. Opening
failures close a newly allocated instance on its original client.

Escape keeps the kernel instance alive; TUI exit closes its created instances
when the connection is still usable. Disconnection cannot guarantee delivery.
The kernel's explicit-close ephemeral policy has no reconnect grace or restart
restoration: restart ends instances and callers must open again. No agent,
workspace or session ownership is inferred from TUI focus.

## Validation and removal

Focused Node tests exercise command routing, bound input, owner approval choices,
terminal escapes, kernel switching and native-host refusal. OpenTUI tests cover
flag-off allocation, real terminal key parsing, prompt retention, popup labeling
and switched-kernel proof refusal. Existing Room App/outline and popup suites
are unchanged and rerun.

After building the CLI, run:

```sh
bun apps/cli/scripts/user-app-view-tui-drill.mjs /absolute/external/evidence
```

This drives a real PTY and the production OpenTUI projection/composition,
App slash-command handler and LocalIpcClient against a **synthetic loopback
kernel/App peer**. Nine checks cover session-less list/open/accessibility text,
Tab/type/Enter channel action, direct keyboard call, kernel approval display/
choice, close and absence of Room/session requests. It saves a rendered frame,
terminal transcript, wire requests and results outside the checkout. It does
not claim full-CLI bootstrap, a live daemon, provider, Chromium or real Vault.

The separate real sandboxed `multidomain-host-drill.mjs` validates the production
kernel router, signed App, actual Chromium page bridge and fixed ABI worker,
synthetic approval, Tab/type/Enter App call and accessibility reply, close,
then focused runtime MCP tab access. It exposed and now covers Chromium's need
for Enter's native carriage-return key text to execute form/button actions.
No protocol shape, unsafe Chromium flag, relay policy or client authority was
added for that correction.

Remove `createCliUserAppViewsComposition` wiring, its named controller/renderer,
flagged App command hook/help and detached popup adaptation to remove this tier.
The kernel App service and host seam remain independently usable. The shared
Room accessibility formatter remains a small pure module.

## Open questions

- Owner: confirm the text tier and kernel-host fallback before enabling this flag.
- Owner: choose additional accessibility selection/action conventions and keyboard
  modifier support; this prototype implements basic human keyboard interaction.
- Coordinator: provide a compatible approved signed App runtime for release trust
  acceptance. Live full-CLI/display checks use a private signed test runtime below.
- Reconnect grace, restart restoration and focused MCP naming remain deferred
  as documented in MULTIDOMAIN_APP_VIEWS.md; no restoration is claimed here.

## Live full CLI follow-up

With the prototype flag, `/` in the waiting room focuses the existing command
input; it does not activate a session. The shortcut is listed in waiting-room
hotkey help. `/app view open INSTALLATION`, `/app views`, and `/app view show VIEW`
are then available without a Room. Once projected, Tab/type/Enter sends real
keyboard actions; Ctrl+W closes the view, Escape returns to the command input.
The flag-off waiting room retains its existing navigation.

The Cloud `user-app-view-browser-drill.mjs` now drives the full standalone CLI
in an owned tmux pane against the same disposable production kernel used by
native web and shared Chromium tests. It checks open/list, real accessibility
text, keyboard App channel execution and rendered backend reply, close, and no
session creation. The old terminal component drill remains a focused synthetic
regression. Live display acceptance still uses a temporary signed test runtime
on this builder until its installed runtime group-mapping ABI is updated; see
Cloud's USER_APP_VIEWS_PROTOTYPE.md for that trust boundary. Hosted/account
bootstrap and approved release-artifact acceptance remain separate.
