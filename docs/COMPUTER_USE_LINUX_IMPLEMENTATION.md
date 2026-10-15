# MP-08 / MP-10 / MP-11 — Linux Computer Phase B

## MP-08 / MP-11 native protection fallback

Native Computer does not have a proved CDP document-to-desktop transform.
It therefore withholds browser windows and their structured names/actions,
including unregistered OTP, payment, private ancestors, frames and shadow
content. The existing document-bound Browser path remains available.

Agent native text and single chords require a complete, uniquely bound owned
foreground accessibility frame and a public focused leaf. Each text keystroke
rechecks that leaf and fences the X11 focus; web-document leaves require the
Browser/Vault path. Agent native clipboard writes, text drag/drop, middle-button paste and
keyboard holds are refused because they cannot fence every text-producing
repeat/paste. Human input retains its separate admitted path.

Capture coverage is proved per visible window using a unique showing AT-SPI
frame, title and X11 geometry. An unmatched/ambiguous window is masked even
when its process has another accessible frame. Only proved coverage may
subtract an occluding window. Override-redirect windows are always inspected;
unknown popups are masked independently of the normal-window list. The
before/after observation fence covers these masks and window bindings.

MP-08 / MP-11: Room screenshots and OCR combine registered-value masks with
this same native coverage, including an empty secret registry. Both captures
fence the native tree before and after reading pixels. Room agent text and
key-repeat helpers bind and recheck the same public native leaf before each
press. Chords permit modifiers and at most one ordinary key. Human and approved
Vault typing keep their existing actor paths. Known browser applications stay
opaque during native traversal; clipboard reads also stay protected while
those applications exist, since the clipboard has no proved source.

These conservative fallbacks are security restrictions, not functional or
MP-10 acceptance evidence. Real-live hosted/client/provider parity still
requires the current validation matrix and independently reviewed anchors.


MP-08 / MP-11 reviewer R1/R2: authenticated Room worker routes refuse agent
keyboard/pointer holds, clipboard writes, drags and middle-button paste before
spawning a physical helper. Human and approved Vault input keep their admitted
paths. The keyboard helper independently refuses agent holds.
Native paste chords (including Ctrl+V and Shift+Insert variants) share the direct
clipboard-read protection policy. Unknown selection owners are protected even
with an otherwise public desktop. A public clipboard requires a live owned
native source, complete public accessibility coverage and unchanged selection
owner/tree; each paste press rechecks the admitted clipboard contents. Registered
or unknown Room protection also fences agent shortcuts. Browser-origin clipboard
content remains opaque. These are conservative input restrictions, not PR5
display or MP-10 acceptance evidence.


Phase B implements PR boundaries 1–4 of the approved
`COMPUTER_USE_LINUX_PLAN.md` on multidomain round 2 (`6dde21a8c`).
This source supplies lifecycle, native operations, AT-SPI targeting and shared
host admission. It does not close the provider/client acceptance matrix.

## MP-08 owned host desktop and shared operations

A headed Linux host browser lazily creates a kernel-owned Xvfb, Openbox and
private session bus under its external profile state. The kernel must run as a
normal Unix user. Xvfb disables TCP, allocates distinct authenticated local sockets and requires a fresh
0600 Xauthority file; inherited login-display/bus authority is discarded.
Native helper temporary files stay in the private runtime directory.
MP-08 / MP-11: Xvfb alone uses a Bubblewrap mount of that owned directory at
`/tmp`, because Xvfb hardcodes its socket and keymap scratch paths. Network and
IPC remain shared for authenticated abstract X11 connections; provider launches
are unchanged. Owned session and accessibility buses use distinct short abstract
addresses with EXTERNAL authentication, avoiding global temporary directories
and Unix socket path limits. The accessibility daemon inherits its own address
before activating registry children.
MP-08 / MP-11: Chromium retains its renderer sandbox and CDP pipe and binds to that display. Its temporary directory uses a held `/proc/<adapter-pid>/fd/<directory-fd>` reference to the same owned runtime, so singleton socket paths stay bounded under long state roots. The descriptor stays open until Chromium settles.
Shutdown uses positive PID/start-time identities for owned children and
verified descendants, removes the private runtime, and retires input helpers.
No real login screen is adopted.

Host prerequisites are Bubblewrap, Xvfb, Openbox, dbus-daemon with the installed
AT-SPI accessibility bus configuration, setxkbmap, python3 with
python-xlib/Pillow/pyatspi, xclip, Tesseract and native Chromium. OCR reuses the
slice text finder. Unicode input reuses the slice keyboard helper, including
its accelerator-safe overlay fence; a portable Xlib backend supports the host
without installing a slice's Python package. The placement adapter requires
explicit `host` or `slice`; it never silently redirects to another desktop.
MP-08 / MP-10: round 3 built and drilled an exact-source Linux slice image.
Full slice acceptance still needs bounded AT-SPI operations on the Room peer path.
The existing Room/slice request authority is unchanged.

Physical X11 keycodes are individual down/up messages through a warmed helper,
with no event batching. `text` and committed `composition` are separate from
physical events. Preedit remains with the client/input method. The virtual
keymap starts as US; clients must map physical events to this display keymap.
MP-08 / MP-11: after booting its owned virtual desktop, the slice prepares
keycode 8 as an inert text overlay slot. This requires explicit owned-display
intent, rejects held keys, preserves other modifiers and restores the previous
mapping on failure; it never remaps a user's real display implicitly.
MP-08 / MP-11: Room slice key chords use the same strict native XTEST helper
as host input, with case-independent navigation names, bounded repeats and
cancellation handlers restored between chords. Unknown or unmapped keys fail
before dispatch; they cannot be acknowledged as applied.
Each successful native input calls `wakeCapture` with its surface/generation.
This is the display lane's capture-wake seam. PR5 must bind client keyboards,
IME, capture, glyph/caret tiles and viewer lifetime to the reused display
pipeline and measure type→echo P95 alongside click→present. No desktop streamer,
caret-damage tuning or display performance result is claimed by Phase B.

Exact PNG, OCR, clipboard, finite keyboard/pointer holds and pointer operations
share the native adapter. Persistent physical key presses are a human channel;
agents use finite key chords/holds. Cancellation releases finite holds before
completion. Clipboard writers remain owned foreground processes until replaced
or shutdown. Warm keyboard helpers remain owned after key-up and are reaped.
MP-08 / MP-11: text/composition actions accept at most 128 Unicode characters,
checked before dispatch to fit the existing execution deadline; longer text
must be split into blocks. Clipboard limits remain independent. Browser recovery
retires held keys and helpers before replacing the desktop; fresh bindings get
fresh keyboard channels.

## MP-08 AT-SPI and MP-11 observation protection

AT-SPI targets are scoped to verified owned application PID/start-time identities
on the private bus. Public opaque handles are observer-bound and revision-bound;
PID, bus names and object paths remain internal. Both the JS adapter and Python
action helper revalidate the tree before dispatch. Mutation retires handles;
stale denial requires rediscovery, never blind mutation replay. MP-11: protection policy participates in cache revisions, policy updates clear observers, and actions recheck current target protection. Missing or
incomplete accessibility coverage reports OCR fallback. MP-08 / MP-11: the
public projection is bounded to 64 nodes and a 3 KiB serialized-node budget, prioritizing
visible actionable or editable controls. Pruning reports incomplete coverage
and OCR fallback. The complete private tree still fences actions and
observation protection, including changes in omitted nodes.

Password roles expose no text/actions. Protected registry values/targets,
foreign or unattributed visible windows, incomplete AT-SPI coverage or password
widgets cause full-desktop black PNG and withheld OCR/clipboard. An owned
window without AT-SPI (e.g. xterm) instead blacks out only its frame, minus
parts covered by owned AT-SPI windows stacked above it per
`_NET_CLIENT_LIST_STACKING` (whole frame without stacking data), plus every
viewable override-redirect popup. Snapshots list at most 16 of these regions as `masked`
`[x,y,w,h]` so agents know the black window still takes input; coverage stays
incomplete and clipboard read stays closed while one is visible. Coverage is
checked before and after capture or clipboard read; a changed tree or mask
rejects the observation. Agents cannot operate
or observe through Computer while human App views are open, preventing bypass
of App human-channel admission before the display lane provides a mapping.
MCP screenshots use the existing native image block, without a duplicate base64
text copy. Pixels stay under the existing protection barrier.

## MP-08 shared contract and MP-11 desktop arbitration

The coordinator-reserved versions are local daemon **461**, relay peer **92**.
`KernelBrowser { command: { op: "computer", command: ... } }` carries the typed
native command through the existing terminal authority and host service.
MP-08 / MP-11: the TUI exposes `/computer start|state|takeover|release|type <text>` through this shared contract and `/access revoke` through existing grant authority. Commands pin the selected transport and acquire fresh desktop identity before mutations. Native Computer clients require 446, per-agent Room Computer grants require 449, and bulk Room Computer restore (`/access grant all`) requires 461; unchanged Browser/UserDomain clients keep 443. Room Computer denials and explicit restores are durable across kernel restart. Both bulk revoke and restore publish visible TUI notices.
Protocol snapshot/hash tests cover all native commands/input variants and the
new `Desktop` user-domain resource. Existing historical shape fixtures retain
their filenames and data; current version pins advance once in PR4.

Official provider MCP exposes `chariox.load_kernel_computer` and, once loaded,
`chariox.kernel_computer`. It uses the existing local run/user authority, explicit
focus-created grant, retained resource scope and revoke/run epoch fencing.
Room/slice membership alone grants no host desktop access. A retained agent can
reuse its granted desktop; creating a new desktop requires focus.
Provider tools cannot forge actor fields, take over/release human input, or hold
persistent physical keys. Worker/browser tool wrappers cannot bypass admission.

Browser and Computer mutations reserve the same Desktop input target in the
existing actor/action ledger. Whole-desktop input also reserves open Browser tabs,
so tab-scoped human takeovers fence and cancel native input. Ordinary Browser
takeovers retain their existing tab scope. Human Computer input requests takeover before
waiting for controller I/O, cancelling active agent input. Takeover acknowledgments
wait for held-key reset. Release validates surface/generation/owner, resets keys
before releasing ownership, and denies a superseded owner. Disconnect/revoke
retire only the relevant observer and input; uncertain cleanup fences the backend.

## MP-11 validation scope and MP-10 remaining acceptance

Focused source checks exercise root/inherited authority denial, unsafe signal
IDs and stale PID roots, host/slice selection, viewport/input validation,
physical-key actor exclusion/retirement, App-view bypass denial, foreign/stale
AT-SPI handles, strict protocol fields, capability load/revoke and Desktop
Browser/Computer arbitration. Real non-root Xvfb drills exercise private cookie
admission, fresh lifecycle, Unicode editor persistence, down/up, clipboard,
cancelled-hold key release, real OCR, protected PNG pixel contents, scoped GTK
AT-SPI actions and cleanup. Evidence records each source hash and command;
working-tree trials are explicitly labelled as such.

MP-08 / MP-10 / MP-11 round 3 real-path drills additionally establish
Codex and OpenCode controlling a kernel-owned Chromium/native desktop through
local and remote real TUIs: Unicode save with independent UTF-8 bytes, AT-SPI
action and stale refusal, pixel-canvas OCR, human takeover and grant revoke.
An exact-source slice image also supports the existing Room Unicode, OCR and
human takeover flow through real provider runs. Full slice cells remain blocked
on the Room AT-SPI extension and a defined per-agent revoke contract.
MP-10: an existing product-linked Claude profile is authenticated, but its
real host provider turns are blocked by account quota. Claude slice cells
also require administrator-provisioned access to the shared Docker admission
locks and the supported managed credential launch path. These prerequisites
must be resolved without replacing live locks or manually transferring credentials.

These do not establish optimized display transport, Web input/IME, type-to-echo
performance, OSWorld or MP-10 fresh-machine ordinary/managed Path-1 parity.
Web/display acceptance awaits PR5 as directed. MP-11 review scope is security-critical anchors and behavioral
parity; this document does not require non-security exact-blob audits.
