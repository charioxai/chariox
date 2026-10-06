# MP-08/MP-10/MP-11: phase 16 bounded stripe motion

Assigned base: multidomain round 2 `6dde21a8c10c9ef2b9f7f0a271cace00591f0bd1`.
The phase-15 source is retained locally at `md/display-phase15-retained`.
The replay preserves round-2 private CDP pipes, admission/grant revocation,
typed refusals and DOM mirroring. Coordinator allocation: local 447, relay peer 90.

## MP-08/MP-10: motion and exact scheduling

An owned XDamage readback compares complete raster bands with exact bytes;
XXH3 is a scheduling prefilter. Equal hashes never authorize observation or
suppress different bytes. The native helper publishes changed readbacks using
three private mapped buffers, with serial-bound leases. The encoder holds a
lease until it has consumed the mapping. Node transfers bounded metadata rather
than multi-megabyte raster bytes through its main thread. Startup attestation
reads one immutable snapshot and compares native RGB against protected CDP RGB.
The pool lives inside the disposable owned-display directory, files are 0600,
and the directory is 0700. Encoders verify owner, mode, type and length before
mapping. All files are removed after the owned helper exits.

Motion does not run Node PNG, zlib or whole-raster cryptographic hashing.
Native readback compares exact bands independently of XDamage hints. SHA-256
remains for artifact/source identities and exact PNG verification, outside the
motion scheduler. The protected CDP fallback still decodes its JPEG fingerprint
in the helper; it is outside the admitted native fast path and remains a
performance limitation. Hashes are never credential, source-scope, document,
protection, grant or relay-admission authority.

Exact refinement begins after 300ms quiet. Encoding PNG tiles and decoding the
exact capture run in the existing pixel worker. Pending exact work does not
hold the kernel backend mutex while waiting. Document, policy, input epoch and
source serial are rechecked before delivery. New input retires old exact work;
quiet scheduling cannot weaken the observation barrier.

Microbenchmark: 100 1080p readbacks, same host. SHA helper to XXH3 helper CPU:
dense 466.0ms to 75.3ms; sparse 46.3ms to 23.6ms; idle 8.87ms to 10.87ms.
This measures the helper only, not end-to-end latency or acceptance.

## MP-08/MP-10/MP-11: protocol 447/90

A viewer explicitly offers `chariox-stripes-v1` plus an ordinary supported
codec and PNG. Without that offer the existing whole-frame path remains.
The frame kind is `stripes`, with the existing subscription, generation, tab,
document, canvas geometry and monotonically increasing outer sequence. It also
contains `base_sequence` and one to eight full-width rows. Each row has `row`,
`y`, `height`, `codec`, `key`, `sequence`, `reference_sequence`, `data_base64`.
H.264 uses `avc1.420033`; the requested BSD software comparison additionally
uses `vp8`. The relay transports encrypted opaque events as before.

Eight even-height rows partition the complete canvas. Only changed rows encode.
Exact byte comparisons guard fast-hash collisions. Each row owns its own codec
reference chain. Dropped queued rows force only those rows to independent keys;
full navigation/protection/source retirement resets all rows. Fresh document
bootstrap requires a complete independent cover. Client validation rejects
lost row references, overlap, dimensions, bounds and wrong canvas base before
any draw. Every decoded row is validated before one synchronous draw commit.
Exact PNG/tile settle remains on the same protected capture path. An unchanged
readback creates no motion packet; idle measurement distinguishes media bytes
from existing control credits.

No cursor or copy-rectangle protocol was added. The implementation is original;
no pixelflux dependency or upstream source was copied.

## MP-11: encoder choice and licensing

`CHARIOX_BROWSER_DISPLAY_SOFTWARE_ENCODER` selects `libx264`, `libopenh264`,
or `libvpx`; codec negotiation must match H.264 or VP8. Encoding runs in the
separate Python helper; GPL libraries are not linked into the kernel process.
The choice is pluggable, not a distribution approval. x264 is GPL; OpenH264
is the BSD H.264 software alternative; libvpx is BSD. Separating a process does
not remove the distributed dependency's obligations.

The supplied PyAV build exposes OpenH264, but FFmpeg's OpenH264 adapter does
not expose screen-content `iUsageType`: it uses defaults. An ignored `usage`
option would falsely claim that mode; none is installed. The current diagnostic
OpenH264 rows measure that adapter. Cisco runtime-download provenance and a
screen-content native adapter remain separate work before the requested
OpenH264 distribution comparison can pass. See the [FFmpeg adapter source](https://github.com/FFmpeg/FFmpeg/blob/n8.0/libavcodec/libopenh264enc.c),
[Cisco OpenH264 documentation](https://github.com/cisco/openh264/blob/master/README.md)
and [libvpx license](https://www.webmproject.org/license/software/).

## MP-08/MP-10/MP-11: acceptance boundary

The supplied `/root/work/cloud` checkout has no real-app display integration,
and neither checkout contains `scripts/e2e-stack`. The private paired Cloud ref
cannot be fetched with the current Git access. A presenter harness and kernel
libtest relay drill are component evidence only. Building product binaries does
not turn those fixtures into the real-app path. Real-app red/green acceptance
requires the coordinator to supply the paired app integration ref or harness.
The software comparison and focused guards are recorded separately below;
none closes an MP item on its own.
