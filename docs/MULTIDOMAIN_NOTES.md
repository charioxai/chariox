# MD-N1–N5 / MP-08 / MP-10 / MP-11: selected-text notes

Protocol 424 adds `Notes { command }` and private `Notes { result }` responses.
The home kernel owns the durable SQLite notes and audit under its private
runtime state. Author and owner come from terminal authentication, never a
request. Notes are independent of agent/Room history. Nothing broadcasts note
contents to session members, Apps, browser pages or the relay control plane.

MD-N1: capture/report gives an owner-bound selection receipt. `create` consumes
that receipt idempotently with a comment. Native transcript/editor panels use
`report_selection` with their node/message hint; terminal selection carries its
scrollback hint. Browser and App targets must use `capture_selection`, never a
client-claimed streamed quote. Original exact/prefix/suffix quote remains immutable.
Replies and resolve are durable; create/read/list/reply/resolve/reanchor/Ask are
audited without copying note contents into the audit. Listing returns bounded
metadata summaries; `read` returns the selected text, comment and replies.

MD-N2: both kernel-browser and Room-browser windows use the shared controller's
`browser.notes.observe` operation. The observer is installed lazily into #607's
controller isolated world. Capture immediately reads the current selection,
then watches selection and DOM changes. Frame traversal uses the existing
controller frame ownership channel. Quote matching spans text nodes, open
shadow roots and owned frames. Contextual matches take priority across every
owned frame and root; exact-only fallback runs only when no contextual match
exists anywhere. Ambiguous matches do not silently pick one.
Closed shadow roots and selections spanning separate DOM roots return no
selection. Quote size, frame/root count and text indexing are bounded. Context truncation
uses Unicode code points so an emoji cannot break the serialized quote. A changed
or unavailable page retains the original quote, with `missing`, `ambiguous` or
`unavailable` state. Read refreshes browser anchors. Durable host-tab listing follows stable tab
identity across browser generations; re-anchoring uses the current observed
generation, while new selection capture still rejects stale generations. The observer creates no DOM
nodes or page-visible bindings. No note UI is injected into the browser/App.
Existing Vault observation barriers and scrubbing apply to controller results.
MD-N2 / MP-08/MP-10/MP-11: before splitting a quote or truncating its context,
the isolated observer checks complete protected-value spans in the raw text
index using the shared observation variant policy. A selection whose exact
text or 64-code-point context overlaps a protected span is withheld (null),
including values crossing text nodes or a context cutoff. Kernel-replayed
retired values protect cached observers too. DOM offsets remain unchanged for
benign selections; protected values never enter page/App JavaScript worlds.

MD-N3 Cloud integration: import `notesRequest`, `noteOverlayBox`, and
`notePromptDraft` from the OSS shared client. Gate notes at protocol 424 before
sending. Poll `capture_selection` for the visible streamed window; its
`selection_changed` result is scoped to the requesting terminal. Clear the
host icon on a null selection, target/generation change, disconnect, or stale
reply. Draw the round icon and composer in trusted host UI outside all App/page
frames. Render quote, comment and reply strings as plain text nodes. Use `noteOverlayBox` with the **rendered page rectangle**, including
browser-bar/App-panel offsets and letterboxing, rather than the entire screen.
Native App frames report selections through the authenticated host bridge;
never accept page `postMessage` as a streamed selection report.

Clicking Ask requests `ask` for the saved note. The response contains visibly
marked selected text/comment and an ordinary `PromptAttachment` (text/plain,
filename, URL and base64 contents). Insert the returned text and attachment into
the prompt editor. Preserve any existing draft. Only the user's ordinary Send
uses `SubmitPrompt`; Ask itself never creates, dispatches or resolves a prompt.
TUI note UI is deferred by the owner.

MD-N4 / MP-08 / MP-11: `chariox.load_notes` loads list/read/reply/resolve on
demand while focused. Browser and notes loading remain independent. A note read
claims its stable note ID; changing focus retains that note while the agent has
an active turn or a pending wake, then for the configured idle window.
MP-11: retained holders can read, reply, resolve and use the ordinary note
operations on granted resources, with the same uninterrupted grant epoch at
the commit boundary as focused agents. New-resource claims still require focus.
Automatic anchor refresh during a read remains observation housekeeping. A retained
holder cannot claim another note. Listing metadata does not claim contents; a
retained list shows only claimed note IDs. Explicit revoke, idle expiry, session
end and agent destruction fence results and commits. See
[MULTIDOMAIN_USER_DOMAIN_ACCESS.md](MULTIDOMAIN_USER_DOMAIN_ACCESS.md) for the
432/78 shared contract. Remote/leased calls explain both kernels without control
forwarding. Room membership alone grants no notes. Human terminals manage only
their own notes.

MD-N5 validation receipts live outside the repository. Source/unit/native drills
are separate proof; none closes MP-08, MP-10 or MP-11 or proves Cloud overlay,
real provider execution, hosted relay, macOS or fresh-machine acceptance.

MD-N5 native drill: run the ignored
`runtime::router::tests::notes::live::notes_user_and_room_native_integration_drill`
with an explicit disposable `CHARIOX_HOME` below `CHARIOX_MDNOTES_DRILL_ROOT`,
as a normal Unix user with native sandboxed Chromium and usable fonts. Set
`CHARIOX_KERNEL_BROWSER_EXECUTABLE`, `CHARIOX_KERNEL_BROWSER_HEADLESS=1`, and
`CHARIOX_MDNOTES_DRIVER` to `notes-drill-driver.mjs`; use that same driver as
`CHARIOX_BROWSER_CONTROLLER_SCRIPT`. The driver serves the normal stdio controller
and scopes its real process/profile inventory to its own launched Chromium child.
This fixture does not supply a Docker PID namespace or verify slice provisioning.
It provides genuine DOM selection stimuli, including page-world forgery, to the
normal kernel host and Room controller paths. Receipt output contains fixture
checks only; remove all fixture browser profiles and product identities afterward.

MD-N5 / MP-08/MP-10/MP-11 local-connection regression:
`runtime_transport::tests::kernel_browser_terminals::mdnotes_two_local_connections_keep_observation_and_takeover_private`
uses two actual loopback WebSockets, the production Rust authority and JavaScript
host adapter, and a synthetic CDP factory in an isolated child. It checks private
document observations and takeover actors without Chromium or provider execution.
The fixture owns and removes its state; it does not establish native or hosted acceptance.

MD-stack integration: the unreleased feature allocation is folded into local
protocol 427 (relay peer 74). This union and its shape/hash guards supersede the
per-feature versions described during development above.
