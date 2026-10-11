# R12 review fixes

The r12 fixes cover four P2 findings from #962/#963. The late #964
@6101151bd review contains one P1 and two P2 findings, addressed in
`COMPUTER_USE_MACOS_R13_REVIEW_FIXES.md`. The earlier no-findings statement
was incorrect.
Each finding has one source commit, mirrored to every milestone containing the
implementation. M0-M2 have neither M3 physical input nor M4 target references;
their heads stay unchanged.

| Review and finding | M3 commit | M4 commit | M5 commit |
| --- | --- | --- | --- |
| #962 @d4a7743ac, explicit release of physical ownership | 3a5603f82 | b5ffe7affe | 8d2e0d65ce |
| #962 @d4a7743ac, released physical pause loses pairing | da338f4db | 271ff91f22 | c4673581fb |
| #962 @d4a7743ac, cleanup lacks destination fencing | 5c2ee2eb2 | 20002214e | 309a368f44 |
| #963 @6041368df, expiry during AX validation | absent | 8ea542eaa | 87a4b99758 |

Physical owner release now uses the authority terminal release path. The ledger
keeps physical ownership through helper reset, including failed reset, and
rechecks terminal admission before clearing ownership. It does not request an
ordinary competing actor takeover. The kernel request regression covers valid,
stale, unauthenticated and retired-terminal release. The isolated production
release method also proves failed reset retains ownership.

An authenticated no-press/released physical refusal publishes safe pause status
before the production takeover callback cancels its Action. Both the independent
RequestFence watcher and response path keep that pairing when authority remains
valid. Concurrent authority retirement still closes it. The integration regression
uses the real actor ledger and actual cancellation signal code, waits for the
watcher, and checks move/click cancellation, reset, explicit release and the same
generation. Unconfirmed input and transport failures retain their fences.

Normal input and cleanup now share one destination fence. Injected AX and
WindowServer reads exercise the production release check, rather than replacing
releaseFence. It rejects programmatic focus loss, foreign and same-process cover,
mouse/key geometry changes, hit mismatch and key focus changes. No up is posted;
owner reset stays latched. A cancelled Action can release at a proven destination
without restoring its mutation authority. Observer and posting permission checks
remain required.

Target consumption preserves the original absolute two-second deadline. Resolution
checks expiry and current observation again after validation. After consumption,
final native validation runs before the deadline/revision check immediately next
to AXPress. Two injected-clock regressions expire inside resolution and final
validation and observe zero effects. Failed reference replay invalidates the
pending mutation; a fresh consumed target still dispatches.

## Validation

Evidence is under
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r12-20261011`.
Behavioral fail-first logs are `release-red.log`, `pause-red.log`,
`destination-red.log`, `target-red.log` and `target-final-red.log`.
The corresponding green logs and combined suite logs are retained.

- 37 isolated actual actor/adapter/release checks pass. The small host shell and
  fake socket peer are component seams; neither is live acceptance. Production
  release, actor, action ledger, adapter, RequestFence and cancellation signal
  code are exercised. Full kernel test execution belongs on b3.
- Five input, six target and six capture executables pass, plus policy checks.
  No native event posting, capture of owner windows or helper launch occurs.
- 32 Python checks and 28 M5 Node checks pass. Structural checks follow the shared
  destination fence and retain explicit native reader wiring assertions.
- Complete helper source typechecks for arm64 macOS 14. Swift builds run serially
  with `-j2`, under critical-pressure and 3 GiB owned process-group RSS guards.
- Guarded `cargo check --tests --offline -p chariox-kernel -j2` passes.
  This compiles the real kernel request tests; it does not execute them.
  Inventoried R12 compile-only output is removed; logs and focused outputs remain.

OWNER_RECHECK remains one conditional matching-build owner appointment. It adds
same-generation physical release/resume and deadline checks. Destructive
post-down destination-loss variants belong at the final owner-reset gate. Any
unrun variant remains BLOCKED. The coordinator must supply the matching combined
build, real protection, fixture and transport hooks before that appointment.

No public serialized shape, allocation, crypto or Cloud change occurs. Local 497,
peer 106 and frozen display #950 remain fixed. No push, CI, Keychain, TCC, sudo,
provider/hosted run, helper build/sign/replacement or installed bundle launch
occurs. The r6 helper executable hash is unchanged:
`b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028`.

Historical accepted rows retain their heads, statuses and dates. M2-M5 remain
BLOCKED. These source regressions do not establish a new live PASS.
