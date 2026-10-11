# R11 review audit

The lane contains 18 pinned review files. None is newer than the reviews handled
in r10. No unhandled P1/P2 remains in those files. This is a source audit, not
live acceptance. Published M0-M4 heads remain unchanged in r11.

| Review | Finding and existing fix |
| --- | --- |
| #920 @286ced452 | Global HID fallback confinement, removed/refused at `eca423fb51`; AX-only saved geometry and hit-test remain |
| #920 @eca423fb5 | No further finding |
| #920 @3e5e59acf, @58dec4ba9 | Unattended watchdog `bbc9ec4dff`; fresh evidence parent `72c7d13dab` |
| #920 @b8a2a24fc | Both above resolved, no further finding |
| #920 @5cf1dc62b | Foreign-cover mutation fence `bee94b88fc`; real new-build replay still pending |
| #920 @5973ba185 | Same-process window identity and selected build/launch receipt `bc53791b18`; owner recipe includes cover cleanup and paired-build prerequisite |
| #920 @bc53791b1 | Focus/pointer takeover during warp/jitter `d30baee37d` |
| #920 @d30baee37 | Native-process single warp and permanent takeover retirement `5da701e0c3` |
| #920 @5da701e0c | Both caret runners use that same lease, `d71ca03c7a` |
| #923 @43fd9f1a6 | Unresolved owned release latch and geometry retirement `8fc71e5601` |
| #923 @8fc71e560 | Arm input uncertainty before cancellation `8208cd9d01` |
| #923 @8208cd9d0 | Authenticated no-effect/released refusals stay usable `66777ac31e`; identity-only confirmation `edfee08114` |
| #923 @edfee0811 | Both caret runner leases `3b69f43758`, same shared fix as M0 |
| #957 @777b256c4 | P1 secure classification withholds pixels; cleanup deadline/verdict, `eed91bc079` |
| #957 @b5300f01f | Admitted caret change creates damage and strict explicit cancellation oracle, carried by `bd64296785` |
| #957 @bd6429678 | Authenticated no-effect refusals `3210def588`; identity-only confirmation `50cd5fe155` |
| #957 @50cd5fe15 | No further actionable finding |

Shared fixes are already mirrored across M0-M4. The M4 source includes the
latest caret-runner fix at `ec89687e7a`. M5 starts exactly at published M4
`6041368df5fb0a46a3d619c5c53c1ea6e43fe68b` and inherits those fixes. No review
finding was silently reclassified as acceptance or assigned a new fix commit.
Existing fail-first evidence remains in the r5-r10 lane evidence directories.

R11 reran the 31 Python and 31 Node component checks on M5 and focused Swift
capture/codec checks. The actual caret-runner tests execute generated native
ownership scripts against takeover facts; the cancellation negatives reject
timeout, transport loss and failed release. The secure-classification Swift
case refuses messaging/type/missing-value failures before publication. These
checks supplement the previous red/green evidence. They do not supply the new
grant, real occlusion/input replay, fatal release or encrypted hosted receipts.
