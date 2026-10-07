# M0 owner-attended session

Budget 10 minutes. macOS 14+; use only the public fixture, with synthetic text.
Nothing in this guide has been run against the desktop. This helper is disabled
without `--enable-fixture` and a separate operation flag. It exits after each
batch, has no network listener, and is not integrated into the kernel.

M0 compiles with the installed Xcode toolchain and uses ad-hoc signing. Its Team
is absent. Developer ID team `CM352DTZV6`, notarization and a protected signing
workflow are still pending. Do not create certificates or access Keychain for
this session. The plan's Developer ID feasibility gate remains UNPROVEN.

## Before the appointment

Run from this checkout. Outputs stay outside source; no app is launched.

```bash
bash apps/macos-computer-use/build.sh
```

## Minutes 0-2: install and verify public identity

Run these commands with the owner present. Stop if the destination exists;
never overwrite a previously permissioned installation during this drill.
Keep both bundles together so the helper's fixture path allowlist matches.

```bash
cu_build=/Users/miguel/.chariox/dev/cumac/build
cu_install="$HOME/Applications/Chariox Computer M0"
cu_evidence=/Users/miguel/.codex/evidence/browser-computer-use-macos
test ! -e "$cu_install" || exit 1
mkdir -p "$cu_install" "$cu_evidence"
ditto "$cu_build/Chariox Computer Helper.app" "$cu_install/Chariox Computer Helper.app"
ditto "$cu_build/Chariox Computer Fixture.app" "$cu_install/Chariox Computer Fixture.app"
cu_helper="$cu_install/Chariox Computer Helper.app"
codesign --verify --strict "$cu_helper"
codesign -dvv "$cu_helper" 2> "$cu_evidence/owner-signature.txt"
shasum -a 256 "$cu_helper/Contents/MacOS/helper" > "$cu_evidence/owner-sha256.txt"
open -n -g -W --stdout "$cu_evidence/owner-identity.json" --stderr "$cu_evidence/owner-identity-error.txt" "$cu_helper" --args --identity
cat "$cu_evidence/owner-identity.json"
```

Expected identifier `ai.chariox.computer-helper`, installed executable path,
`signatureValid=true`, `team="none (ad-hoc)"`, CDHash, PID and parent PID.
These are public identity facts, not production pairing credentials.

For the later kernel attribution test, have the disposable kernel's supervisor
launch the same installed bundle and the same `--identity` command through
LaunchServices. Set its `CHARIOX_HOME` to a fresh absolute directory below
`/Users/miguel/.chariox/dev/cumac/`, with no owner credentials. Record its launch
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

## Minutes 4-7: one window, AX target and benign input

```bash
open -n --stdout "$cu_evidence/owner-fixture.txt" "$cu_install/Chariox Computer Fixture.app"
```

Read `fixturePID=... windowID=...` from `owner-fixture.txt`. Set these two
variables to those exact printed numbers. The fixture must stay frontmost for
input. Do not enter any real secret. The secure field starts empty.

```bash
cu_pid=REPLACE_WITH_FIXTURE_PID
cu_window=REPLACE_WITH_WINDOW_ID
cu_scope=(--enable-fixture --fixture-pid "$cu_pid" --window-id "$cu_window")
cu_run() { open -n -g -W --stdout "$cu_evidence/$1.out" --stderr "$cu_evidence/$1.err" "$cu_helper" --args "${cu_scope[@]}" "${@:2}"; cat "$cu_evidence/$1.out" "$cu_evidence/$1.err"; }
cu_run frame --capture "$cu_evidence/owner-fixture.png"
cu_run ax --target ordinary --ax-read
cu_run button --target button --click
cu_run scroll --target scroll --scroll
```

Inspect the PNG: exactly the selected fixture window, moving patch, no cursor,
no other desktop windows or audio. Record pixel dimensions and display scale.
Observe the button counter increase once and scroll rows move. Dispatch receipts
alone do not establish application completion.

Click the ordinary field yourself, leave the fixture frontmost, then run:

```bash
cu_run unicode --target ordinary --text $'é e\u0301 😀 中'
cu_run after --capture "$cu_evidence/owner-after.png"
cu_run secure --target secure --text canary
```

Expected ordinary text exactly `é é 😀 中`, no navigation or shortcut, and
`refused: secure` for the secure target without any event. Unicode uses one
bounded UTF-16 payload, clears modifiers and never normalizes or uses clipboard
or AX value assignment. Keycode 0 inertness is experimental until observed.
[Apple documents that apps may ignore the Unicode payload](https://developer.apple.com/documentation/coregraphics/cgevent/keyboardsetunicodestring(stringlength:unicodestring:)).
Focus the secure field yourself, repeat `cu_run frame-secure --capture
"$cu_evidence/owner-secure.png"`, and expect secure-input refusal. Then return
focus to the ordinary field. Leave all real apps outside this fixture-only test.

This implements only the window subset of plan steps 3-4. App/display scope,
private-region masking, OCR, real apps, mixed-display movement, kernel grants,
human takeover, local helper Stop and fatal owned-release proof require later
slices. The fixture's Stop button closes the fixture; it is not the kernel Stop
contract. Input checks narrow focus races but cannot make CGEvent delivery atomic.
There are no long holds or retries. Do not label those broader checks PASS.

## Minutes 7-10: revoke and finish

The owner switches off only the installed helper in both permission lists and
removes its entries with the minus button where available. Do not reset TCC
globally. Repeat `cu_run revoked-frame --capture "$cu_evidence/revoked.png"`
and `cu_run revoked-input --target ordinary --text public`; expect refusals and
no new pixels/input. If macOS needs a relaunch, these are fresh helper processes.
If attribution failed, cancel its prompt instead of granting another app.

Close the fixture using Stop or its close button. Each helper already exited.
Inventory only the two installed bundles before removing the M0 install
directory if the owner wants rollback; keep the public evidence. Do not touch
installed kernels, shared state, signing assets, credentials or the reviewer.
Record OS version, public identity/hash, displayed TCC app, success/refusals and
any unproven checks in the evidence directory. Kernel attribution and the full
plan gate remain pending even if every standalone fixture check succeeds.
