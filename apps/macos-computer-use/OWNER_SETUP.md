# M0 owner-attended session

For the click/scroll repair after the 2026-10-08 owner session, use
[OWNER_RECHECK.md](OWNER_RECHECK.md). It reruns only fixture button, fixture scroll
and TextEdit click after replacing the rebuilt helper's permission entries.

Budget 15 minutes. macOS 14+; use the public fixture and one owner-selected
real TextEdit window, both with synthetic text. Owner mode and `--list-windows`
admit only `com.apple.TextEdit` whose running PID satisfies the code requirement
`identifier "com.apple.TextEdit" and anchor apple`. Other apps and TextEdit
impostors refuse before capture, AX or input.
Nothing in this guide has been run against the desktop. This helper is disabled
without `--enable-fixture` or `--enable-owner-window` and a separate operation flag. It exits after each
batch, has no network listener, and is not integrated into the kernel.

M0 compiles with the installed Xcode toolchain and uses ad-hoc signing. Its Team
is absent. Developer ID team `CM352DTZV6`, notarization and a protected signing
workflow are still pending. Do not create certificates or access Keychain for
this session. The plan's Developer ID feasibility gate remains UNPROVEN.

## Before the appointment

Run from this checkout. Outputs stay outside source; no app is launched.

```bash
CUMAC_BUILD_DIR="$HOME/.chariox/dev/cumac/build" bash apps/macos-computer-use/build.sh
```

## Minutes 0-2: install and verify public identity

Run these commands with the owner present. Stop if the destination exists;
never overwrite a previously permissioned installation during this drill.
Keep both bundles together so the helper's fixture path allowlist matches.

```bash
cu_build=${CUMAC_BUILD_DIR:-"$HOME/.chariox/dev/cumac/build"}
cu_install="$HOME/Applications/Chariox Computer M0"
cu_evidence="$HOME/.codex/evidence/browser-computer-use-macos"
if [ -e "$cu_install" ]; then
  printf 'Installation exists; stop this drill and retain it.\n'
else
  mkdir -p "$cu_install" "$cu_evidence"
  ditto "$cu_build/Chariox Computer Helper.app" "$cu_install/Chariox Computer Helper.app"
  ditto "$cu_build/Chariox Computer Fixture.app" "$cu_install/Chariox Computer Fixture.app"
  cu_helper="$cu_install/Chariox Computer Helper.app"
  codesign --verify --strict "$cu_helper"
  codesign -dvv "$cu_helper" 2> "$cu_evidence/owner-signature.txt"
  shasum -a 256 "$cu_helper/Contents/MacOS/helper" > "$cu_evidence/owner-sha256.txt"
  open -n -g -W --stdout "$cu_evidence/owner-identity.json" --stderr "$cu_evidence/owner-identity-error.txt" "$cu_helper" --args --identity
  cat "$cu_evidence/owner-identity.json"
fi
```

Expected identifier `ai.chariox.computer-helper`, installed executable path,
`signatureValid=true`, `team="none (ad-hoc)"`, CDHash, PID and parent PID.
These are public identity facts, not production pairing credentials.

For the later kernel attribution test, have the disposable kernel's supervisor
launch the same installed bundle and the same `--identity` command through
LaunchServices. Set its `CHARIOX_HOME` to a fresh absolute directory below
`$HOME/.chariox/dev/cumac/`, with no owner credentials. Record its launch
receipt and helper identity together. LaunchServices may parent the helper to
launchd, so parent PID alone cannot prove the kernel initiated the launch.
M0 provides no supervisor command or normal Computer Action/grant API; M1 must
supply that launcher. Do not substitute a provider shell or an installed kernel.
Until that exists, the following commands are standalone OS diagnostics and
cannot pass the plan's kernel-launch or normal Action/grant gates.

## Minutes 2-4: owner grants to the helper

The owner opens System Settings, Privacy & Security, Screen Recording or Screen
& System Audio Recording, then adds **Chariox Computer Helper.app** at the
installed path above. Add that same app to Accessibility. The executable
receiving both grants is `Contents/MacOS/helper`. Authenticate personally if
macOS asks. Do not grant Terminal, iTerm, a provider, the fixture or the kernel.
No Input Monitoring is needed for this slice because there is no activity tap.
Quit/reopen the helper if macOS requests it; each command below launches anew.

Inspect the displayed app and installed identity before enabling either grant.
For the later kernel-launched check, keep Terminal ungranted, repeat the capture
and input commands through the disposable kernel supervisor, then disable only
the helper's grant and repeat. Expected: helper success when granted, refusal
when revoked, and no Terminal/provider permission request. If macOS attributes
the request to any other executable, cancel it and record attribution FAILED.
Ad-hoc signing or a rebuilt binary may change TCC attribution; do not assume
the result will survive a signing change.

## Minutes 4-9: fixture regression checks

```bash
open -n --stdout "$cu_evidence/owner-fixture.txt" "$cu_install/Chariox Computer Fixture.app"
```

Read `fixturePID=... windowID=...` from `owner-fixture.txt` and set the exact
numbers below. All operations require the selected window to be frontmost and
AX-focused, including capture. Never enter a real secret. The secure field
starts empty.

```bash
cu_pid=REPLACE_WITH_FIXTURE_PID
cu_window=REPLACE_WITH_WINDOW_ID
cu_scope=(--enable-fixture --fixture-pid "$cu_pid" --window-id "$cu_window")
cu_run() { open -n -g -W --stdout "$cu_evidence/$1.out" --stderr "$cu_evidence/$1.err" "$cu_helper" --args "${cu_scope[@]}" "${@:2}"; cat "$cu_evidence/$1.out" "$cu_evidence/$1.err"; }
cu_delay() {
  for cu_second in 5 4 3 2 1; do
    printf 'Runs in %s seconds: click the selected window/field now.\n' "$cu_second"
    sleep 1
  done
  cu_run "$@"
}
```

Run each following command separately from Terminal. During each visible
countdown, click the fixture's ordinary field, except where another field is
specified. Stay in the fixture until the helper finishes. `open -g` leaves that
app frontmost. Do not run the whole block as a batch. Returning to Terminal
before dispatch should produce `refused: target` with no input or saved frame.

```bash
cu_delay frame --capture "$cu_evidence/owner-fixture.png"
cu_delay ax --target ordinary --ax-read
cu_delay button --target button --click
cu_delay scroll --target scroll --scroll
```

Inspect the PNG for exactly the selected fixture window and moving patch, no
cursor, other windows or audio. Record dimensions and display scale. Observe
the button counter increase once and scroll rows move. Dispatch receipts alone
do not establish application completion.

Run these separately, clicking the ordinary field during each countdown. For
`emoji20`, also put the caret at the end of the existing text during the
countdown, for example with Command-Right Arrow:

```bash
cu_delay unicode --target ordinary --text $'é e\u0301 😀 中'
cu_delay emoji20 --target ordinary --text '😀😀😀😀😀😀😀😀😀😀'
cu_delay emoji22 --target ordinary --text '😀😀😀😀😀😀😀😀😀😀😀'
cu_delay after --capture "$cu_evidence/owner-after.png"
cu_delay secure --target secure --text canary
```

Expect the ordinary text exactly `é é 😀 中` followed by ten emoji. The eleven
emoji request contains 22 UTF-16 units and must return `refused: text`, leaving
text unchanged. Each request admits at most 20 UTF-16 units. Overflow refuses
the whole string without truncation, grapheme splitting or surrogate splitting.
The explicit secure target must return `refused: secure` without an event.
There is no normalization, clipboard or AX value assignment. Modifiers are
cleared. Keycode 0 inertness remains an owner-session gate; apps may ignore the
[Unicode payload](https://developer.apple.com/documentation/coregraphics/cgevent/keyboardsetunicodestring(stringlength:unicodestring:)).

For the actual secure-input test, run the following command, then click the
empty secure field during the countdown and keep the fixture active. Do not
return to Terminal until the helper exits. This capture uses the ordinary target
by default, so refusal must come from the live secure-input/focus fence rather
than the explicit `--target secure` policy. Expect `refused: secure` and no PNG.
Record a failure if macOS does not enable secure input or the refusal differs.

```bash
cu_delay frame-secure --capture "$cu_evidence/owner-secure.png"
```

## Minutes 9-12: one owner-selected real app window

The fixture is a regression target. It does not satisfy the real-app merge gate.
The owner opens a new blank TextEdit document manually, makes it plain text and
uses only synthetic content. Keep existing private documents out of this drill.
Do not select System Settings, password fields or another person's window.
The owner chooses the exact window ID anew in this session; no target is stored
or selected automatically.

With the owner present, list public window IDs, PIDs and app bundle identifiers.
This command does not capture pixels, read AX, list window titles or post input.

```bash
open -n -g -W --stdout "$cu_evidence/owner-window-choices.json" --stderr "$cu_evidence/owner-window-choices.err" "$cu_helper" --args --enable-owner-window --list-windows
cat "$cu_evidence/owner-window-choices.json" "$cu_evidence/owner-window-choices.err"
```

Successful `--list-windows` stdout contains exactly two JSON lines: the helper's
public launch identity, then an array of visible, layer-zero windows whose live
PIDs pass the Apple TextEdit allowlist. Each array item has exactly `bundle`,
`pid` and `windowID`, for example
`[{"bundle":"com.apple.TextEdit","pid":123,"windowID":456}]`.
The numbers must be the live TextEdit PID and window ID. There are no window
titles, fixture entries or other bundle IDs; stderr is empty on success.
An empty array means no allowed window was found.

Inspect `owner-window-choices.json` and record in `owner-allowlist.txt` that the
array contains only `com.apple.TextEdit` and no `ai.chariox.computer-fixture`
entry, while the fixture remains open. If TextEdit is missing, another bundle
appears, or discovery refuses, stop the drill and record `allowlist gate FAILED`
with the observed output. Do not continue with a guessed or fixture PID.

Exercise the owner-mode allowlist against the still-open fixture before selecting
TextEdit. Run this one command separately. During the countdown, click the
fixture's ordinary field and leave it frontmost until completion, so a focus
failure cannot substitute for the allowlist refusal:

```bash
cu_scope=(--enable-owner-window --owner-pid "$cu_pid" --window-id "$cu_window" --target focused)
cu_delay owner-fixture-refusal --ax-read
```

Expect `refused: target` in `owner-fixture-refusal.err`, with no AX role receipt,
capture or input. Record that result alongside the discovery check in
`owner-allowlist.txt`. Record `allowlist gate PASS` only when both checks match;
otherwise record `allowlist gate FAILED` and stop.

Choose the PID/window ID for `com.apple.TextEdit`. If several TextEdit windows
are listed and the owner cannot identify the blank one, stop and arrange one
unambiguous blank window before listing again. Never guess the ID.

```bash
cu_real_pid=REPLACE_WITH_OWNER_SELECTED_PID
cu_real_window=REPLACE_WITH_OWNER_SELECTED_WINDOW_ID
cu_scope=(--enable-owner-window --owner-pid "$cu_real_pid" --window-id "$cu_real_window" --target focused)
```

Run each command separately. During every countdown, click the blank document's
text area and leave TextEdit frontmost until completion.

```bash
cu_delay real-before --capture "$cu_evidence/owner-real-before.png"
cu_delay real-ax --ax-read
cu_delay real-click --click
cu_delay real-unicode --text $'é e\u0301 😀 中'
cu_delay real-after --capture "$cu_evidence/owner-real-after.png"
```

Expect AXTextArea or AXTextField role only, one benign click, exact synthetic
text and before/after PNGs containing only the selected real window. Inspect the
real app's result personally. Switch to another window during an additional
countdown to confirm `refused: target` and no event/frame, then restore focus.

The owner path uses the same secure-input, PID/window-ownership, frontmost,
AX-focused-window and input-hit fences as the fixture. The focused AX element
must belong to that exact window; secure ancestors refuse. Public AX geometry
must uniquely match the selected CG window, otherwise the helper refuses.
No fallback to another window, application or display exists.

Text clicks now prefer AXRangeForPosition and AXSelectedTextRange readback.
Only an unresolved position range uses a fenced session/HID click, which can
move the system pointer. Unicode typing remains per-PID. Use the rebuilt helper
and the shorter `OWNER_RECHECK.md` for the pending click check. Session-tap
observation, input self-tagging and fatal release recovery remain unproven.

This implements the single-window part of plan steps 3-4 as standalone OS
diagnostics. App/display scope, private-region masking, OCR, mixed-display
movement, kernel grants, human takeover, local helper Stop and fatal
owned-release proof require later slices. The fixture Stop button closes only
the fixture. Focus checks cannot make CGEvent delivery atomic. There are no
long holds or retries. Do not label those broader checks PASS.

## Minutes 12-15: revoke and finish

First the owner switches off only the installed helper's Screen Recording grant
and removes that entry with the minus button where available. Leave the helper's
Accessibility grant enabled. Do not reset TCC globally. Run each command below
separately and focus the selected TextEdit text area during its countdown:

```bash
cu_delay revoked-frame --capture "$cu_evidence/revoked.png"
cu_delay screen-revoked-input --text public
```

Expect `revoked-frame` to print `refused: permission` with no PNG. The capture
path checks Screen Recording before Accessibility or AX target resolution.
Expect `screen-revoked-input` to print `dispatched; path=CGEventPIDText; application completion unproven`
and observe exactly `public` inserted in TextEdit. This input does not check
Screen Recording. Record both results and that Accessibility stayed enabled;
these establish the Screen Recording revocation separately. Stop and record
revocation FAILED if a frame appears or the short text does not dispatch and
complete.

Then the owner switches off only the installed helper's Accessibility grant and
removes that entry where available, keeping Screen Recording disabled. Focus
the same selected TextEdit text area during this countdown:

```bash
cu_delay revoked-input --text public
```

Expect `refused: permission` and no additional text. Record the separate
Accessibility revocation result; any input is revocation FAILED. These are
fresh helper processes. If attribution failed, cancel its prompt instead of
granting another app.

Close the fixture using Stop or its close button and close the synthetic
TextEdit document without saving. Each helper already exited. Inventory only
the two installed bundles before removing the M0 install directory if the owner
wants rollback; keep public evidence. Do not touch installed kernels, shared
state, signing assets, credentials or the reviewer. Record OS version, public
identity/hash, displayed TCC app, selected real PID/window ID, observed app
results, success/refusals and unproven checks in the evidence directory. Kernel
attribution, Developer ID and the full plan gate remain pending even if every
standalone check succeeds. M0 remains implemented but unattended until this
owner session produces real-app evidence.
