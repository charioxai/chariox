# M3 native input source checkpoint

R9 starts `cu/macos-m3` from published M2 `bd64296785ef05d802a8ce5a9a9882e085b20d34`.
It retains frozen display #950 `f04b4006b3b8b1b833560022128b0345db873e5a`
and local protocol 497 / peer 106. No public request, event or crypto shape
changes. Private bootstrap and pairing status carry the experimental input flag,
physical pause and authenticated released-input status. The adapter consumes
those fields; the shared actor snapshot projects the physical Mac owner.
499 / 108 remain unallocated. Cloud#325 `536bb5b1a` remains the frozen reference.

Physical input is off unless `CHARIOX_MACOS_COMPUTER_INPUT=1` is supplied to the
next matching kernel. The path admits only the exact signed public fixture.
Real owner windows refuse. Supported shared input kinds are move, left/right/
middle click, vertical scroll mapped to 40 pixels per shared step, a limited set
of navigation/deletion keys, and those keys held for at most 100 ms. Arbitrary
keycodes, shortcuts, long holds and drag refuse. Existing bounded Unicode text
remains a separate experimental PID-event path with its original target budget.
This checkpoint does not enable general native typing or claim its inertness.

The listen-only session tap records no human keycodes or text. Expected helper
events require their private marker, source PID and event class/location; the
marker supplies classification only. Unexpected movement, button, scroll, key
or flag activity latches a local pause before the next dispatch. Tap disablement,
missing listen access and secure input prevent dispatch. No permission request
API is called. A detached pairing reader preserves coalesced requests while the
main run loop delivers the tap. Every physical event repeats the authority,
permission, target/focus, geometry, visibility and observer fences.

The private paired pause reaches the existing human actor/takeover ledger. Agent
Actions are fenced locally before that projection; queued input cannot rely on
heartbeat timing to bypass the pause. Ordinary failed-Action reset never resumes
control. Only an authenticated authority-owner release can ask the helper to
clear a pause, after the observer and foreign-held-input checks. A fresh target
admission is still required. No idle timer or automatic refocus resumes input.

Mouse/key downs are paired within an Action. Cancellation attempts only the
helper-owned release under a separate safety check. An unconfirmed release
latches owner reset. The kernel arms uncertainty before physical clicks or key
input and retains it on fatal helper death, cancellation or missing confirmation.
Stop, Start and reset cannot clear that latch. Authenticated no-press/released
refusals can clear uncertainty; oversized text, unsupported input and permission
refusal therefore leave the seat usable. No all-keys-up reset or target-app kill
is used. Fatal live recovery and physical owner reset remain unverified. The
in-memory kernel latch does not prove that restarting a kernel clears OS state.

The latest reviews of #920 `d30baee37`, #923 `8208cd9d0` and #957 `bd6429678`
contain P2s, no P1s. Single pointer warps now recheck scope and last position in
the native process. Any reported ownership loss permanently retires the runner
lease through cleanup. The no-effect refusal fix is mirrored in M1 and M2;
pointer fixes are mirrored in M0, M1 and M2. These are local commits, not pushed.

Evidence is under
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r9-20261011`.
Fail-first logs reproduce right/middle release loss, fatal physical-click
uncertainty, paired no-effect refusal and runner restoration after takeover.
New observer/native-input/paired-input APIs also retain missing-implementation
compile failures. Mutation-negative adapter checks verify that omitting pause
projection or released-input status fails the regression. None is live acceptance.

Headless checks cover the paired serialized requests, dispatch cancellation and
release, physical pause and explicit release, no-effect refusals, pointer script
execution in a Node VM, and strict native drill outcomes. The actual Rust Mac
adapter is compiled and tested in a focused harness; full kernel/test sources
are compile-checked with the critical-pressure/3 GiB owned-RSS guard. Whole-kernel
execution belongs on b3. No helper bundle was built, signed or launched in r9.

`native-input-drill.mjs:runNativeInputDrill` is the unattended M3 runner boundary.
It needs the coordinator's real admitted agent/owner transports, independently
observed fixture selection, foreign-event/actor receipt, verified fatal helper
press hook and owned-seat cleanup. Prepare the fixture with
`prepare-seat.py --input-drill`; its value-free local event monitor supports the
fatal press oracle. The fatal hook must observe key-down while the bounded
100 ms Action is pending and kill only the verified paired helper. A timeout,
generic failure, missing press or unverified helper cannot pass. Run this gate
last; preserve FAIL receipts and do not replay input to manufacture PASS.

M3 remains BLOCKED on a matching helper/kernel build and grant, listen-event
permission validation, adversarial focus/physical/remote takeover, owned physical
release/death proof, full input/Unicode/drag/two-axis-scroll coverage and real
Web/local-TUI/remote-TUI/provider routes. M2's hosted viewer and M4 protection
gates also remain open. These components are not ready for owner staging.
