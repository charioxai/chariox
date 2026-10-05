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
The known-protection row uses public static field/canvas canaries and a supplied
value policy. Real OCR first acknowledges two visible canaries; the masked PNG
must cover both text boxes and every canvas pixel, preserve the benign marker,
retain canonical geometry/focus, and make protected OCR return no match while
the two benign labels remain readable. Its private raw baseline stays in the
disposable container. This tests helper masking, not Vault authorization or
model-visible delivery. Reset seeding uses Ctrl+Shift+F8 instead of Chromium's
Ctrl+Shift+A tab-search command so later cases retain the original browser UI.

The G2 native Unicode failure can trigger BrowserRefresh because an unassigned
X11 keycode may still have a Chromium hardware fallback. The Computer helper
limits overlays to keycodes 8 and 92 when upstream discovery marks them spare;
both have unknown defaults in the
[Chromium native keycode table](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/ui/events/keycodes/keyboard_code_conversion_x.cc).
Occupied/modifier slots remain excluded. Recycling retains the pinned keyboard
implementation's mapping-settle delay and releases the press-time keycode.
Lookup retains upstream's full inherited-overlay distrust set, including unsafe
keycodes excluded from allocation. The physical fixture seeds a Unicode overlay
on BrowserRefresh (181), then requires trusted text without document reload or
focus loss. `test_slice_keyboard_overlays.py`, run with the pinned Selkies Python
inside the dependency image, also checks core/XKB lookup, actual upstream
press/release, prebinding and exhausted-safe-pool rejection using a fake display.
Single/double/right-click acknowledgements are asserted before dragging. The
drag's press/release and optional release click are counted separately, because
Chromium versions can differ in whether this movement dispatches a click.
The pointer target disables text selection to prevent a native selected-text
drag from swallowing its release; the textarea separately proves selection/copy.
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
separate rows. Allocated local protocol421 adds bounded `keyboard_hold` and `pointer_hold`.
The physical fixture now records trusted initial press/release timestamps,
modifier chords, interrupt finalization, kernel-style SIGKILL/reset, invalid
bounds, pre-existing foreign-hold preservation, and Mousepad selection/focus.
One hold owns the Room desktop through release, including cancellation through
`CancelRoomEnvironmentAction`; separate persistent down/up calls are not added.
Kernel tests cover Room/Tab identity, agent membership, human takeover, stale
generation and redacted idempotency through the actual shared dispatch with a
synthetic native helper. They are source evidence, not live host-browser,
provider, transport or Web/TUI observations.
