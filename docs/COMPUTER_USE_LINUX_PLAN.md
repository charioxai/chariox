# Linux Computer use: MP-08 / MP-10 / MP-11 Phase A

Phase A source: OSS main `e325afa580d81954e2c179757fc53fa02ed2a2b3`
(local435 / relay73). Work branch: `computer/unicode-input-ocr-main`.
This is an audit and proposed design, not Phase B implementation or acceptance.

## MP-08 / MP-10 / MP-11: reconcile #846

Compared #846 `0c4e848eb9997741cbde03f4c20f5de502a0794e` with the
specified main, including all seven commits since G2. There is no remaining
runtime or test patch to replay on this main. Of its 99 touched paths, 18 are
byte-identical, including the OCR implementation/tests, keyboard overlay tests,
physical fixture, image receipt tests, input action policy, hold adapter, hold
tests and session action/model changes.

Other paths retain #846's behavior with subsequent main changes: protocol
snapshots use the union versions; IPC admission was split into responsibility
modules; Computer execution gained live authorization checks; Browser artifacts
added commands/types; keyboard text rejects inherited accelerator bindings and
key chords translate Enter to X11 Return. Copying the old files would regress
these changes. The Dockerfile still installs German OCR and runs native tests.
The final #846 physical fault environment fix is already byte-identical.

Retained comparison: external `culinux/reconcile.json` and
`culinux/reconcile-current.diff`. Focused current-source checks: 131 Node tests
and 42 native helper tests pass, zero skips. These establish fixture/policy
seams, not live provider, Web/TUI or ordinary/managed acceptance. No protocol
number was allocated or changed by this lane. The coordinator can close the
obsolete #846 branch after independently checking this reconciliation.

## MP-08 / MP-11: current end-to-end path

Paths below are relative to this frozen OSS source. Cloud source and a live
Web/TUI session were not inspected in this lane; their acceptance is unclaimed.

| Seam | Current implementation |
| --- | --- |
| Tool exposure | `apps/kernel/src/transport/runtime_tools/slice_tools.rs` defines screenshot, status, OCR/find-text, mouse, keyboard and clipboard tools. `runtime/state/tool_dispatch.rs` advertises them for the Room Environment or a slice kernel, rather than for an arbitrary reachable machine. |
| Official providers | `apps/kernel/src/provider/{codex,opencode,claude}/mcp_config.rs` binds the shared authenticated runtime MCP endpoint into each official harness. `transport/mcp_server.rs` converts bounded screenshot bytes into native MCP image content and removes the base64 copy from JSON. Exposure and a captured file do not prove model perception. |
| Home admission | `runtime/state/browser_controller_action_execution_runtime_state.rs` checks agent/session membership, live authority, Environment generation and canonical geometry, creates an attributed Computer Action and reserves desktop input. Human requests use `human_environment_action_runtime_state.rs`. Both execute through `room_browser_controller.rs`; takeover/cancellation release the same target. |
| Placement/transport | `runtime/state/room_environment_placement.rs` binds the Room to one slice. `room_browser_controller.rs` routes input to that worker using existing peer requests. `room_computer_observation.rs` checks the bound slice, remote response identity, canonical viewport and observation policy. Room tools can reach the Environment from a home/leased agent; the legacy unbound path still requires execution inside a slice. |
| Native desktop | `apps/kernel/slice-linux-docker/docker/start-runtime.sh` and `slice-screen.sh` own headed desktop startup/status. The latter selects Xorg or Xvfb, starts Openbox/Chromium and invokes native helpers. `slice-keyboard.py` handles safe Unicode overlays and bounded holds; xdotool handles pointer/chords; xclip handles clipboard; `slice-text-finder.py` runs bounded Tesseract matching with NFC queries and distinct OCR targets. Inputs preserve the focused desktop app. |
| Observation/secret safety | `runtime/state/tool_dispatch/slice/controller_computer_observation.rs` uses opaque Room artifact IDs, rejecting caller image paths. `slice-observation-mask.py`, `slice-secret-target.py`, `room_secret_observation.rs` and capture barriers protect text/pixels and clipboard. Unknown protection withholds observation. Vault insertion keeps its separate authorization. |
| Viewer | `apps/kernel/src/slice/{local_docker,store,display}.rs`, `runtime/state/room_display.rs`, `transport/secure_display.rs` and shared display requests project the headed slice endpoint and lifecycle. Existing slices still expose Selkies/noVNC. They are baseline dependencies, not the target for new work. |
| TUI | `apps/cli/src/room-command-handler.ts` exposes status, view, screenshot, action history, takeover/release and lifecycle. `room-environment-activity-controller.ts` replays cursor-bound actor/mode/target/outcome notices. Local and remote clients consume the same kernel requests; these source paths are not physical TUI validation. |
| Web contract | `packages/kernel-client/src/{kernel-types-environment,ipc-room-environment-requests,ipc-slice-requests}.ts` provides shared state/action/display requests. `docs/M22_VIEW_FEATURE_A_PLUS_PLAN.md` describes View display, selected-agent prompt, shared transcript and lifecycle stores. Current Cloud renderer/mode switching requires a paired Cloud audit and live replay. |

Browser and Computer share Room identity, canonical viewport, Action ledger,
input ownership, protection and persistence. Action requests carry
`EnvironmentMode::Browser` or `Computer` (`session/room_environment/action.rs`);
the snapshot is not a separate desktop created by a mode toggle. The current
provider tool dispatch and Computer observation binding remain slice-dependent.
Installing Xvfb on the host alone does not enable host Computer tools.

## MP-08 / MP-10: acceptance cells and local results

Evidence root: `/root/.codex/evidence/browser-resume-20260930/culinux/`.
No cell below gains full acceptance from these runs.

| Cell | Phase A result and still-required proof |
| --- | --- |
| `PROVIDER-CODEX-COMPUTER-FALLBACK` | NOT_RUN; shared image-admission preflight fails before account import/provider launch. Need an exact-source slice runtime, product-linked account, paid official-harness screenshot/input task and independently checked effect. |
| `PROVIDER-OPENCODE-COMPUTER-FALLBACK` | Same prerequisite failure; no OpenCode provider turn or image delivery observed. |
| `PROVIDER-CLAUDE-COMPUTER-FALLBACK` | Same prerequisite failure; no Claude provider turn or image delivery observed. |
| Deterministic computer-use functional task subset | PASS_HELPER_ONLY: 18/18 physical rows on Xvfb and 18/18 on Xorg. Native input drives a first-party Chromium page and Mousepad. Source suites also pass Node131/native42 with zero skips. Stream-loss exactly-once behavior, canonical resize across viewers, human transport, provider perception and full Room routing remain unestablished. |
| Mode switching | NOT_RUN live. Must switch Browser↔Computer on the same host/Room surface, retain profile/tab identity and pending input ownership, and verify Web/local/remote TUI reconnect. A mode field on an Action is narrower evidence. |
| Tool traces | PASS source receipt assertions only; NOT_RUN live client conjunction. Must join provider turn, native image/effect, kernel Action actor/target/outcome and Web/local/remote TUI traces, including cancellation and replay. |
| OSWorld | BLOCKED: no `/dev/kvm` on this builder; needs the approved KVM host and benchmark harness. Neither helper tests nor a deterministic subset count as OSWorld. Benchmark rounds are separate from functional merge gates. |

The two physical runs use frozen runtime source `e325afa580`, a clean tree,
mounted current helpers and an immutable lane-created dependency image based
on G2. Mousepad/German OCR were added using that image's package sources. This
is not a current complete slice image or signed-runtime proof. `report.json`
records image/helper hashes, native versions, commands/exits, geometry,
trusted page acknowledgements, public screenshots, five-second resource samples
and exact-container cleanup. No `--test-binary` was supplied: the optional
physical kernel fatal-child reset seam was not executed. Helper reset/hold
checks must not be relabelled as that kernel test.

The provider preflight calls the runner's actual `validatePrebuiltSliceImage`
policy. The available runtime-labelled images have no matching source revision;
G2 also advertises relay70 versus this source's relay73. The first failure is
`prebuilt slice image must contain the exact current runtime source`, not an
authentication failure. Only allowlisted image identity labels were read;
no account credentials were inspected or provider processes started. Source
provider fixtures pass, but all three paid cells remain NOT_RUN. A newly built
matching runtime plus its provenance is a prerequisite for their replay.

## MP-08 / MP-11: proposed Linux host Computer mode

The separately inspected #900 successor is
`6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1` (local443 / relay86).
It was fetched to a read-only reference branch; none of its runtime changes
were imported or tested here. Its `docs/MULTIDOMAIN_USER_DOMAIN_ACCESS.md`
(#880/#884 access lineage) is authoritative for user-domain access. Its
`MULTIDOMAIN_KERNEL_BROWSER.md` owns one browser per authenticated user/kernel,
independently of Rooms; `MULTIDOMAIN_KERNEL_BROWSER_DISPLAY.md` describes the
flagged protected display transport. Earlier Room-only and Selkies direction
in the frozen plan is superseded by the owner's multidomain direction.

Start with one kernel-owned virtual X11 desktop per user/kernel: Xvfb, a minimal
WM (Openbox initially), sandboxed headed Chromium and explicitly launched
graphical apps. The kernel browser and Computer tools bind to this same display
and browser profile. #900's current host launcher starts Chromium on its launch
environment; it does not yet implement this owned virtual-display lifecycle.
That lifecycle is proposed work, not an existing capability.

Keep lifecycle in a named Linux desktop service, native operations in a desktop
adapter, and admission in the existing user-domain grant/actor services. Extract
reusable native input/observation operations from slice helpers without copying
their slice startup, viewer policy or `-ac` fixture configuration. Keep tool
semantics (screenshot, OCR/text targets, mouse, keyboard/holds and clipboard)
shared below providers. A placement adapter selects a slice desktop or this
host desktop; it must not silently reinterpret an existing slice tool as the
host. The slice Room authority remains available for its existing topology.
In particular, `slice-screen.sh:stop_process_pattern` uses name-based `pkill`
for container desktop lifecycle; never reuse that implementation on the host.
The host service must use verified owned process identities and signal guards.

Agent targeting starts with a bounded AT-SPI accessibility snapshot and stable
target handles, the structured desktop equivalent of Browser DOM references.
[AT-SPI](https://gnome.pages.gitlab.gnome.org/at-spi2-core/devel-docs/architecture.html)
exposes application accessibility over D-Bus. Own a desktop session bus and
accessibility service, scope enumeration to launched windows/apps, and bind
handles to surface/generation, app/window identity and tree revision. Return
protected role/name/state/bounds/actions, never unrestricted password text or
the raw bus. Revalidate handles and grant/run authority before accessibility
actions; stale targets require rediscovery. Actions use the same arbiter,
attribution and App human-channel restrictions as physical input. Missing or
incomplete accessibility falls back to protected OCR, then exact screenshot
targets. Agents receive on-demand snapshots/screenshots and never depend on
video decoding, stream timing or whether a human viewer is attached. AT-SPI
coverage and secret masking need new live fixtures; none was tested in Phase A.

Use a private Xauthority cookie, display socket with no TCP listener, private
runtime directory and explicit display/geometry binding; launch the user browser
as a non-root user with renderer sandboxing. Do not adopt the logged-in desktop
or inherit arbitrary DISPLAY/XAUTHORITY into providers. On Path 1, use the same
service-user launch behavior as an ordinary kernel. This desktop is a controlled
execution target, not a new security sandbox: same-UID programs and a root-capable
Path-1 agent are not isolated by Xauthority. Retain MP-11 security review of
sandbox, credentials, observation protection, admission and owned signals.

User-domain Computer access claims an explicit desktop resource through the
existing capability loader/focus grant. Retained holders keep the same operations
on granted resources across focus changes; busy turns/tools/subagents retain
access, while fully idle holders enter the existing expiry window. Explicit
revoke, expiry, session end, destruction and placement retirement cancel the
grant epoch and active input. No cross-kernel user-domain control is added:
remote agents receive the existing actionable kernel/focus refusal. A Room's
membership never becomes an implicit grant to the user's host desktop.

Serialize native desktop mutations with browser mutations that change physical
focus, keys or pointer on this display. Parallel structured reads can remain
concurrent; do not let a tab-level Browser lock race a desktop Computer action.
Human takeover uses the existing actor/input arbiter and releases held native
keys/buttons before granting the target. Every input binds surface, desktop
generation, viewport revision and grant/run epoch; browser-targeted actions
also bind tab/document identity. Recheck through waits and before each event.
Uncertain input failure resets/fences only the owned display process tree and
fails the Action without replay. Reads never implicitly start/recover a stopped
desktop. Explicit start/open creates a fresh generation; it never adopts stale
grants, artifacts or viewer receipts.

Reuse multidomain display negotiation, encrypted events, credit/backpressure,
exact repair, actor overlay, takeover and client presenters. The coordinator's
binding review directs reuse of lane display's `md/display-perf` (#893):
XShm/XDamage capture from the kernel-owned Xvfb, motion video plus exact settle,
with the whole desktop as source. Do not create a separate Computer streamer.
The inspected #900 receipt above describes its earlier protected CDP tab path,
not proof of that optimized desktop source. Map browser protection to desktop
coordinates and withhold/mask the full frame whenever non-browser coverage,
geometry, focus or policy is uncertain. OCR and native MCP images consume the
same protected frame; raw pixels never enter encoding, traces or model delivery.
Computer clipboard uses the same observation/Vault policy. Do not add a
Selkies/noVNC endpoint or a second relay/control authority.

Design the following inside that shared pipeline with the display lane:

- Copy-rect/scroll detection sends a region move only against an acknowledged
  exact frame base; generation/sequence/protection changes invalidate it and
  force exact repair. Masking must remain correct after every region move.
- XFixes cursor shape/position travels separately from desktop pixels, bound
  to surface/geometry and the kernel actor model; do not burn cursors into video.
- Classify changed text/UI regions for exact lossless tiles, true motion for
  H.264. Classify only already-protected pixels and repair settled regions
  exactly; codec/classifier policy remains in the existing display pipeline.
- Stream/encode only while at least one authorized human views. Stop capture
  polling/encoding when the last viewer leaves; agent snapshots remain available
  on demand without reactivating video. Slow viewers retain bounded credit and
  cannot block input or another viewer.

Acceptance targets from the coordinator: at most one CPU core at 1080p software,
under 50 ms input-to-visible effect, exact settled pixels, and superiority to
the retained Selkies CPU/GPU baseline. Measure identical geometry/content/rate,
publish percentile/latency endpoint and CPU accounting, and report software and
GPU results separately. Require moving content, text scroll, cursor, idle,
slow-viewer and reconnect cases. Targets are unproven here; the physical helper
fixtures perform no stream/performance comparison. Reuse display-lane baseline
evidence rather than implementing Selkies-specific changes.

Browser↔Computer switches the selected surface projection inside one Environment;
it must preserve browser tabs/profile and use explicit transform/viewport
revisions rather than scaling coordinate guesses. Mode switching neither creates
another browser nor grants new resources. Applications/documents persist in
normal kernel-user storage outside source checkouts. A display process restart
does not promise restoration of arbitrary application RAM; slice snapshots and
host profile/document persistence remain distinct lifecycle contracts.

## MP-08 / MP-11: real host desktop scope

X11 attachment could reuse XTEST/xdotool and root-window capture, but would expose
unrelated windows, password dialogs and clipboard state. Treat attachment to the
human's real desktop as a later explicit grant with separate protection and
multi-monitor/scale validation. The first implementation controls only programs
launched on the kernel-owned display. [Xvfb](https://www.x.org/releases/X11R7.6/doc/man/man1/Xvfb.1.xhtml)
provides an X server without a physical display, suitable for that server path.

For later real-session control, X11 uses XShm capture and XTEST input. Wayland
uses a supported ScreenCast/RemoteDesktop portal with PipeWire/libei rather
than extending XTEST to native Wayland surfaces. Omarchy/Hyprland need a compositor-supported capture
and input adapter; existing X11 tools do not establish native Wayland control.
Xwayland alone cannot be treated as authority over native compositor surfaces.
Probe the actual compositor/backend rather than claiming universal Hyprland
support. The current [Hyprland portal declaration](https://github.com/hyprwm/xdg-desktop-portal-hyprland/blob/master/hyprland.portal)
lists ScreenCast and InputCapture, but not RemoteDesktop; InputCapture is not
proof of remote input injection. Omarchy inherits the installed compositor and
portal combination's limits. [RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html)
and [ScreenCast](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.ScreenCast.html)
define portal sessions, selected devices, consent and stream coordinates; backend
support must be proven before choosing that path. A private Xvfb desktop works
independently of the host's compositor and gives deterministic focus/coordinates
without requiring control of the user's login session. Mac follows Linux
acceptance with its native display/permission adapter; it is not Phase A work.

## MP-08 / MP-10: exact proof matrix before OSWorld rental

Coordinator review makes this a prerequisite even though Hetzner approved the
OSWorld machine. Run every row below on the optimized whole-desktop pipeline,
with each terminal column independently verified, plus the combined-client run.
Each cell ID is the row prefix plus `W`, `L`, `R` or `J` (24 cells).
All slice cells are NOT_RUN in Phase A; all host cells are PLANNED.

| Provider × placement / cell prefix | Web (`W`) | Local TUI (`L`) | Remote TUI (`R`) | Web + both TUIs (`J`) |
| --- | --- | --- | --- | --- |
| Codex slice / `CU-CODEX-SLICE-` | required | required | required | required |
| OpenCode slice / `CU-OPENCODE-SLICE-` | required | required | required | required |
| Claude slice / `CU-CLAUDE-SLICE-` | required | required | required | required |
| Codex kernel-host virtual display / `CU-CODEX-HOST-` | required | required | required | required |
| OpenCode kernel-host virtual display / `CU-OPENCODE-HOST-` | required | required | required | required |
| Claude kernel-host virtual display / `CU-CLAUDE-HOST-` | required | required | required | required |

Each cell must run the official provider with a product-linked profile and
complete the same public-fixture task set:

1. Observe an AT-SPI tree, target a graphical editor, enter/select/copy Unicode
   text, use shortcuts, save/edit a document and verify its bytes independently.
   Assert native focus, stale accessibility-target rejection and action receipts.
2. Deliberately remove accessibility from a fixture region. Use OCR fallback
   and on-demand exact MCP screenshot bytes to act on randomized visual targets;
   retain delivered-image hashes, provider turn identity and independently checked
   native effects. Repeat blinded targets; no shell/CDP or file-reading shortcut.
3. Switch Browser↔Computer without changing the Chromium profile/tab registry,
   exercise browser chrome and the non-browser app, then reconnect that terminal
   to the same surface/generation and authoritative provider history.
4. Observe optimized human display pixels, separate cursor and exact settle;
   compare public source/viewer pixels. Verify traces identify the same actor,
   surface, action and effect; TUI uses screenshots/view URL plus trace projection.
   TUI columns include an authorized human display viewer for stream verification.
5. Take over during bounded input, release holds, revoke/expire access and reject
   a stale input/frame. Verify no replay/duplicate provider run. In combined `J`
   cells prompt from TUI, observe/control in Web, and replay both TUIs' notices.
6. Detach the last human viewer: encoding/capture polling stops while the agent
   still receives AT-SPI/OCR/exact screenshots. Reattach with a fresh exact base.
   Check protection for text, pixels, cursor/copy-rect and clipboard, resource
   targets, process ownership and complete cleanup.

Use the same task oracle with separate placement adapters. Slice cells retain
Room admission; host cells use user-domain desktop grants. Add negative Room,
foreign-user, expired-grant and cross-kernel host-control cases, plus focused
versus retained-grant parity. Keep headless/unavailable states explicit. Repeat
the applicable cells on the same reviewed ordinary and fresh Path-1 sources for
MP-10; builder runs alone do not establish that parity. No OSWorld host rental
or scored execution until all three providers pass slice and host with the
optimized stream and the required terminal projections. GPU validation needing
another approved host remains a declared resource prerequisite, never inferred
from software success.

## MP-08 / MP-10 / MP-11: protocol and rough PR sequence

Ask the coordinator to reserve a protocol allocation after design review; no
number is claimed. Likely additive shared contracts: desktop/surface identity,
placement/capabilities, generation/geometry, mode selection, desktop access,
bounded accessibility targets/actions and actor/input/display projections.
Desktop copy-rect and cursor extensions also need coordinated shared snapshots.
Reuse existing fields wherever they already
express those semantics. Any changed wire shape requires the OSS version bump,
shape/hash snapshots, dependent-client minima and a boundary drill. Old clients
must report unsupported host Computer capability before sending input. Add no
relay peer authority to bypass the user-domain cross-kernel refusal.

| Proposed PR | Rough size | Reviewable exit |
| --- | --- | --- |
| Linux owned desktop lifecycle | M, 400–700 source lines + fixtures | Lazy non-root Xvfb/WM/session-bus lifecycle shared with kernel browser; no adoption, isolated sockets/profile, explicit recovery and exact cleanup. |
| Shared native Computer adapter | M, 500–800 + focused tests | Factor existing input/observation helpers; direct host placement and existing slice behavior, safe Unicode/OCR/holds and protected MCP pixels. |
| AT-SPI targeting | M, 300–600 + toolkit fixtures | Scoped protected tree/target handles, revision-bound actions, stale-target denial, missing-tree OCR fallback and on-demand exact image proof. |
| Host access and input arbitration | M, 300–600 + adversarial tests | Desktop resource grant, retained/focused parity, revoke/idle/run fencing, Browser/Computer input exclusion and human takeover. Security review required. |
| Protected desktop display and clients | M OSS + S/M Cloud, 500–900 total + drills | Extend the display lane's XShm/XDamage pipeline with copy-rect, XFixes cursor, lossless UI/motion regions and human-viewer lifetime; mode switching and Web/local/remote TUI traces. Pair exact sources. |
| Linux functional acceptance | S harness + execution campaign | All 24 cells above, optimized-stream performance targets, protection/resources/cleanup and ordinary/Path-1 comparison before OSWorld rental. |

Sizes are estimates, not permission to build mega-files. Split lifecycle,
capture/input and protocol/client policy along the existing responsibility seams.
OSWorld execution waits for KVM and the benchmark campaign; host-desktop
attachment and Mac are follow-on adapters after Linux functional acceptance.
Phase B starts only after coordinator review of this Phase A report.
