# MP-08 / MP-10 / MP-11 shared Computer helper fixtures

These fixtures test production native input and observation helpers independently
of the viewer. They do not close any MP item or substitute for the signed
ordinary/Path-1, official-provider, Room, host-browser, Web or TUI matrix.

Run from a clean source checkout with an immutable local fixture image containing
Chromium, Xorg dummy/Xvfb, Openbox, Mousepad and English/German Tesseract models:

```sh
python3 apps/cli/scripts/live-computer-input-x11-drill.py \
  --image sha256:<fixture-image-id> --server Xorg \
  --output /absolute/external/evidence/computer-xorg
python3 apps/cli/scripts/live-computer-input-x11-drill.py \
  --image sha256:<fixture-image-id> --server Xvfb \
  --output /absolute/external/evidence/computer-xvfb
```

The runner mounts only the public helper source read-only, creates a unique
container without network or published ports, and caps it at 2 GiB, one CPU and
512 PIDs. Admission reserves its 2 GiB forecast above 16 GiB MemAvailable and
retains 10 GiB disk headroom. A five-second watchdog stops only this container
if either floor is crossed. SIGINT/SIGTERM and assertion failures run the exact
owned-container cleanup. The output directory must be new and outside the repo.
No provider credentials, runtime identities, volumes or images are created.

The physical oracle reads a first-party page's acknowledgements through CDP.
All tested page mutations use native production Computer helpers. It records
trusted event counts, stable tab/document identity, non-US layout text, composed
and decomposed Unicode, multiline text, bounded Unicode recycling, cancellation,
reset of held native keys/buttons, clicks/double/right-click/drag, both scroll
axes, selection/copy, clipboard directions, native focus retention and a real
Mousepad edit/save/copy. Screenshot assertions check original desktop geometry
and a rendered color marker. Actual Tesseract checks no-match, two occurrences,
different font sizes and German text. Unknown secret protection must withhold
pixels and publish no image. Clipboard values and command output are never
retained in the receipt. Screenshots contain public fixture data only.

The G2 native Unicode failure can trigger BrowserRefresh because an unassigned
X11 keycode may still have a Chromium hardware fallback. The Computer helper
limits overlays to keycodes 8 and 92 when upstream discovery marks them spare;
both have unknown defaults in the
[Chromium native keycode table](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/events/keycodes/keyboard_code_conversion_x.cc).
Occupied/modifier slots remain excluded. Recycling retains the pinned keyboard
implementation's mapping-settle delay and releases the press-time keycode.
The behavior must be physically replayed when the pinned keyboard implementation,
Chromium, or supported X server changes. An exhausted safe pool fails closed.

`apps/cli/scripts/lib/computer-image-evidence.mjs` supplies receipt assertions for
the Room MCP image boundary. A live replay must pass the unmodified screenshot
MCP result to `assertRoomComputerImage`, or use `assertRoomComputerSurface` with
the actual kernel Environment snapshot, slice binding, input acknowledgement and
Computer status. It requires one native image block, byte-exact digest/size,
bounded PNG header dimensions and canonical geometry. Artifact references alone,
text/structured duplication, another Room/surface/agent, another authority or a
stale input generation fail. These assertions check transport receipts, not full
PNG decoding, screenshot redaction or whether a model perceived the image.

MP-08/MP-10/MP-11 acceptance still requires a pixel-only task through each
official provider's Chariox MCP path, with retained prompt/turn identity, actual
model-visible image bytes and an independently checked visual acknowledgement.
Join those observations to the same Room/host-browser surface and attributed
input action seen in Web, local TUI and remote TUI. A captured file or one correct
guess does not prove this gate. Human clipboard transport, Vault secret policy,
IME preedit/composition, stream-loss takeover and physical viewer replay remain
separate rows. G2 exposes only whole text and key-chord actions; public separate
hold/release/cancel semantics require a coordinator-allocated protocol before
implementation. Native reset tests do not imply those actions exist.
