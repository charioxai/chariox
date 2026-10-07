# MD-DISPLAY-02/04: kernel-owned native capture, Linux first

Phase8 follows the owner's native-adapter decision. The display feature remains
`CHARIOX_KERNEL_BROWSER_DISPLAY=1`, default off. Local 427/relay74 are unchanged;
reservation441/84 is unused. This changes private source/encoder plumbing only.
It does not establish managed parity, native-OS acceptance or rollout readiness.

## MD-DISPLAY-02: capture interface and ownership

`kernel-browser-native.mjs` supplies the OS-neutral capture contract:
`start() -> source`, `subscribe(listener) -> unsubscribe`,
`sample(afterSerial) -> immutable latest frame|null`, `close() -> settled cleanup`.
Linux implements it with XDamage/XShm. `selectNativeCapture` returns unsupported
on macOS and Windows; ScreenCaptureKit can implement this same contract later.
No extension, automatic picker, user grant or portal route is introduced.

When the experimental headed Linux browser starts, `HostChromium` starts its own
Xvfb with a dynamically allocated display, no TCP listener, and a 0600 Xauthority
cookie beneath the kernel browser state root. Its environment replaces inherited
DISPLAY/XAUTHORITY for Chromium only. A module-private WeakSet brands the live
server capability; a display string, copied descriptor or claimed `owned` flag
cannot select native capture. Failure to start Xvfb leaves the existing CDP path
available; it never authorizes native readback of the inherited display. The
kernel supervises Chromium and Xvfb and removes the cookie when they stop.

The Python helper discovers a unique window with `_NET_WM_PID` matching the
supervised Chromium child on that private server. It never reads the root window.
[XComposite](https://www.x.org/guide/extensions/) supplies that window's backing pixmap, keeping another window's
occlusion outside captured pixels. XDamage coalesces changed rectangles to 60 Hz;
XShmGetImage reads its native-DPR viewport into a 0600 SysV segment. IPC_RMID marks
the segment for reclamation after attachment, including helper exit. Missing
X11/XDamage/XShm/XComposite libraries/extensions, ambiguous windows, unsupported
pixel format or geometry cause fallback. There is no Wayland compositor in this
implementation; a future kernel-owned PipeWire compositor adapter can use the
same contract without accessing a user's session or invoking a portal.

The current launch fits1280x800 CSS atDPR 2 below87 CSS pixels of Chromium chrome.
A strict complete RGB comparison against document-bound CDP capture admits that
crop; first-paint mismatch gets at most three observation retries. An unfamiliar
browser decoration/layout falls back. DPR 1 uses the existing protected CDP path
when its geometry does not match the native surface. Multi-tab native capture is
conservatively disabled; adding a second tab closes the native source. Admission
activates only that sole browser tab inside the private display.

## MD-DISPLAY-02/04: protection and bounded pipeline

Native readback is admitted only with an empty, known Vault protection policy,
no protected targets, one tab, unchanged generation and document, live owned
server, and visible source in a private isolated world. Navigation/new-page
notifications fence it synchronously. Every raw sample awaits the loader and
visibility checks before publication. Policy change closes the source and
invalidates existing producers; final frame commit retains the existing policy,
source/document, actor/takeover and cancellation checks. Protected pages keep
the current protected CDP capture and masking path atDPR 1/2. Neither native
capability nor a hardware codec grants browser input authority.

XShm eliminates Xlib pixel transport. It is not end-to-end zero-copy: the helper
copies one immutable BGR0 frame into its bounded binary pipe, Node keeps latest
capture/encode work, and the portable encoder receives raw bytes beneath a small
private JSON header. Motion avoids PNG compression, base64 image transport and
PNG decode. This private pipe does not change terminal/relay serialized shapes.
The existing VP9/H.264 software encoder, bounded dependency queue, credit window,
pacing, encrypted runtime events and client decoder/presenter remain shared.
Exact settled repair still uses asynchronous protected CDP PNG verification and
the existing compressed damaged tiles, without blocking the input lane. Damage
rectangles are retained privately; the motion encoder currently encodes the
complete viewport rather than using codec-specific ROI hints.

Linux capture needs python3 and the system X libraries; portable software encode
also needs PyAV/libvpx/libx264 as before. No C compiler or development headers are
required at runtime. This builder has no supported hardware codec. VAAPI/QSV is
still a future encoder attachment; no GPU performance is claimed. macOS remains
unsupported behind the source contract until ScreenCaptureKit supplies an owned
Chromium surface and its policy/document fence; VideoToolbox remains the encoder
attachment described in the phase7 note.

## MD-DISPLAY-04: validation scope

Fail-first adapter-selection receipt, live private-Xvfb selection/retirement,
foreign-display refusal, unsafe PID cases and hidden/document/policy publication
fences live under `display/phase8/`. Tests and one static native attestation do
not close a multidomain or Path-1 acceptance item. The source-bound campaign and
performance matrix belong in `MULTIDOMAIN_DISPLAY_PERFORMANCE.md`; historical
phase7 receipts retain their original source identities. No owner/provider
credentials are read, copied or printed, and no Cloud or hosted relay is changed.

The XShm shared-image interface and lifecycle follow the
[X.Org MIT-SHM specification](https://xorg.freedesktop.org/archive/X11R7.7/doc/xextproto/shm.html).
The scope here is kernel-created browser displays. Future computer use can reuse
the same owned-window raw adapter with its own existing kernel surface/input and
observation fences; it must not broaden this capability to a real desktop.
