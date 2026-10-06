# MP-08 / MP-10 / MP-11 — Linux Computer Phase B

Phase B implements PR boundaries 1–4 of the approved
`COMPUTER_USE_LINUX_PLAN.md` on multidomain round 2 (`6dde21a8c`).
This source supplies lifecycle, native operations, AT-SPI targeting and shared
host admission. It does not close the provider/client acceptance matrix.

## MP-08 owned host desktop and shared operations

A headed Linux host browser lazily creates a kernel-owned Xvfb, Openbox and
private session bus under its external profile state. The kernel must run as a
normal Unix user. Xvfb disables TCP, allocates distinct authenticated local sockets and requires a fresh
0600 Xauthority file; inherited login-display/bus authority is discarded.
Chromium retains its renderer sandbox and CDP pipe and binds to that display.
Shutdown uses positive PID/start-time identities for owned children and
verified descendants, removes the private runtime, and retires input helpers.
No real login screen is adopted.

Host prerequisites are Xvfb, Openbox, dbus-daemon, setxkbmap, python3 with
python-xlib/Pillow/pyatspi, xclip, Tesseract and native Chromium. OCR reuses the
slice text finder. Unicode input reuses the slice keyboard helper, including
its accelerator-safe overlay fence; a portable Xlib backend supports the host
without installing a slice's Python package. The placement adapter requires
explicit `host` or `slice`; it never silently redirects to another desktop.
Slice image helper dependencies are prepared, but no image was built here.
The existing Room/slice request authority is unchanged.

Physical X11 keycodes are individual down/up messages through a warmed helper,
with no event batching. `text` and committed `composition` are separate from
physical events. Preedit remains with the client/input method. The virtual
keymap starts as US; clients must map physical events to this display keymap.
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

## MP-08 AT-SPI and MP-11 observation protection

AT-SPI targets are scoped to verified owned application PID/start-time identities
on the private bus. Public opaque handles are observer-bound and revision-bound;
PID, bus names and object paths remain internal. Both the JS adapter and Python
action helper revalidate the tree before dispatch. Mutation retires handles;
stale denial requires rediscovery, never blind mutation replay. Missing or
incomplete accessibility coverage reports OCR fallback.

Password roles expose no text/actions. Protected registry values/targets,
unknown visible windows, incomplete AT-SPI coverage or password widgets cause
full-desktop black PNG and withheld OCR/clipboard. Coverage is checked before
and after capture; a changed tree rejects the observation. This conservative
policy avoids claiming precise non-browser secret masks. Agents cannot operate
or observe through Computer while human App views are open, preventing bypass
of App human-channel admission before the display lane provides a mapping.
MCP screenshots use the existing native image block, without a duplicate base64
text copy. Pixels stay under the existing protection barrier.

## MP-08 shared contract and MP-11 desktop arbitration

The coordinator-reserved versions are local daemon **446**, relay peer **89**.
`KernelBrowser { command: { op: "computer", command: ... } }` carries the typed
native command through the existing terminal authority and host service.
Computer clients require 446; unchanged Browser/UserDomain clients keep 443.
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
existing actor/action ledger. Human Computer input requests takeover before
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

These do not establish native Chromium on this builder, provider MCP image
receipts, Codex/OpenCode/Claude paid control, an exact-source slice image, real
slice transport, optimized stream, Web/TUI input/IME or MP-10 fresh-machine
ordinary/managed Path-1 parity. Those acceptance cells and PR5 remain deferred
as directed. MP-11 review scope is security-critical anchors and behavioral
parity; this document does not require non-security exact-blob audits.
