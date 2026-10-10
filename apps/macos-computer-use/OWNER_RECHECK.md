# One owner recheck, at most three minutes

Use the rebuilt helper at
`/Users/miguel/.chariox/dev/cumac/build/Chariox Computer Helper.app`
in **both** privacy lists. Its ad-hoc identity changed. Do not rebuild after
granting. The installed bundles are retained; all commands below use the build
bundles. This is one button, one scroll and one TextEdit click, with no replay.

## 0:00-1:30, grant the rebuilt helper

Run in Terminal:

```bash
cu_build="$HOME/.chariox/dev/cumac/build"
cu_helper="$cu_build/Chariox Computer Helper.app"
cu_evidence="$HOME/.codex/evidence/browser-computer-use-macos/owner-recheck-20261010-axtext"
mkdir -p "$cu_evidence"
codesign --verify --strict "$cu_helper"
codesign -dvv "$cu_helper" 2> "$cu_evidence/signature.txt"
shasum -a 256 "$cu_helper/Contents/MacOS/helper" > "$cu_evidence/sha256.txt"
python3 -c 'import sys; open(sys.argv[1],"w").write("\n".join(f"line {n:02d} public caret probe" for n in range(1,13)))' "$cu_evidence/caret-probe.txt"
open -R "$cu_helper"
```

In System Settings > Privacy & Security > Accessibility, remove the old
Chariox Computer Helper entry with minus. Drag the rebuilt app from Finder into
the list and enable it. Repeat removal, Finder drag and enable in Screen
Recording, called Screen & System Audio Recording on some versions. Grant
only this helper. Authenticate personally if requested. Accept quit/reopen if
shown. Every command below starts a fresh background helper.

## 1:30-2:10, fixture button and scroll

If an older public fixture is running, close it with its Stop fixture button.
Launch the rebuilt sibling and use its fresh IDs:

```bash
open -n --stdout "$cu_evidence/fixture.out" --stderr "$cu_evidence/fixture.err" "$cu_build/Chariox Computer Fixture.app"
sleep 1
read cu_pid cu_window <<< "$(python3 -c 'import re,sys; m=re.search(r"fixturePID=(\d+) windowID=(\d+)",open(sys.argv[1]).read()); assert m; print(m[1],m[2])' "$cu_evidence/fixture.out")"
cu_scope=(--enable-fixture --fixture-pid "$cu_pid" --window-id "$cu_window")
cu_delay() {
  cu_label="$1"; shift
  printf 'Five seconds: focus the target and leave its window frontmost.\n'
  sleep 5
  open -n -g -W --stdout "$cu_evidence/$cu_label.out" --stderr "$cu_evidence/$cu_label.err" "$cu_helper" --args "${cu_scope[@]}" "$@"
  cat "$cu_evidence/$cu_label.out" "$cu_evidence/$cu_label.err"
}
```

Run separately. During each countdown, click the fixture's ordinary field and
leave the fixture frontmost. Use only the ordinary field, not the secure one.

```bash
cu_delay button --target button --click
cu_delay scroll --target scroll --scroll
```

Expected receipts:

```text
dispatched; path=AXPress; observed fixture counter increment
dispatched; path=AXScrollValue; observed vertical scroll position increased
```

The fixture should show Clicks: 1 and later rows. Close it with Stop fixture.

## 2:10-3:00, TextEdit line-5 caret

Open the generated 12-line plain-text document. Close other TextEdit document
windows so discovery returns exactly one. Keep all 12 lines visible.

```bash
open -a TextEdit "$cu_evidence/caret-probe.txt"
open -n -g -W --stdout "$cu_evidence/textedit-choices.out" --stderr "$cu_evidence/textedit-choices.err" "$cu_helper" --args --enable-owner-window --list-windows
cat "$cu_evidence/textedit-choices.out" "$cu_evidence/textedit-choices.err"
read cu_pid cu_window <<< "$(python3 -c 'import json,sys; rows=json.loads(open(sys.argv[1]).read().splitlines()[-1]); assert len(rows)==1; print(rows[0]["pid"],rows[0]["windowID"])' "$cu_evidence/textedit-choices.out")"
cu_scope=(--enable-owner-window --owner-pid "$cu_pid" --window-id "$cu_window" --target focused)
cu_delay textedit-click --click-at-pointer --click
```

During the five-second countdown, click at the beginning of **line 01**, which
has text. Then move the pointer, without clicking, over the middle of the
letter **r** in **caret** on **line 05**. Leave TextEdit frontmost and keep the
pointer there until the helper finishes. `--click-at-pointer` snapshots that
requested global point once; the AX path posts no mouse events.

Expected second stdout line, with the point's actual coordinates:

```text
dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point=X,Y
```

The insertion caret must now be before that `r` on line 05. The range readback
proves the application's selection without needing someone to watch delivery.
Index 125 is UTF-16, four preceding 27-unit lines plus 17 units on line 05.
A point over another letter within `caret` yields 123-127; record the exact
index and point, but use 125 for the specified middle-letter check.

Record the three receipts and verdicts in `SESSION.md` under `cu_evidence`.
Close only this disposable TextEdit document. A refusal, a different path, or
`application completion unproven` is not PASS. Stop there, without retrying or
changing permissions again to hide the result. If the helper reports
`refused: ownedInput; unresolved owned input; owner reset required`, stop all
helper input until the owner clears the target application's tracking state.

## What this single recheck resolves

The unattended drill is `python3 apps/macos-computer-use/text-click-drill.py`.
It launches its own NSTextView fixture, publishes a line-5 glyph point and
expected index 125, and requires both the helper's public observed-selection
receipt and the fixture's actual selection to agree. It closes only its own
fixture. On this lane's Mac launch context, the rebuilt helper returned
`refused: permission`; no AX or HID input was dispatched. This is BLOCKED,
not caret PASS. The owner-only unresolved cell is **real TextEdit AX caret
placement with range readback at line 5**. Button and scroll are regression
cells in the same recheck, previously PASS on October 9.

The center of a partly visible text element now uses its element/window/active-
display intersection; a fully invisible element still refuses. Unit tests cover
that clamp, composed-character range mapping, fallback selection, stale-point
refusal and owned mouse-up cleanup. AX range lookup uses Apple's public
[AXRangeForPosition](https://developer.apple.com/documentation/applicationservices/kaxrangeforpositionparameterizedattribute)
and [AXSelectedTextRange](https://developer.apple.com/documentation/applicationservices/kaxselectedtextrangeattribute).
Only an unsupported/unavailable position range selects
[HID posting](https://developer.apple.com/documentation/coregraphics/cgeventtaplocation/cghideventtap).
Permission errors, malformed ranges and failed AX mutations stop the operation.
HID retains window ownership, frontmost, secure-input, permission and saved-
geometry fences plus the original-process owned release cleanup. It may move
the system pointer; live HID delivery and fatal cleanup remain unproven and are
not extra owner operations in this recheck.
