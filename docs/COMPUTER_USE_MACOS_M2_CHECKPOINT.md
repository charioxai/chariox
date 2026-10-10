# M2 first checkpoint: paired protected capture source

Branch `cu/macos-m2` starts from M1's r6 revocation checkpoint. M2 remains
disabled and unstaged. This slice adds on-demand paired ScreenCaptureKit capture
of one explicit signed public fixture window. The existing Computer screenshot
result shape is reused. No public command/event DTO or protocol version changes.
The selected window filter omits audio and cursor pixels. Window position, scale,
AX secure-region coverage, kernel connection and permission are checked before
and after capture. Secure regions receive opaque masks before encoding or damage
comparison. Registry protection masks the whole window; unknown protection or
coverage withholds capture. Real-app capture stays refused until M4 coverage.
Frames stay in memory, never in helper logs or disk. Encoded output is at most
512 KiB and protected RGBA state at most 16 MiB. Snapshot peak memory has not
been measured; the bounded objects do not establish a runtime memory PASS.

The private attachment seam retains one protected RGBA base and compares exact
64-pixel tiles. It emits an exact full base for a new source/protection/geometry,
an unknown acknowledged sequence or explicit repair. Unchanged frames advance
no sequence. Edge tiles are clipped. Invalidation drops the retained raster.
The source returns a protected PNG and a private publication description; it
currently encodes a full PNG even when only some tiles changed. Damage tile
encoding, subscriber credits and periodic streaming are the next M2 slice.

This is not an encrypted shared-viewer acceptance result. The existing
`KernelComputerCommand` has no native display-subscribe/attachment operations.
The existing Browser display paths bind `tab_id` and numeric browser generation.
M2 must extend the agreed multidomain display surface registration instead of
inventing a Mac tab ID. Coordinate any serialized contract change with the
coordinator's protocol allocation, snapshots/hashes and dependent minima.
Reuse `transport/kernel_browser_display.rs` for transient frame-event extraction
and no history/trace replay, the caller-bound Browser attachment authorization,
and existing encrypted relay delivery. `transport/secure_display.rs` is the
existing encrypted display channel for the Room/Selkies path; select the agreed
multidomain presenter route rather than adding a second encryption stack.
The relay remains opaque. Local public-fixture screenshots or passing encryption
unit tests alone cannot prove this delivery path.

Next M2 needs actual shared viewer registration, native surface/geometry metadata,
protected damage tile output, credits/acknowledged base validation, exact repair,
SCK stream callbacks, exclusions/protection-change flushes and encrypted hosted
relay/Web/TUI drills. Capture a clean public fixture with the re-granted helper,
inspect exact pixels independently, and then connect that same source to the
shared presenter. No hardware/video or full-desktop scope is enabled here.

M3 must add physical input/owned holds, local-human activity and self-tags,
serialized seat delivery, fatal owned-release reset and takeover drills. Use
this source's bound origin/scale for canonical pixels and expire all targets
on source or transform changes. A failed bounded M0 owned release now poisons
the paired input session; reset refuses instead of claiming it released input.
The helper cannot silently clear that state or replay the action.

M4 must replace fixture-only coverage with bounded AX refs/revisions and
conservative real-app private/secure coverage, Vision on protected pixels only,
OCR ambiguity, opaque AX and no-leak canary tests. M2 and M3 stay disabled until
those gates pass. Missing geometry must withhold the affected frame.

M5 must attach VideoToolbox/software codec selection, measure authoritative
presentation and exact restore, provide the protected Developer ID/notarized
upgrade path, and validate owner onboarding, Web/local TUI/remote TUI and hosted
relay loss/reconnect, sleep/wake and lock. No signing-key or Keychain work occurs
in this checkpoint.

Regression tests cover exact bootstrap, edge damage, quiet frames, missing-base
and explicit repair, protection/source invalidation, raster budget, opaque mask
placement at 2x with negative origins, and no remaining synthetic canary pixels
under a whole-window mask. Native permission-backed capture and input drills
wait for the one final helper re-grant. Live acceptance remains BLOCKED.
Rollback removes the explicit Mac helper and scope configuration. Cleanup is
restricted to inventoried build intermediates and runner-owned processes; no
recursive runtime-state removal is authorized by these scripts.
