# M2 native capture attachment checkpoint

`cu/macos-m2` remains disabled and unstaged. It starts from the published M1
revocation branch. The protected fixture-only SCK capture is now connected to a
private helper `host.display` source contract with subscribe, sample and close.
The kernel must supply `_attachment_owner` only after caller admission. This
private method has no public KernelComputer route yet. It must not be enabled
by forwarding caller JSON directly or by inventing a browser tab identity.

Each source binds the explicit signed fixture PID/window, pairing generation,
Quartz origin, native pixel size and 1x/2x scale. Real-app capture remains refused
until M4. A selected-window filter excludes everything outside the admitted
fixture, including Chariox helper, approval and viewer windows. Registry targets
or values mask the entire window; unknown protection withholds capture. Secure
AX geometry receives opaque masks before encoding, damage comparison or cache
retention. Output is SDR sRGB, without audio, microphone or cursor pixels.

The source uses SCK complete-frame callbacks at 30 Hz, queueDepth 3 and one
latest pending buffer. New samples replace older pending samples. Incomplete or
unproven metadata drops pending pixels. A closed/failed stream never accepts a
later callback. Presentation timestamps, content size and scale are checked;
old frames cannot satisfy a fresh sample request. Static streams use independent
full SCK screenshot readback for byte-exact verification. Capture errors fence
the stream rather than switch to a different capture API. Policy changes close
the stream and flush protected bases. Pairing EOF, geometry loss, Stop and
capture permission health failure clear source state. Health retires idle
subscriptions and stops their stream. There is no raw-frame log or disk output.

At most two private subscriptions retain one protected RGBA base each, at most
16 MiB per base. Each has one outstanding credit and a 60-second idle lease.
Foreign owners, overlapping credits and expired/closed subscriptions refuse
before capture. The helper compares every protected byte in clipped 64-pixel
tiles, emits no duplicate pixels for a quiet surface, and encodes changed tiles
as lossless PNG. More than 260 tiles or 32 KiB of encoded tile damage becomes a
full exact PNG. PNGs are capped at 512 KiB and serialized packets at 700 KiB to
leave room inside the existing 1 MiB private channel envelope. Two caches and
bounded packets do not establish a measured peak-memory or performance PASS.

Unknown acknowledged sequence, explicit repair, new source/geometry/protection,
failed encoding or withheld publication forces a full exact base. Final
publication rechecks owner, lease, source revision and authority after encoding.
A fail-first test found and fixed a protection-change gap at that final fence.
The headless suite independently decodes native PNG tiles, checks an edge pixel,
reconstructs exact damage and verifies full repair. Additional coverage includes
quiet frames, slow viewers, overlapping credits, foreign owner denial,
negative-origin 2x masks, whole-window canary removal and callback retirement.

## Shared viewer dependency

The checked-in shared display route remains browser-specific. Its registration
accepts tab IDs and numeric browser generations; the presenter validates a
1280x800 CSS viewport. `KernelComputerCommand` has no display operations. The
macOS plan reserves the multidomain display union and numeric protocol versions
for coordinator allocation. This checkpoint allocates none and changes no
public DTO, version, snapshot or client minimum. The allocated contract commit
and version were requested during r7 and have not been supplied.

Full M2 still requires native surface registration, caller-bound kernel
attachment and presenter binding through the existing encrypted transient
kernel/relay event route. Reuse `transport/kernel_browser_display.rs` to remove
frame bytes from response/history/trace, existing attachment authorization and
encrypted relay delivery. Do not add a second encryption stack, put private
frames into ordinary request replay, or use a Mac window as a browser tab.
After allocation, update the shared version and snapshots/hashes, dependent
client minima and focused relay drill together.

No r7 helper rebuild occurred. The preserved r6 bundle is awaiting Miguel's
re-grant. All r7 helper changes are batched in source for one later rebuild and
new grant. `OWNER_RECHECK.md` gives the exact retained-r6 fence replay, in-flight
kernel revoke, and paired capture/native damage replay. That replay uses live
r6 PNGs and the compiled r7 private attachment. It cannot validate r7 callbacks
in a granted helper or encrypted shared-viewer delivery. Those live gates remain
BLOCKED; no capture/input was attempted during r7.

## Remaining milestones and rollback

M3 needs physical input, owned holds, local-human observation/self-tags, seat
delivery and fatal release/reset proof. The paired bounded AX drill does not
prove those capabilities. M4 needs AX refs/revisions, protected Vision fallback,
private coverage, OCR ambiguity and real-app no-leak tests. M5 needs codec
attachment, measured presentation/restore, Developer ID/notarized upgrades and
the Web/local TUI/remote TUI/hosted-relay loss, sleep/wake and lock matrix.

Rollback removes the explicit helper/scope configuration. Cleanup is limited
to inventoried compiler output and runner-owned processes. Durable kernel homes,
keys, backups, credential profiles and reviewer infrastructure are preserved.
All builds stay under the approved build root with `-j2`; no CI or push.

R7 reviews of #920 @5973ba185, #923 @43fd9f1a6 and #957 @777b256c4 arrived
before handoff. Fail-first source fixes now reject same-process window covers,
verify non-default build launches, preserve owner-reset-required after a failed
owned release, retire invalid geometry pairings, withhold capture on failed
secure-field classification, and require successful bounded owned-window
cleanup for a fence PASS. The actual adapter harness passes 15 tests; the
paired reply and secure-classification tests pass. These source changes are
also retained on the affected M0/M1 branches. The r6 helper/kernel remain
unchanged, so the pending re-grant suite cannot validate the new review fixes.
Those exact new-binary live gates await the next combined rebuild and grant.
