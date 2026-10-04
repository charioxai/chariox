# MD-1: kernel browser outside slices

Protocol allocation: local 417. Base: OSS G2 `9334141d420f8a32393f206102c5b8b4a1b0b609`.
MD-1 is design, MD-2 host/browser and shared protocol, MD-3 focused runtime MCP,
MD-4 Linux drill and recovery. These are not MP acceptance claims.

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

A bounded supervisor restarts an exited browser on the next request; profile
storage survives. Durable tab records restore URLs under the same Chariox tab
IDs, with a new browser generation and document references. Old references and
screen subscriptions fail after restart. An internal blank target keeps headed Chromium alive when no user tabs are open;
it is excluded from user tab lists and persistence. Normal shutdown closes the browser
before killing remaining owned descendants. Controller stdin loss closes its
browser. Startup fails safely if another process owns the profile; never adopt
an unrelated user's Chrome or delete its locks.

## MD-3: authority and secrets

Human requests derive user identity from KernelCaller, not request arguments.
Only authenticated terminals may call the public host interface. MCP derives
user/agent identity from the admitted local provider run and checks the user's
current focus on discovery and every call. Focus changes revoke old access;
remote/leased runs are excluded. A small loader tool makes the capability known;
the browser operation tool is advertised only after on-demand loading.
No additional approval/grant path is introduced.

Ordinary input is not a Vault operation. No secret-reading or secret-insertion
endpoint is added. Text input into password/OTP fields is refused by this initial host adapter; structured
observations reuse controller redaction. Vault authorization, target binding,
protected-value tracking and screenshot suppression must be integrated before
claiming Vault support for host tabs. Browser profiles remain private kernel
state. Arbitrary JavaScript and CDP are internal implementation seams only.

## MD-2: interfaces for appviews and display lanes

`KernelBrowserHost::request(user_id, KernelBrowserCommand)` is the common
sessionless tab/control interface. `app_view(user_id, BrowserAppViewRequest)` is
kernel-internal and uses the SAME per-user controller, including App calls and
responses; appviews retains installation trust, CSP and bridge authority.
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
This does not close Mac streaming, client integration, Vault, multi-user security
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
The script starts disposable kernel router subprocesses with dev-stub MCP
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
