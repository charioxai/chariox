# MD-1: kernel browser outside slices

Protocol introduction: local 417. Integration current: 424/70 on OSS main
`358491d66` plus Apps-on-main. See MULTIDOMAIN_STACK.md and MULTIDOMAIN_INTEGRATION.md for merge-order
renumbering and the bound App host.
MD-1 is design, MD-2 host/browser and shared protocol, MD-3 focused runtime MCP,
MD-4 native drill and recovery, MD-5 shared Vault protection. These are not MP acceptance claims.

The home kernel owns a lazy browser host per authenticated user. No session,
Room, slice, container, grant, or private-window flag is involved. Room browser
paths continue unchanged. Profile selection is one module: private
`<kernel-state-root>/kernel-browser/<sha256(user-id)>/profile`, one per user on
this kernel. The state root comes from the existing private kernel state path
(default beneath `CHARIOX_HOME/kernels/<kernel-id>`). No cookie/profile synchronization or access from other kernels.

## MD-2: ownership and control

A kernel service owns the controller child; its host launcher owns Chromium and
starts it directly on Linux, with the renderer sandbox enabled. Root Chromium
and unsafe fallback flags are rejected. The launcher uses a private profile,
a dynamic loopback CDP port, and no provider credentials. Production is headed;
explicit headless mode exists for disposable drills. On macOS the same launcher
selects native Chromium/Chrome from Applications (or an explicit executable),
with the same separate profile. Windows is deferred.

The host needs Node 22+ and a native Chromium installation; shared controller
assets are embedded in the kernel and materialized under its private state.
The existing BrowserCdpClient and controller stdio server handle reconciliation,
document identity, navigation, snapshots and App interception. A host adapter
adds process ownership, unattached tab identity and screen/input primitives;
it does not recreate CDP. Public commands contain stable tab IDs and generation
checks, never debugger URLs, raw CDP, JavaScript, cookies or filesystem paths.
Mutations serialize per user's browser. Control I/O runs off the async router.
Failed mutations are returned, never replayed automatically.
Only observational reconciliation retries a stale-document navigation race,
with three bounded reads; it never recreates a tab or repeats input.

An explicit start/open recovers an exited browser; observation reads never
start/restart it. Profile
storage survives. Durable tab records restore URLs under the same Chariox tab
IDs, with a new browser generation and document references. Old references and
screen subscriptions fail after restart. An internal blank target keeps headed Chromium alive when no user tabs are open;
it is excluded from user tab lists and persistence. Normal shutdown closes the browser
before killing remaining owned descendants. Controller stdin loss closes its
browser. Startup fails safely if another process owns the profile; never adopt
an unrelated user's Chrome or delete its locks.

## MD-3: authority and secrets

Human requests derive user identity from KernelCaller, not request arguments.
Only authenticated terminals may call the public host interface. MP-08/MP-11:
MCP derives user/agent identity from the admitted local provider run. Focus
claims resources; active tasks and pending wakes retain their exact grant after
focus changes. MP-11: retained grants allow the same input/mutations on granted
resources as focus, including text, keys, Tab and clicks. Explicit start/open
is allowed; unrelated resource claims still need focus. No observation read
starts/recovers the controller or browser.
A small loader advertises tools on demand. See the authoritative
[user-domain access amendment](MULTIDOMAIN_USER_DOMAIN_ACCESS.md) (432/78).
Remote/leased calls name both kernels and request focus on the window's kernel;
there is no cross-kernel control.

MD-3 local socket admission carries a nonserialized connection cancellation
lifetime through router, Vault barrier and backend waits. Disconnect cancels it
before releasing actor presence. Ledger registration checks it under the same
lock as disconnect, so queued physical input cannot recreate a departed actor.
Retiring or explicitly revoking an agent removes its actor and pointer slots;
bounded terminal action history remains. A failed CDP input attempt fences the
browser to clear uncertain held keys/buttons before another actor can dispatch.

MD-5 browser receipts also bind the current observation-protection revision.
Replay waits at the normal Vault capture barrier, requires a readable registry
and rechecks revision and connection lifetime. A changed or fenced policy refuses
old text/pixels while preserving the existing mutation receipt. Clients must
reconcile uncertain mutations; safe reads use fresh command IDs after reconnect
because the new socket has a different authenticated actor.

MD-3 browser retry receipts include the authenticated terminal caller in their
fingerprint. Terminal admission runs before replay lookup; another user cannot
receive a cached observation. Browser receipts remain memory-only, preserving
in-process mutation deduplication without persisting screenshot/frame payloads.
MD-3 local terminals have a kernel-generated identity per admitted connection.
Observations and takeover belong to that connection; another terminal cannot
refresh its input receipt or release its control. Closing that connection releases
its control, withdraws pending takeover, cancels active input and removes its live
actor/pointer presence. Retries on the live connection
retain the identity. Reconnecting with an old browser command ID fails closed
on the caller fingerprint mismatch; clients must observe again and explicitly
submit a new command, never automatically replay that mutation under a new ID.
Native URLs outside HTTP(S)/about:blank restore as about:blank; reconciliation
keeps at most 128 durable tabs, prioritizes existing identities, closes excess
native targets and truncates oversized legacy registries.

MP-08/MP-11: each MCP call retains its grant admission through async waits and
the controller operation. Idle expiry, explicit revoke, agent destruction/session
end/placement move or loss of provider-run authority cancels pending input.
MP-11: ordinary input keeps grant/run authority across focus changes. Vault
fills keep their additional live-focus authority through the shared stdio cancellation path. A timed-out cancellation fences the
controller before settlement. Input checks cancellation and the observed CDP
loader before every physical event. If input is interrupted between events, the
host stops its owned Chromium to clear held keys/buttons; an explicit start/open
recovers with a new generation. This cannot undo input already delivered before
revocation.

The private MCP browser envelope takes `document_id` beside `command` for input.
State/snapshot and MCP screenshot supply that identity. Missing or stale
bindings fail before input; observing another tab or another caller's refreshed
state cannot rebind an old request. Protocol-417 human input uses a receipt scoped
to the terminal caller until the public contract can carry explicit document
identity. That protects input across replacement documents; explicit concurrent
frame/document binding for public display clients requires a coordinator
protocol allocation and client integration. It is not claimed accepted here.


Ordinary input is not a Vault operation. MP-08 / MP-11: ordinary text insertion
and all text-producing keys into
password/OTP or focused frame targets are refused before dispatch for both
focused and retained grants; those targets require the Vault path. MD-5 adds
`chariox.kernel_browser_paste_secret` after on-demand loading: an opaque Vault
credential handle plus observed tab/generation/document/node reference, never
secret text. Only the host owner (local or configured Cloud identity) can use
the host Vault. Collaborators keep separate browser profiles. The existing Room
Vault service, unlock RuntimeInteraction and lifecycle read lock authorize the
observed frame URL; focus (or an A06 sudo window), metadata and the editable
password target are checked again after waits. The existing controller Fill enforces document/URL/masking
and native-form submission. `submit=false` is the default. Browser profiles remain private kernel
state. Arbitrary JavaScript and CDP are internal implementation seams only.

## MD-2: interfaces for appviews and display lanes

`KernelRuntimeState::kernel_browser_request(user_id, KernelBrowserCommand)` is
the common sessionless tab/control interface. `kernel_browser_app_view(user_id,
BrowserAppViewRequest)` is the kernel-internal App seam and uses the SAME
per-user controller, including App calls and responses; appviews retains installation trust, CSP and bridge authority.
App views must not use a synthetic Room or independently launch Chromium.

The display seam is a tab frame source plus input sink: screenshot, subscribe,
poll latest bounded frame, unsubscribe, and typed pointer/key/text input. CDP
screencast is an initial source adapter. Subscriptions carry user, tab and browser
generation, expire when idle, and retain at most the latest frame; transport may
replace this source without replacing tab or input authority. No Selkies/noVNC,
encoder, relay authority, Cloud proxy, or client rendering choice is added.

## MD-4: validation and open questions

Unit coverage: user profile separation, URL/input validation, focus revocation,
protocol-417 serialization and stale generation rejection. Linux drill: explicit
disposable state, local fixture, sessionless open/navigate, screenshot, focused
MCP input, subscription, browser crash and kernel restart, exact owned cleanup.
This does not close native Mac execution, client integration, multi-user security
review, or any MP ledger item.

Owner questions: final display source/transport and Mac display acceptance;
future profile synchronization (default remains local); focus conflict policy
when two terminals select different agents for one user. Initial focus policy
is last explicit kernel focus selection; closing a terminal does not regrant an
older agent. Coordinator must pair this seam with appviews/display and arrange
independent review and Mac evidence.

MD-4 Linux replay: build `cargo test -p chariox-kernel --lib --no-run` under the
allocated compile lock, then run the resulting test executable with
`--ignored --exact runtime::router::tests::kernel_browser::kernel_browser_linux_integration_drill`.
Use a normal Unix user, a clean environment, `CHARIOX_MD4_DRILL_ROOT=<disposable-root>`,
`CHARIOX_HOME=<root>/home/chariox`, `HOME=<root>/home`, a short private `TMPDIR`,
and `CHARIOX_KERNEL_BROWSER_EXECUTABLE=<native-Chromium>`,
`CHARIOX_KERNEL_BROWSER_HEADLESS=1`. The host needs Chromium dependencies, fonts,
and an OS policy permitting its sandbox. The drill boots the production kernel
router with a dev-stub provider binding and restarts it in a second process; it does not execute a model or validate
client/socket/relay projections. Keep `screenshot.png` and the run receipt as
external evidence, then remove the exact owned state.

## MD-2 / MD-4: native macOS replay

Host policy lives in `kernel-browser-linux.mjs` and `kernel-browser-macos.mjs`.
macOS discovers installed Chromium, Chrome or Chrome for Testing under system
and user Applications; `CHARIOX_KERNEL_BROWSER_EXECUTABLE` can select an absolute
pinned build. Launch the bundle executable directly, with the same private
per-user profile, dynamic loopback CDP and sandbox. No X11/Xvfb, screen-recording
permission, Keychain export, mock keychain or default Chrome profile is used.
CDP drives input and the existing screenshot/screencast frame source. Crash and
kernel restart use the common supervisor and stable tab records.

On the Mac, compile the native kernel test artifact under an external Cargo
target directory (`cargo test -p chariox-kernel --lib --no-run`). Run:
`node apps/kernel/slice-linux-docker/kernel-browser-macos-drill.mjs /absolute/kernel-tests /absolute/external/evidence`.
The script is standalone (no sibling JavaScript imports) and starts disposable kernel router subprocesses with dev-stub MCP
identity, a local fixture and a private HOME/CHARIOX_HOME, checks screenshot,
MCP click/type, CDP frames, Chrome SIGKILL recovery, kernel restart and close.
It never connects to a daemon listener (including :44240). This is native
kernel/router evidence, not a provider model or Web/TUI transport acceptance.
It records resource samples and binary hash and removes exact owned state only
after checking process cleanup. Native Mac execution remains coordinator-owned.

Chromium's [profile contract](https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md)
allows the explicit separate user-data directory. Its [POSIX singleton implementation](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/browser/process_singleton_posix.cc)
uses a hostname/PID symlink on current macOS as well as Linux; the host refuses
a live owner before launch and leaves stale-lock recovery to Chromium.

## MD-5: shared Vault observation protection

The Room `RoomSecretObservations` implementation also stores user-domain
protection in a separate namespace under `kernel-browser/observations`, sealed
to the ordinary kernel runtime identity. Active and retired values survive
browser/controller/kernel restart. Vault deletion, rotation and credential
metadata changes retire matching dormant user profiles through the same Vault
lifecycle lock. Retired values still scrub prior page echoes. Missing/corrupt
provenance fences observations; shutdown remains available. No human clearance
or reset can erase live provenance. This state is separate from Room stores and
never cleared by closing a tab or stopping/restarting Chromium.

A scope input/capture barrier surrounds requests; private controller policy is
seeded before every operation. Shared CDP redaction runs before snapshot
compaction, then the kernel scrubs reply/error metadata. URLs echoing protected
values are scrubbed in observations and persisted as about:blank for restart,
never saved as plaintext secret-bearing restore URLs. No secret-reading MCP
endpoint exists. The obsolete unprotected host request/App methods are removed;
appviews must use the unchanged async `kernel_browser_app_view` seam.

PNG screenshots reuse the Room's trusted CDP region locator, including field,
plaintext echo, iframe and opaque-media masks. A small bounded native PNG
adapter applies the masks after capture. Content capture binds to the emulated
CDP viewport rather than desktop window bounds; Room desktop binding stays
unchanged. Unsupported/racing layouts use a
whole-frame opaque mask. Screencast activity triggers that same protected
screenshot path with one in-flight capture, a latest-frame bound and a 5Hz cap.
Subscribers on one tab share one CDP source and acknowledgment. Raw protected
screencast pixels never leave the controller. An opaque fallback is available
immediately while a bound frame is pending, even without a repaint. This is a conservative frame source,
not the display lane's final transport. The display lane must preserve the
input/capture barrier, policy revision and protected pixel path for any new
frame source; it must not consume raw CDP frames after secret insertion.

MD-5 tests cover owner-only Vault admission, no secret arguments/results,
focus revocation, dormant-profile retirement, sealed restart scrubbing, background
content masks, pixel replacement and full-mask fallback. The native kernel replay
also uses a disposable synthetic Vault/password fixture, deliberately echoes
secret input, captures protected pixels, retires the credential and checks
scrubbing/frames and process recovery. These checks do not establish a real
provider/public-site login, Web/TUI projection or native Mac execution.

## MD-3 / MD-4: shared actors and document-bound display adapter

The host attaches the existing `EnvironmentActionLedger`, `TabRegistry`,
`EnvironmentActor`, pointer, ownership and takeover types to a per-user
sessionless model (`kernel_browser_actors.rs`). It creates no Room or session.
Authenticated terminal identity and admitted focused-agent identity select the
actor; clients cannot supply an actor or user. Input, navigation, tab open/close,
Vault input and stop use that ledger. Entries retain input counts/coordinates,
never typed text, URLs or Vault payloads. Hot/cold history is bounded to 256
entries. The shared Room model's retention policy is unchanged.

Human takeover cancels a running same-tab agent action through the existing stdio
cancellation path. Pending takeover fences subsequent mutations immediately;
ownership is granted only once that action settles. Release requires the same
terminal actor. Browser recovery preserves deliberate human ownership of stable
tabs and invalidates old action/generation references. Focus/run revocation still
uses the existing grant admission; release does not restore old
focus or grant access to another agent. Observations remain available during
takeover. Appviews retains its own admission/attribution above the unchanged App
seam. Actor projections are live kernel state. A full kernel restart releases control;
a human or agent must request it again. This is the owner decision recorded by
the coordinator on 2026-10-05 and may be revisited. Browser-process recovery
continues to preserve deliberate human takeover.

Internal API for the display lane:

- `KernelRuntimeState::kernel_browser_display_request(&KernelCommand,
  KernelBrowserDisplayRequest) -> Result<Value, DaemonError>`.
- `Capture { tab_id, generation }`: protected PNG with the existing image fields
  plus `document_id` from the captured source.
- `Subscribe { tab_id, generation }`: existing subscription identity; poll and
  unsubscribe through the existing kernel-browser command path. Bound frames
  include their captured `document_id`; opaque pending frames omit it and cannot
  authorize input.
- `Input { binding: KernelBrowserDocumentBinding { tab_id, generation,
  document_id }, input: KernelBrowserInput }`: explicit document-bound input,
  through the same protected host path. Replacement documents are rejected.
- `Takeover { tab_id, generation }`, `Release { tab_id, generation }`, `Actors`:
  shared takeover outcome, owner-only release, and current actor/pointer/action
  projection. Projection targets use stable host tab IDs.

These Rust types have no serde implementation. The coordinator/display lane
owns the new serialized public adapter and protocol allocation; this lane must
not take an unallocated version. The existing protocol-417 screenshot/stream
adapter keeps its original shape and caller-scoped human observation receipts.
The App seam and ordinary screenshot/subscribe/poll/input methods stay stable.

A bound capture checks the document before and after capture. Raw asynchronous
CDP screencast pixels cannot prove document identity, so a bound subscription
uses those events only to trigger the existing protected screenshot source,
with one capture in flight, latest-frame storage, and the existing 5 Hz bound.
This is a verification source, not a final display transport. The display lane
may replace it while retaining Vault masking, source document binding, the
capture/input barrier and kernel actor authority. After navigation, clients
refresh state and subscribe to the new document; old frame input is rejected.
Its public adapter must also preserve caller-scoped retry deduplication and
recheck terminal admission before serving cached responses.

MD-4 validation adds native takeover, agent fencing, owner human input, release,
agent resumption and bound frame capture to the disposable Linux/Mac replay.
The replay and focused API regression use default Rust thread stacks; production
App-copy/control future boundaries are boxed to prevent the discovered overflow.
Native Mac execution and the public display/client adapter remain external gates.
No MP acceptance item closes from these checks.
The first integration uses the App seam for explicit user-domain Chromium fallback
while keeping native rendering default. Generation-bound App requests cannot
start a stopped browser; App instances/tabs are ephemeral and excluded from
ordinary-tab restoration. Focused browser input/navigation/close is refused on live App tabs to
preserve the human/agent App actor boundary. No separate Chromium or Room is
created. App trust, tool queue and detached approvals remain in the App runtime.

MD-stack integration: the unreleased feature allocation is folded into local
protocol 427 (relay peer 74). This union and its shape/hash guards supersede the
per-feature versions described during development above.

## MP-08 / MP-10 / MP-11: agent tabs visible (PR 937 protocol 481)

The combined unmerged PR 937 uses its coordinator allocation 481 (OSS main 472).
Kernel browser state carries every admitted viewer's tab inventory, including
agent-created tabs. Each tab has `opened_by` (actor ID, human/agent kind, display
label; null for previously discovered tabs whose opener is unknown). Names come
from the authenticated agent record. `agent_activity` identifies the latest
admitted agent mutation with a generation-local monotonic sequence and tab ID.
The same retained-grant scope bounds both inventory and activity metadata.
Closing a tab retires its metadata; a browser generation change resets activity.

The web tab strip projects this inventory and defaults to following agent
activity. Follow-off retains the selected view and shows a notice. Selecting or
following a tab attaches protected observation only: it never activates the
native tab, takes over input, or changes grants. Watched surfaces reject input
across mirror, video and image adapters, including queued gestures from retired
control bindings. Vault capture protection remains on the shared kernel path.
`/access tabs` lists the same inventory with opener, title and URL in the TUI.

The owner decision of 2026-10-09 15:20 UTC adds acceptance scenario
`apps-agent-model/TABS-VISIBLE`: official Codex opens and fills a real public
form while hosted web viewers watch at DPR 1 and 2; screenshots show discovery
and follow, follow-off shows notice, and watching preserves input ownership.
Source checks are supplementary; hosted real-site, shaped-network and stability
evidence is required before this row can pass. A separately published PR needs
a fresh coordinator allocation rather than treating 481 as a new allocation.
