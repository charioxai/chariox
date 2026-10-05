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
shadow roots and owned frames. Ambiguous matches do not silently pick one.
Closed shadow roots and selections spanning separate DOM roots return no
selection. Quote size, frame/root count and text indexing are bounded. A changed
or unavailable page retains the original quote, with `missing`, `ambiguous` or
`unavailable` state. Read refreshes browser anchors. Durable host-tab listing follows stable tab
identity across browser generations; re-anchoring uses the current observed
generation, while new selection capture still rejects stale generations. The observer creates no DOM
nodes or page-visible bindings. No note UI is injected into the browser/App.
Existing Vault observation barriers and scrubbing apply to controller results.

MD-N3 Cloud integration: import `notesRequest`, `noteOverlayBox`, and
`notePromptDraft` from the OSS shared client. Gate notes at protocol 424 before
sending. Poll `capture_selection` for the visible streamed window; its
`selection_changed` result is scoped to the requesting terminal. Clear the
host icon on a null selection, target/generation change, disconnect, or stale
reply. Draw the round icon and composer in trusted host UI outside all App/page
frames. Use `noteOverlayBox` with the **rendered page rectangle**, including
browser-bar/App-panel offsets and letterboxing, rather than the entire screen.
Native App frames report selections through the authenticated host bridge;
never accept page `postMessage` as a streamed selection report.

Clicking Ask requests `ask` for the saved note. The response contains visibly
marked selected text/comment and an ordinary `PromptAttachment` (text/plain,
filename, URL and base64 contents). Insert the returned text and attachment into
the prompt editor. Preserve any existing draft. Only the user's ordinary Send
uses `SubmitPrompt`; Ask itself never creates, dispatches or resolves a prompt.
TUI note UI is deferred by the owner.

MD-N4: `chariox.load_notes` loads list/read/reply/resolve tools on demand using
kbrowser's existing focus loader and epoch. Browser and notes tool loading are
independent; loading either capability never exposes the other. Only one authenticated local provider
run for the user's currently focused agent can discover or call them. Both
session and user-domain focus are checked on every call; focus revocation fences
results and commits. Remote/leased agent access remains out of scope with the
existing multidomain focus contract. A Room agent does not get notes merely by
Room membership. Terminal users can manage only their own notes.

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
