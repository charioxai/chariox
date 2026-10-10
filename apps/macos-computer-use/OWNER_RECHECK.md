# One owner step: re-grant the rebuilt helper

Miguel re-grants this build bundle in Accessibility and Screen Recording:

```text
/Users/miguel/.chariox/dev/cumac/build/Chariox Computer Helper.app
```

The October 10 rebuild has public cdhash
`d171e9751ee725169cdb777d6ffc3c46e6bdd86b` and executable SHA-256
`d999417c2a30c1b182ccaf6609406182fb11aa88737955506545a8eaa3df005e`.
The pre-grant unattended attempt returned
`refused: permission (accessibility-denied)`, so live acceptance of this new
identity remains BLOCKED until this owner step.

Use Finder to add this exact bundle in System Settings > Privacy & Security.
Remove the prior helper entry if needed, enable the rebuilt entry, and accept
quit/reopen if requested. The helper uses ad-hoc signing, so a rebuild changes
its identity. The coordinator records the new public cdhash and executable
SHA-256 before the grant and does not rebuild afterward.

That is the entire owner recheck. No pointer placement or owner clicks in a
fixture or TextEdit are needed. The coordinator runs this unattended command
from `/Users/miguel/chariox-worktrees/cumac-m0` after the grant:

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
and script-positioned pointer snapshots are checked. The helper samples AppKit
screen points once and refuses a changed live pointer as `stale-pointer`. The script restores the
pointer if nobody has moved it during the operation.

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

The October 10 owner run proved button and scroll, but caret placement was
wrong: location 84 instead of the owner's intended 126. The earlier unattended
refusal happened while Miguel was typing elsewhere and is invalid evidence.
The unattended suite replaces that ambiguous manual caret check. It proves
mapping on its own NSTextView window; separate real TextEdit compatibility,
fatal input recovery, Developer ID signing and M1-M5 acceptance remain outside
this recheck.
