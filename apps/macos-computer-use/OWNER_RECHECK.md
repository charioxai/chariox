# Fixture pass; real TextEdit reference waits for a re-grant

The r4 review fixes are complete. Manually launched fixtures stay open; only
`--unattended` enables the 20-second watchdog. Both runner modes supply that
flag and match it during cleanup. A 21-second lifecycle check proved manual
survival and unattended termination. The headless suite now creates its
evidence parent before `mkdtemp`, and passes with a fresh home.

The requested AX-bounds/AppKit comparison needs metadata that the granted
helper does not expose. A direct diagnostic had no Accessibility grant and
returned AX status -25211. One helper rebuild added an explicit public-document
diagnostic. It is in a separate build bundle; the original granted bundle was
not overwritten or re-signed:

```text
/Users/miguel/.chariox/dev/cumac/build/Chariox Computer Helper Recheck.app
CDHash: 425753b7ce44dd772d2e428f3bc36b634d69eaeb
SHA-256: 91150453be0ccb4a61b53ce29d7fbb1608bf86cb991fe9e61bf913007b809ba3
```

This bundle's preflight returned `refused: permission (accessibility-denied)`.
The only owner step is to grant the exact new bundle in Accessibility and
Screen Recording using System Settings. Accessibility is required by the
TextEdit drill; no Screen Recording operation is used by that drill. Keep the
original grants and bundle intact. No document opening, pointer positioning,
caret clicking or screenshot inspection is requested from the owner.

After that re-grant, run from this worktree:

```bash
python3 apps/macos-computer-use/textedit-click-drill.py --recheck-bundle
python3 apps/macos-computer-use/text-click-drill.py --suite --recheck-bundle
```

The real-app runner checks the grant before opening anything, opens a new copy
of the same 12-line public probe in Apple's TextEdit, discovers its new window,
and requires an exact file URL and exact synthetic content match. The explicit
`--public-text-drill-document PATH --ax-read` diagnostic publishes AX bounds
for indices 125 and 126, AppKit insertion references using the AX font metadata,
the actual selected range, launch-time CG/AppKit pointer samples and the window's
screen. Ordinary AX reads still return role metadata only. The runner compares
both explicit and warped-pointer clicks with those references, rejects changed
layout/focus, restores an unchanged warped pointer, and closes only the matching
unedited public document. A cleanup failure is FAIL. The new diagnostic has
compiled and passed headless checks, but its real AX reference and close path
remain unverified until the grant.

The unchanged granted helper was also exercised in real TextEdit during r4.
Both paths reported index 126 at 316,192 in the newly opened window. A temporary
marker independently appeared as `car|et` on line 05 through each path, then
was undone. These exploratory points came from the public screenshot, so they
do not substitute for the requested AX-bounds reference. The owned document
was closed. One physical display had origin 0,0, size 1512 by 982 and scale 2;
TextEdit's new window was at Quartz 181,102, size 656 by 422. Other display
arrangements remain untested.

The historical manual receipt used the older `c1b20e40...` helper, whose pointer
sample was `CGEvent(source: nil)?.location`. The granted `d171e975...` helper
uses the same AppKit pointer sampler for fixture and owner mode. Replaying the
historical reported point with the new window's 29,29 translation produced
index 84 again; an independent marker appeared as `lin|e` on line 04. This
establishes where that reported point maps in this layout, not why the old
helper sampled it instead of Miguel's intended line-05 point. That cause is
still unresolved. CG and AppKit agreed at the unattended warped points.

R4 evidence is under
`~/.codex/evidence/browser-computer-use-macos/cumac-r4-20261010/`.
The permission-blocked new runner produced
`ax-textedit-20261010-221956`. No real TextEdit reference PASS is claimed.

## Preserved granted helper and fixture results

Miguel re-granted this build bundle in Accessibility and Screen Recording
on October 10 at approximately 20:00 UTC:

```text
/Users/miguel/.chariox/dev/cumac/build/Chariox Computer Helper.app
```

The October 10 rebuild has public cdhash
`d171e9751ee725169cdb777d6ffc3c46e6bdd86b` and executable SHA-256
`d999417c2a30c1b182ccaf6609406182fb11aa88737955506545a8eaa3df005e`.
The pre-grant unattended attempt returned
`refused: permission (accessibility-denied)`. After the grant, the unattended
suite passed all four explicit caret points, all four pointer caret points,
button and scroll. No further owner action is required for this fixture recheck.
The script fix did not rebuild or change the granted helper identity.

The helper uses ad-hoc signing, so a future rebuild changes its identity and
requires a new grant. Preserve this build for subsequent drills.

No pointer placement or owner clicks in a fixture or TextEdit are needed.
Run this unattended command from `/Users/miguel/chariox-worktrees/cumac-m0`:

```bash
python3 apps/macos-computer-use/text-click-drill.py --suite
```

The drill launches only the build fixtures. It waits until its fixture is
frontmost, records public window/display/pointer geometry and helper identity,
and tests four known caret points on lines 1, 5 and 9. AppKit independently
checks each fixture point's insertion index and coordinate round trip before
publication. The line-5 points expect UTF-16 indices 125 and 126, before and
after the `r` in `caret`. The helper must report the requested point and index,
and the fixture's latest actual selection must agree. Both explicit points
and script-positioned pointer snapshots are checked. CG warp quantized the
fractional fixture x coordinates to whole screen points on this Mac. The
script allows less than one point per axis for that warp and requires CG and
converted AppKit pointer readings to agree within 0.01 point. The helper receipt
must match the sampled pointer within 0.01 point, and the fixture's actual
selection must still match the independent reference. The helper samples AppKit
screen points once and refuses a changed live pointer as `stale-pointer`.
The script restores the pointer if nobody has moved it during the operation.

The successful October 10 run has evidence under
`~/.codex/evidence/browser-computer-use-macos/ax-text-click-20261010-220350`
and `ax-fixture-20261010-220356`. The prior `pointer-warp-coordinate-mismatch`
was the script's 0.6-point tolerance rejecting a 0.61328125-point quantization,
before a pointer-path helper launch. Both coordinate readings and the unchanged
helper agree on this single 1512-by-982-point display with backing scale 2.
Other physical display arrangements were not exercised.

`open -W` can print `Unable to block on application (GetProcessPID() returned
18446744073709551016)` on short-lived identity queries. The value is unsigned
`-600`, Apple's `procNotFound`. A separate identity probe returned a complete
receipt and open exit 0 with the helper already exited, consistent with a race
in open's wait setup. This warning is separate from caret mapping; application
completion still requires the helper receipt and actual fixture selection.

The same suite checks an observed fixture button counter increment through
AXPress and an observed vertical scroll increase through AXScrollValue. Every
runner-owned fixture has an opt-in 20-second exit timer, and the runner closes only its own fixture.
Evidence goes under `~/.codex/evidence/browser-computer-use-macos/`.

GUI runs are allowed by default. While the lane's GUI_HOLD file exists, the
lane writes GUI_READY.md and polls every 60 seconds. It starts when the hold
disappears, GUI_OK appears, or the authorized 20-minute wait expires. A focus
loss invalidates the run. A refusal or an unproven completion is never PASS;
`refused: permission` is BLOCKED. Refusals include a public reason tag without
application text or framework error contents. An owned-input refusal still
requires the owner to clear the application's tracking state before more input.

The earlier October 10 owner run proved button and scroll, but caret placement
was wrong: location 84 instead of the owner's intended 126. Its cause remains
unreproduced. The earlier unattended
refusal happened while Miguel was typing elsewhere and is invalid evidence.
The unattended suite complements that ambiguous manual caret check. It proves
mapping on its own NSTextView window; separate real TextEdit compatibility,
fatal input recovery, Developer ID signing and M1-M5 acceptance remain outside
this recheck.
