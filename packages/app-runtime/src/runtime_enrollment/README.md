# Installed runtime trust

`EnrolledRuntime::open_installed` reads the system installer's fixed external
trust file: `/etc/chariox/apps/runtime-enrollment.json` on Linux, or
`/Library/Application Support/Chariox/AppRuntime/runtime-enrollment.json` on macOS.
Every ancestor must be a real root-owned directory without group/other writers.
The regular enrollment file must be root-owned, singly linked, and have no
group/other writers. App data, CLI request fields and environment variables cannot
substitute a trust path or public key.

The enrollment uses schema `chariox.app-runtime-enrollment.v1`, a positive
`revision`, the current `target`, an absolute `runtimeRoot`, the exact
`inventorySha256` (64 lowercase hex), and `publicKeyHex` (32-byte Ed25519 key as
64 lowercase hex). The root installer owns revision advancement and rollback
policy; the verifier does not let a bundle nominate its own trusted version.

The final runtime directory contains `runtime-inventory.json`, its detached
`runtime-inventory.sig` (64 bytes encoded as exactly 128 lowercase hex characters,
without a newline), and an empty `.runtime-lease`, all mode0444. The signature
covers the exact inventory bytes. The inventory schema is
`chariox.app-runtime-inventory.v1` and contains `target`, `runtimeVersion`,
`workerAbi`, `nodeVersion`, `nodeModuleAbi`, `sdkVersion`, `sourceCommit`, and
sorted `files` entries with `path`, `size`, `sha256`, and `executable`.

The allowed source graph comes from the kernel's compiled bundle contract.
It includes the Node/runtime libraries, native launcher, bootstrap and complete
SDK, licenses and build manifests. Linux additionally requires the native cgroup
entry and bundled bubblewrap. Executables have mode0555; other files mode0444.
Unknown/missing files, extra directories, symlinks, hard links, incorrect modes,
ABI/version mismatches and changed digests fail verification. Hashing uses a
64KiB buffer and the existing 512MiB bundle ceiling. Nothing is executed here.

The installer takes an exclusive nonblocking lock on `.runtime-lease` before
removing an old generation. Readers hold shared locks plus exact open file and
directory descriptors. The platform resource domain must retain the entire
`EnrolledRuntime` until the process domain is empty **and** admitted broker work
has drained. The installer publishes new generation directories; it never edits
a live generation in place. The lock is cooperative with the trusted installer,
not a security boundary against root.

This is the filesystem/cryptographic verifier. A release assembler and installer
must still produce and enroll the signed final files. macOS must additionally
verify the actual Developer ID signatures and notarization after signing, before
enrollment. The unsigned native CI artifact is not accepted merely because its
manifest contains successful validation fields. The real platform factory must
still establish and observe containment before running App code.

Three focused tests execute real Ed25519 verification, digest checks, filesystem
denials and shared/exclusive lease behavior. Their private trust root uses the
test user's UID under `~/.chariox/dev/` and is removed on completion; the public
production constructor always requires UID0 and the fixed system path. These
tests contain text fixtures, and do not attest to Node execution or code signing.
