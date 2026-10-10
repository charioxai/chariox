# Remaining macOS acceptance rows, r5

Run on Miguel's arm64 Mac, macOS 26.5, on 2026-10-10. M0 checkout and published
#920 head: `b8a2a24fc06c9d279e6cb6c4fd259f96d7f43d30`. M1 checkout and kernel build
#923 head: `3d8a6374c91e8acb35f7cf980a296f72c7058345`.

The existing granted M0 helper was reused without rebuilding. Its CDHash is
`d171e9751ee725169cdb777d6ffc3c46e6bdd86b`; executable SHA-256 is
`d999417c2a30c1b182ccaf6609406182fb11aa88737955506545a8eaa3df005e`.
This identifies the runtime tested; it does not claim the later opt-in TextEdit
reference diagnostic was rebuilt into that helper. The ungranted Recheck bundle
and the historical reported caret sample of 84 remain deferred and unresolved.

Evidence is outside Git at
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r5-20261010/`.
The local coordinator board's row descriptions were read before execution.
The requested r5 scenarios also require foreign-cover and stale-pointer checks.

| Tracker id | Result | Measured behavior or exact blocker |
| --- | --- | --- |
| computer-use-macos/M0 | FAIL | The foreign-cover fence failed. Capture, AX, a synthetic Unicode round trip, button, scroll and caret components ran. Full grant/permission revocation and fatal owned-input recovery remain unproven. |
| computer-use-macos/M0-CAPTURE | PASS | The allowed disposable TextEdit window produced independently inspected before/after PNGs, 1312 by 844 pixels for contemporaneous 656 by 422 point geometry at scale 2. Foreign code identity and the inactive selected window refused capture; neither refusal wrote a PNG. |
| computer-use-macos/M0-FENCES | FAIL | A visible foreign nonactivating panel covered the caret point while target focus stayed unchanged. AX caret mutation still changed the fixture selection from 0 to 126. A separate live pointer movement refused with `refused: target (stale-pointer)` and left the selection unchanged. |
| computer-use-macos/M0-REVOCATION | BLOCKED | The standing no-TCC-change rule prevents manual OS permission revocation in this unattended run. The granted standalone helper has no kernel grant/revoke channel. #923 identity-seat Revoke all stops its helper, but that seat has no input; it cannot prove interruption of live M0 input. |
| computer-use-macos/M1 | PASS | The actual #923 kernel paired a separate build helper and verified health, lease, Revoke-all Stop, fresh/stale epochs, helper crash, helper termination, kernel exit and identity denial in both directions. Focused adapter and pairing tests accompany the live receipt. This is the M1 identity seat; capture and input remain unavailable. |
| computer-use-macos/M2 | BLOCKED | Missing the macOS capture implementation behind the paired seat, encrypted shared-viewer attachment, damage publication and protected exact repair. M1 reports capture unavailable and refuses snapshot. Standalone SCK window capture is supplementary. |
| computer-use-macos/M3 | BLOCKED | Missing paired native mouse/scroll/keys/Unicode delivery, physical-human activity fencing, input self-tags and fatal owned-press reset. M1 reports input unavailable. Standalone AX/PID input and inert cleanup tests do not establish these capabilities. |
| computer-use-macos/M4 | BLOCKED | Missing Mac target-reference/revision integration, Vision OCR fallback, private-region masking and protected ambiguity/no-leak scenarios. Current standalone reads return role metadata only. |
| computer-use-macos/M5 | BLOCKED | Missing VideoToolbox/software codec attachment, Developer ID/notarized install and upgrade, owner onboarding and the Mac Web/local-TUI/remote-TUI matrix. Functional and performance acceptance are both blocked. The tested helpers are ad-hoc signed. |

## Independent checks

`real-digit/standalone-result.json` records the real TextEdit round trip. Its
synthetic document alone was queried through TextEdit Apple events. The exact
string `7é é 😀 中` was inserted, with the decomposed accent retained. An eleven
emoji request refused with `refused: text (invalid-text)` and independent text
readback stayed unchanged. PNG inspection and the contemporaneous window
geometry are recorded in `independent-verification.json`.

Earlier attempts remain in evidence. The first lost focus and refused input.
Later first-letter trials changed `é` to `É` in TextEdit, so those exact-string
checks failed. The digit-prefixed case avoids that app substitution without
changing app preferences. An exploratory append-position attempt also failed
its expected caret index. None of those attempts is relabeled PASS.

The decisive covering-window case is `fences-bundle/result.json`. The separate
public panel was launched from a build app through LaunchServices. Independent
WindowServer geometry places its layer-3 rectangle at Quartz 313,511 with size
121 by 120, above the layer-0 fixture. It covers the requested point
373.50390625,571. The fixture remains frontmost and its own AppKit selection log
changes from 0 to 126 after the helper returns a successful AX caret receipt.
The preceding non-bundle panel attempts did not establish a visible cover and
are invalid for this scenario. The valid panel case establishes the failure.

The stale-pointer check moves the real pointer during the request and observes
the tagged refusal plus unchanged fixture selection. Policy tests exercise
owned-release cleanup and `owned-release-failed` using inert events and an
injected posting callback. Live fatal cleanup and permission loss during a held
input pair were not established. No policy test is presented as a live drill.

The eight AppKit-referenced fixture caret cells and button/scroll rerun passed at
`ax-text-click-20261010-223928` and `ax-fixture-20261010-223934` under the evidence
parent. The already accepted M0-CLICK and M0-SCROLL tracker rows are retained;
this update covers the nine remaining rows.

## M1 and cleanup

The freshly built #923 kernel passed 19 focused Rust adapter/pairing tests.
Current-head Swift policy checks, 11 structural checks and 14 Python runner
checks also passed. These are local checks, with no GitHub CI run.

`m1/result.json` records the real kernel/helper identities, generations, PIDs,
LaunchServices parent PID 1 and ten lifecycle checks. The runner was adapted to
compile only a separate pairing helper under the permitted build root. It never
launches installed bundles or rebuilds the granted M0 helper. The paired helper
is identity-only and makes no TCC, capture or input call.

Every owned GUI window closed. The pre-existing TextEdit `caret-probe.txt`
remains. Owned helper, fixture, public-cover and kernel processes ended; public
evidence remains. R5 build intermediates were inventoried and removed; the
verified kernel remains under the permitted build root at
`r5-retained-kernel/chariox-kernel`. The normal drill kernel generated relay identities, so its
state was inventoried and retained in protected durable storage instead of
being recursively deleted. No private payload was printed or put in evidence.
Existing owner credentials, protected key assets, reviewer state and relay
infrastructure were preserved. Retention locations are operator-local notes.

No sudo, TCC changes, Keychain access, installed-bundle launch, push or CI.
This commit records acceptance results only. It does not fix the cover failure
or complete M2-M5.
