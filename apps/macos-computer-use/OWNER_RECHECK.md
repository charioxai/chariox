# Granted helper: unattended recheck complete

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
fixture has a 20-second exit timer, and the runner closes only its own fixture.
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
The unattended suite replaces that ambiguous manual caret check. It proves
mapping on its own NSTextView window; separate real TextEdit compatibility,
fatal input recovery, Developer ID signing and M1-M5 acceptance remain outside
this recheck.
