# M5 source checkpoint

R11 starts `cu/macos-m5` exactly at published M4
`6041368df5fb0a46a3d619c5c53c1ea6e43fe68b`. M0-M4 published branches retain their
heads. No push or CI. See the r11 review audit for all supplied P1/P2 closures.

## Codec attachment

`MacVideoEncoder.swift` accepts immutable protected sRGB RGBA frames from the
private caller-bound CaptureAttachment, converts to a protected BGRA buffer,
and requests VideoToolbox H.264 Baseline 5.1. It checks the actual hardware
property rather than assuming acceleration. Realtime operation, no reordering,
zero delayed frames, requested bitrate/data-rate limits and a 200 ms callback
deadline bound each admitted encode. Raster and output packet limits remain.

The existing shared `avc1.420033` whole-video packet carries Annex B SPS/PPS and
an independent keyframe. No decoder configuration, wire version or crypto is
added. Independent frames prevent abandoned credits from dropping a dependency
chain. The private helper subscribe receives the kernel's codec/bitrate offer.
The protocol 497 native presenter forwards only target/codecs and can explicitly
choose AVC while retaining PNG. Browser bitrate options do not cross that native
command boundary. The kernel owns the native bitrate. Default stays
PNG and the experimental Mac observer remains feature-disabled.

Bootstrap, lost base, explicit repair, geometry/protection change and 200 ms
since the first lossy packet require a full exact protected PNG. Quiet credits
send no duplicate pixels before that repair deadline. Encoder unavailability
uses portable software PNG/tile encoding. After any video, codec failure must
send a full PNG because tiles cannot restore untouched lossy pixels. Odd native
sizes refuse VT without stretching or changing the surface. A final authority,
revision, viewer and packet-budget check still withholds stale work.

The current encoder emits only independent frames. This intentionally avoids
new dependency policy while the real performance gate is pending. It may cost
bitrate and cannot claim 30 Hz or an uplink budget without measurement. Hardware
encoder creation/submission remain OS calls; the callback deadline is not a
proof of worst-case OS execution time.

## Real client/relay matrix contract

`client-relay-matrix.mjs:runMacClientMatrix` enumerates 54 immutable ids. Each of
Web/local-TUI/remote-TUI runs VideoToolbox and PNG with official Codex, Claude,
and OpenCode providers, plus Retina/negative origins, loss/reconnect,
lock/sleep/wake, permission revoke, helper crash and signed upgrade cases.
Providers must use the normal kernel-managed official harness paths.

Coordinator supplies `runCase(case, {signal})`, `cleanup(case, {signal})` and
public matching helper/kernel SHA-256 pins. Callbacks use current admitted
kernel/hosted-wss transports and independent native/public evidence. They must
honor abort, retain failures and perform owned release/cleanup. Case deadline is
at most 60 seconds; cleanup has five seconds. Missing callbacks/pins block all
rows. An explicit blocked dependency stays BLOCKED. Any functional/cleanup
failure stops further mutations, even when subsequent callbacks are available.

The case receipt must match client/route/provider/scenario/codec and both pins.
It records `live:true`, actual backend `videotoolbox-hardware`,
`videotoolbox-software` or `png`, independently derived protected source and
presented pixel SHA-256 values that match after exact settled repair, verified
native effect, absent protection canary, stale-epoch refusal, no input replay,
scenario proof and confirmed owned cleanup. The public evidence path must be
under `/Users/miguel/.codex/evidence/`. Do not send keys, auth headers, private
screen content, field values or provider profiles to this module.

Callbacks must assert the scenario against real receipts. For Retina/negative
origins, cover both 1x/2x and window movement. For recovery, establish no stale
pixels, refs, grant or replay. For upgrade, verify Team/identifier admission,
foreign-signature refusal, preserved durable assets and owned press recovery.
Source/presented hashes are of the final protected exact frame, not lossy motion.
Mocks in the component tests never constitute real acceptance.

Performance is separate from functional status. At least 30 measured samples
must show input-to-visual P50/P95, capture-to-publication P95, authoritative
presented fps, settle time, egress bitrate and peak owned RSS. The runner uses
shared display/report.py targets of 30 fps, RTT+50 ms input P95 and settled
repair below 300 ms at 1x or 500 ms at 2x. Publication P95 is bounded at 50 ms,
actual egress at the offered bitrate and owned RSS at 3 GiB. Missing/slow metrics
fail performance while retaining functional evidence. This checkpoint measures
none of those live thresholds. The 60 Hz and supported OS campaigns remain
additional release gates.

## Owner and evidence

R13 closes the late #964 P1 and both P2 findings. See
`COMPUTER_USE_MACOS_R13_REVIEW_FIXES.md` for fail-first evidence. The codec
driver accepts `CUMAC_BUILD_DIR` and `CUMAC_EVIDENCE_DIR`, creates both roots
and passes the same public H264 artifact path to its encoder and decoder.
The executable policy tests remain ad-hoc signed in unsigned owner mode;
only release bundle executables use the unsigned linker option.

`apps/macos-computer-use/OWNER_RECHECK.md` is one conditional owner appointment.
If ready, owner Developer ID signing/notarization happens before publishing pins
and granting the one combined M0-M4 helper. The unattended suite follows; fatal
M3 press/death runs last. The exact owner commands and safe dry-run checklist
are in `COMPUTER_USE_MACOS_M5_SIGNING.md`. No Keychain operation ran here.

Evidence root is
`/Users/miguel/.codex/evidence/browser-computer-use-macos/cumac-r11-20261011`.
The real selected encoder reports `videotoolbox-hardware`. The independent
software decoder verifies SPS/PPS format, dimensions and protected black pixels.
The 31 Python and 31 Node checks pass. Six focused capture executables pass;
the complete helper source typechecks for arm64 macOS 14. Removing the full-PNG
fallback after video makes the focused codec regression fail, then the final
source passes. The paired subscription codec receipt also went red before its
AVC reporting fix and passes after. API-absence compile-red output is supplementary only. Owner
command blocks pass `bash -n`; no signing/notary command was executed.

Focused checks use `-j2`, one Swift compiler at a time, critical pressure and a
3 GiB process-group RSS guard. Public generated raster only; no desktop capture.

The retained r6 executable SHA-256 remains
`b02c5ba5aa6619698ae045a7fff4f4403b7fcf79ff6f09a981143a6ff8282028`.
No helper was built, signed, launched, granted or installed. No provider,
hosted transport, kernel test, relay/reviewer change or durable key use ran.

M0 historical FAIL and previously accepted receipts are preserved. M2-M5 remain
BLOCKED. M5 now has codec/matrix source and component evidence, but lacks real
functional/performance acceptance, Developer ID/notarized release, stable
production admission, install/upgrade/offline packaging and supported OS proof.
M4 protection/integration prerequisites still block the combined suite. Local
497 / peer 106, frozen display #950 and Cloud #325 remain unchanged.
