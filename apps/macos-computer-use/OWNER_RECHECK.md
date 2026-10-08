# M0 delivery recheck, three minutes

Owner only. Use the rebuilt bundles in the build directory for this recheck.
The installed bundles are retained. This helper's ad-hoc cdhash changed, so the
previous permission entry is insufficient. Do not rebuild after granting.
Run only the three input operations below. No login/logout/re-login, account revocation,
TCC reset command, certificate, Keychain or kernel changes are part of this check.

## 0:00-1:30, replace the helper entries in both permission lists

In Terminal, prepare paths and open Finder at the rebuilt helper:

```bash
cu_build="$HOME/.chariox/dev/cumac/build"
cu_helper="$cu_build/Chariox Computer Helper.app"
cu_evidence="$HOME/.codex/evidence/browser-computer-use-macos/owner-recheck-20261008"
mkdir -p "$cu_evidence"
codesign --verify --strict "$cu_helper"
codesign -dvv "$cu_helper" 2> "$cu_evidence/signature.txt"
shasum -a 256 "$cu_helper/Contents/MacOS/helper" > "$cu_evidence/sha256.txt"
open -R "$cu_helper"
```

Open System Settings > Privacy & Security > Accessibility. Select the old
**Chariox Computer Helper** entry and remove it with minus. Drag the rebuilt
**Chariox Computer Helper.app** from that Finder window into the list, then
enable it. Repeat removal, Finder drag and enable in Screen Recording, called
Screen & System Audio Recording on some versions. Grant only this helper.
Authenticate personally if requested. Use the build directory app in both lists;
the plus picker previously failed, while Finder drag worked. Accept a helper
quit/reopen request if shown. Every command below launches a fresh helper.

## 1:30-2:20, fixture button and scroll

Quit only the previous public fixture using its Stop fixture button if it is
still running. Launch the rebuilt sibling fixture so its executable matches
this helper's fixture allowlist:

```bash
open -n --stdout "$cu_evidence/fixture.out" --stderr "$cu_evidence/fixture.err" "$cu_build/Chariox Computer Fixture.app"
cat "$cu_evidence/fixture.out"
```

Use the printed `fixturePID` and `windowID`, never the earlier session's IDs.
If stdout is not ready, rerun only `cat`. Set the two numbers below:

```bash
cu_pid=REPLACE_WITH_PRINTED_PID
cu_window=REPLACE_WITH_PRINTED_WINDOW_ID
cu_scope=(--enable-fixture --fixture-pid "$cu_pid" --window-id "$cu_window")
cu_delay() {
  cu_label="$1"; shift
  printf 'Five seconds: click the selected ordinary field and leave its window frontmost.\n'
  sleep 5
  open -n -g -W --stdout "$cu_evidence/$cu_label.out" --stderr "$cu_evidence/$cu_label.err" "$cu_helper" --args "${cu_scope[@]}" "$@"
  cat "$cu_evidence/$cu_label.out" "$cu_evidence/$cu_label.err"
}
```

Run each separately. During each countdown, click the fixture's ordinary text
field, avoiding the secure field. Leave the fixture frontmost until completion.
After the first command, look for **Clicks: 1**. Before the second, note the top
visible row; after it, look for the rows to move toward later rows.

```bash
cu_delay button --target button --click
```

```bash
cu_delay scroll --target scroll --scroll
```

Expected receipts are `path=AXPress; observed fixture counter increment` and
`path=AXScrollValue; observed vertical scroll position increased`, each prefixed
by `dispatched;`. Scroll is one normalized 0.05 step toward the bottom, rather
than a pixel-wheel event. `application completion unproven` means the helper
did not observe the expected change; a dispatch receipt alone is not PASS.
Verify visible movement as well as the scrollbar receipt.

## 2:20-3:00, TextEdit coordinate click

Use one disposable plain-text TextEdit document. Manually type `public click probe`
if it has no synthetic text, so caret relocation is visible. Arrange one
unambiguous TextEdit window and discover its fresh PID/window ID:

```bash
open -n -g -W --stdout "$cu_evidence/textedit-choices.out" --stderr "$cu_evidence/textedit-choices.err" "$cu_helper" --args --enable-owner-window --list-windows
cat "$cu_evidence/textedit-choices.out" "$cu_evidence/textedit-choices.err"
```

The second JSON line must contain only the Apple-signed `com.apple.TextEdit`
window. Set the exact numbers. Run this single click separately; during the
countdown click just before the synthetic text near its left edge, then move the physical
pointer onto the title bar without clicking. Watch whether the helper relocates
the insertion caret to the text area's center while the physical pointer stays
on the title bar. Do not switch back to Terminal until completion.

```bash
cu_pid=REPLACE_WITH_TEXTEDIT_PID
cu_window=REPLACE_WITH_TEXTEDIT_WINDOW_ID
cu_scope=(--enable-owner-window --owner-pid "$cu_pid" --window-id "$cu_window" --target focused)
cu_delay textedit-click --click
```

Expected receipt is `dispatched; path=CGEventWindow; application completion
unproven`. The helper deliberately does not read TextEdit's selection or text.
Record the visually observed caret movement and stationary physical pointer in
`SESSION.md` under `cu_evidence`, alongside the fixture's counter and row changes.
If there is no visible caret movement, record TextEdit click as UNPROVEN,
not PASS. Do not add any helper text operation to this recheck.
Stop and record a refusal or failure; do not replay, switch delivery paths or
modify permissions again to hide a failed input result.

## Delivery choice and remaining proof

The original PID-only mouse events were ignored in the owner session on macOS
26.5. In an agent-runnable, non-posting check, setting both public CG pointer
window fields still converted to an AppKit event with `windowNumber=0`.
The public [NSEvent mouse constructor](https://developer.apple.com/documentation/appkit/nsevent/mouseevent(with:location:modifierflags:timestamp:windownumber:context:eventnumber:clickcount:pressure:))
preserves the selected AppKit window number, single-click state and matching
pair number in the new tests. It also retains the public CG pointer-window
fields and global point location before per-PID posting. No private event fields
or cursor warp are used.

Buttons use supported [AXUIElementPerformAction](https://developer.apple.com/documentation/applicationservices/1462091-axuielementperformaction)
with AXPress. Scroll areas use their [vertical scrollbar](https://developer.apple.com/documentation/applicationservices/kaxverticalscrollbarattribute),
only when its numeric AXValue is settable and within 0...1. These are primary
paths selected by role before dispatch, with no fallback. The plan's
[Structured targets and observation protection](../../docs/COMPUTER_USE_MACOS_PLAN.md#structured-targets-and-observation-protection)
prefers supported AX presses, and its
[Minimal feasibility prototype](../../docs/COMPUTER_USE_MACOS_PLAN.md#minimal-feasibility-prototype-implemented-m0-and-owner-attended-gates)
defines the explicit M0 scroll-value path and per-PID coordinate click scope.
The [capture and display section](../../docs/COMPUTER_USE_MACOS_PLAN.md#capture-and-display)
reserves system cursor movement for future session/HID input.

All paths retain frontmost, unique CG/AX window binding, focused-window,
PID/AX ancestry, inside-window geometry, live hit-test, permission and secure
fences. Completion polls recheck them after dispatch. Source/policy tests and
the build do not prove real GUI delivery; this owner recheck remains required.

## 2026-10-08, review at 4667722815

The P2 stale-coordinate finding in #920 is corrected. Coordinate click pairs
retain the selected window bounds, element bounds and center used at
construction. Each posting fence refuses changed bounds or event coordinates,
hit-tests the actual event location, and rechecks geometry after the AX hit-test.
The deferred mouse-up uses the same fence and refuses stale geometry too.

The injected regression first failed by recording a stale mouse-down after
the element moved between construction and dispatch. It now passes for element
and window movement and resizing, including resizes with an unchanged center,
altered event locations, and movement between down and up. Unchanged geometry
still delivers one pair to the recorder. These tests make no native input or
permission calls. All five structural checks pass, and `build.sh` compiled and
verified both ad-hoc bundles in `/Users/miguel/.chariox/dev/cumac/build`.

This review pass did not launch either bundle or perform the owner steps above.
TCC and login state were left unchanged. Real GUI delivery remains UNPROVEN;
the owner recheck still applies to this rebuilt helper's changed cdhash.
