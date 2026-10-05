# First multidomain integration

MP-08 / MP-10 / MP-11: [user-domain access](MULTIDOMAIN_USER_DOMAIN_ACCESS.md)
amends this historical integration at reserved 432/78. Focus changes retain
existing task grants; window metadata exposes same-kernel reachability.

Branch `md/integration` combines OSS main `358491d66`, Apps queue
`b02d9b1a3` / `70a17a4f8` / `de553fcdd`, the three published kernel-browser
commits (`b9bdcc29d`, `66fb55a7a`, `8c3d6bafd`) and the three App-view commits
(`202bfdbd2`, `f491e54c3`, `04d19ce57`). All are replayed locally; publication,
review and merge remain the coordinator's job. Cloud prototype `b64dacb3` is
unchanged and off by default.

## One App instance, two user-domain hosts

`OpenUserAppView { installation_id }` still defaults to native rendering. Its
existing wire shape and frontend bundle response remain compatible with the
Cloud prototype. A terminal that cannot render natively requests
`OpenUserAppView { installation_id, host: "kernel_browser" }`. Explicit
`"client_native"` is also accepted. There is no automatic failure-based host
switch, Room inference, synthetic Session or caller-selected browser/CDP endpoint.

Both paths reserve the same owner-scoped ephemeral instance and register the
same call cancellation scope. Trust/asset verification, generation checks,
installation permission/Vault rules and human App actor are shared. Chromium
uses `KernelBrowserAppViewHost`, which delegates to lane kbrowser's existing
App-view service; it launches no additional browser. A trusted ephemeral instance
key gives each user-domain view its own target, even for the same installation;
Room requests omit it and keep one shared tab per installation. User instances
never adopt restored placeholders. The duplicate native-host
registry wrapper has been removed. The Room host implementation keeps its
existing controller behavior.

A hosted instance adds optional `browser: { tab_id, generation }` metadata for
clients to use protocol 417's transport-neutral screenshot/frame/input interface.
The kernel privately retains the CDP target; public App responses never expose
it. The browser is the same per-owner private kernel profile used for ordinary
user-domain tabs. There is no session dependency or display-backend selection.

One App-call drain per owner polls that controller. Exact target/installation,
instance owner and document identity bind each call to the shared App tool queue;
responses use the same controller and browser generation. User-domain calls have
human provenance and no Room/agent context. Old documents cancel their calls;
closing an instance cancels its call budget before attempting target close.
Successful target close also revokes its frame subscriptions; old polls fail.
Uninstall/update checks still use the installation generation. Updates require
explicit reopen when stale; this lane does not add automatic App restoration.

Detached RuntimeInteractions are still kernel-owned and projected through
`SubscribeUserAppViews`; trusted terminals answer through
`AnswerUserDomainInteraction`. The App page has no approval-answer or Vault
endpoint. Existing passkey/receipt ownership and one-use parameter binding apply.
Chromium is a rendering host, not an approval surface or runtime authority.

Browser MCP uses the shared retained-grant authority and on-demand loader.
Browser input/navigation/close from an agent is refused on a live App target: it cannot turn the human App page
bridge into agent authority. The proposed focused App MCP adapter must supply an
agent actor through the App queue; its names/selector are documented in
MULTIDOMAIN_APP_VIEWS.md and are not invented by this integration. Ordinary tab
control remains available over claimed resources after on-demand load.

## Ephemeral App lifecycle vs ordinary tabs

App views remain explicit-close ephemeral instances. Kernel restart restores no
instance/channel id. Browser/controller loss or explicit host stop also ends the
hosted instances; it does not regrant them to restored tabs. App tabs are excluded
from the host's durable ordinary-tab list. Generation-bound App drains/replies/
close cannot start a stopped browser or act on a replacement generation. A new
explicit open can start the host. Native views remain independent of browser stop.
Kernel shutdown revokes all instance scopes and prevents retained host handles
from launching another controller after shutdown.

Ordinary browser tabs retain lane kbrowser's URL/tab-id restoration policy and
private profile. Profile synchronization, Mac evidence, final display transport,
production frontend worker lifecycle and App agent-MCP rollout remain subsequent
owner/coordinator work. No Selkies/noVNC-specific work is introduced.

## Protocol history and merge-order requirements

The coherent order on this branch is **416 Apps → 417 kernel browser → 418 user
App views plus host binding**; relay stays **70**. The current shared local version
is **418** in Rust and TypeScript. Historical browser request/response snapshots
remain in `protocol_shapes/kernel_browser.rs` under their 417-origin test names,
asserting the current 418 constant. App snapshots remain in
`protocol_shapes/user_app_views.rs`, including the native-default baseline hash
and an additional host-selection/tab-reference/internal-close snapshot/hash.
All other protocol anchors now assert 418. Browser-only client minimum is 417;
user App views/host selection minimum is 418. No global Cloud minimum changes.

Coordinator renumbering if merge order changes:

- If Apps 416, then kernel-browser 417, then this combined App-view 418 tree land
  in this order, no renumbering is needed. Keep the historical browser 417 names
  and browser minimum, current 418 anchors and App minimum, relay 70.
- If App views land before the browser lane, the browser's new public shapes and
  the fallback extension need the next **coordinator-allocated** version after
  the then-current main; never replay a commit that changes 418 back to 417.
  Update both the browser minimum and fallback-dependent minimum to their actual
  introduction versions. An already landed native App 418 minimum stays 418.
- If the browser lane lands before Apps-on-main, the later Apps additions also
  need a monotonic, newly allocated local version. Recompute the later App-view/
  fallback allocation after that order is known; do not downgrade 417 to 416.
- For any renumbering, update `local/api/types.rs`, shared `kernel-types.ts`, all
  current-version Rust/TS protocol anchors, versioned snapshot/test names and
  origin notes where introductions change, `kernelBrowserMinimumProtocolVersion`,
  `userAppViewsMinimumProtocolVersion`, any fallback-specific/client prototype
  feature gate that depends on the changed introduction, drill receipts/version
  assertions and both multidomain docs. Keep existing wire snapshots/hashes;
  update a hash only when its actual serialized shape changes. Re-run both lane
  drills and this integration drill. Relay 70 changes only with a separately
  allocated relay shape change. The coordinator allocates all new numbers.

## Validation and reproduction

Run `apps/cli/scripts/user-app-view-protocol-drill.mjs` with Rust 1.88.0, slot-run,
four build jobs, a task-local CARGO_TARGET_DIR and external TMPDIR/evidence. It
covers native no-session signed fixture/channel/passkey, close cancellation,
Room regressions, protocol hashes and shared request/SDK paths. Also run focused
`kernel_browser` Rust suites and both controller JS test files.

Build the kernel lib-test binary under the same slot/memory budget, then run as a
normal Linux user with a sandbox-capable host and explicit Chromium:

```sh
CHARIOX_KERNEL_BROWSER_EXECUTABLE=/absolute/native/chromium \
node apps/cli/scripts/multidomain-host-drill.mjs \
  /absolute/kernel-lib-test-executable /absolute/external/evidence
```

The runner uses two isolated disposable roots and cleans them in finally. First
it runs lane kbrowser's ignored MD-4 drill: actual controller/Chromium, focused
MCP input, screenshot/frame, browser crash, stale references, kernel process
restart and last-tab close. Then it runs the ignored integrated App/browser drill:
no Session while opening/calling/approving/closing the signed App, actual page
`window.chariox.call`, human Tab/type/Enter App call visible in the accessibility
outline, real fixed ABI worker, synthetic passkey/one-use receipt,
foreign-owner/provider denials, independent same-App targets/close, then a focused dev-stub provider's
on-demand MCP tab in the same owner browser/profile. The App screenshot and
receipts/logs go to evidence; fixture workers and browser host are shut down.

This proves production kernel router/controller integration with a **test-only
fixed worker** and synthetic encrypted vault. It does not prove production App
worker sandbox execution, a live provider/model, real owner Vault, browser-client/
relay projection, Cloud deployment or Mac acceptance. Those scopes are not
implied by the screenshots. The Cloud headless prototype drill remains separate.


The flagged TUI text tier is documented in MULTIDOMAIN_TUI_APP_VIEWS.md.
Its runtime opt-in matches Cloud's build opt-in. The PTY component drill uses
production projection/LocalIpcClient with a synthetic peer; the native host
integration drill separately proves the actual kernel/Chromium boundary.
This branch is replayed on #843 head a5c8dbcf1 (including both review fixes).
Cloud's prototype is replayed on ee3a4d556 / G2 main 619486c69 and uses the
shared G2 passkey popup. The original protocol merge-order matrix still applies.
