# Reviewed unsigned embedded execution fixture

This separate hosted Linux x64 fixture executes the actual pinned libnode and
embedder with the current trusted bootstrap/SDK, after the production native
launcher has entered its OS sandbox. It never builds Node and never runs on the
developer laptop. The independent libc containment fixture remains unchanged.

The required input is successful completion of native run `34185809490` from
`charioxai/chariox`, workflow `.github/workflows/app-runtime-native.yml`, source
commit `28904c4f61e4df56d939a9e7b156373cc612c031`. The manual `native_run` parameter
must match this allowlist. Supporting another original build requires a reviewed
source edit. No arbitrary run, repository, workflow or artifact URL is accepted.
A pending compilation records `ready: false` and fails preparation; a failed
compilation is rejected. Neither case can count skipped execution as a pass.
The earlier one-job run `34167089795` timed out and produced no artifact.
Two-job run `34176513092` completed compilation but failed dependency validation
because the target loader was missing from its allowlist; its outputs were
removed and it produced no artifact. The reviewed replacement includes that
target-specific correction and uses the same two-job hosted hard resource limits.
Its explicit label admission passed; compilation alone does not establish an
artifact or an embedded execution result.

Preparation checks the original GitHub run and retained artifact metadata,
streams at most 512 MiB from GitHub's short-lived HTTPS storage redirect, and
verifies the archive's exact API-observed size and SHA-256 digest. The GitHub
credential is sent only to the GitHub API, never to the storage redirect or the
worker. The ZIP helper independently hashes the held regular file before
admitting its bounded headers and extracting exactly four regular files. It
rejects excess entries/bytes, duplicates, links, paths, encryption, ZIP64,
multidisk archives, unsupported compression, inconsistent headers and CRCs.

The unsigned bundle assembler checks original committed source provenance,
library digests, Node/toolchain/ABI pins and the complete current SDK graph. The
assembler retains `matches-current-source` and its exact input hash when all
current native inputs match. If later native source changes, explicitly enabled
historical evidence mode retains `historical-source-only` and
`receiptInputHash: null`. Both cases record the original run and new bundle
digest. Neither enrolls a runtime release or creates a signed artifact.

Only the small production C launcher and digest-pinned bubblewrap are compiled,
with one build job. A dedicated systemd service admits all root provisioning,
coordinator and worker descendants to **2 GiB RAM, no swap, one CPU, 128 tasks
and 180 seconds**. The complete CI job has a ten-minute limit. Mounts exist only
inside that disposable service's private mount namespace. Package/data/tmp
each have an 8 MiB `nodev,nosuid,noexec` tmpfs; the package becomes read-only.
The bounded on-disk runtime bundle gets a read-only `nodev,nosuid` bind mount,
preserving executable library mappings without copying libnode into tmpfs.

Bubblewrap uses the same fixed user/PID/network/IPC/UTS/cgroup isolation and
zero-capability contract as the native fixture. The exact bundled bwrap receives
only its temporary AppArmor user-namespace exception. No global userns/AppArmor
disable, ptrace privilege, extra sandbox bypass, `/proc`, host home or runtime
credential is provided. The root coordinator drops supplementary groups; bwrap
and the worker run as the hosted runner UID with a clean environment. Only
transitive system libraries found by bounded ELF inspection, the ELF
interpreter, harmless exec-denial fixture, and `/dev/null` are exposed.

The coordinator requires exact native FD4 readiness, independently observes
the worker's namespaces, mount flags, UID/GID, empty supplementary groups, zero
capabilities, NNP/seccomp, cgroup and executable inode, and proves no App marker
or FD3 SDK traffic appeared before sending Continue. Only then does the loader
`dlopen` the reviewed runtime and call its embedded Node entry point.

The App fixture registers one tool through the real trusted SDK. It checks Node
24.20.0/module ABI137, ESM/CJS imports, private read/write/copy/watch, crypto,
filtered environment/descriptors, and denial of host/escaped reads, package
writes, subprocesses, native add-ons, and raw TCP/UDP. The kernel fixture invokes
that tool through FD3, verifies the declared catalog, drains the actual shutdown
reply, then separately verifies channel-loss termination and a stalled
registration deadline. These are actual JS/runtime tests only when the hosted
execution step succeeds; local metadata and ZIP tests are not execution proof.

The evidence retains the original artifact/run, current bundle, launcher/bwrap
and system-library digests, actual sandbox observations, each completed case,
and bounded execution logs. Failed runs retain partial evidence and then remove
only their owned cgroup/profile/scratch. New compatibility permissions must be
motivated by an observed failure and retain the independent native hostile
checks; they are not preemptively granted.

This produces reviewed unsigned Linux execution evidence, with the bundle
recording whether its native inputs still match the current source. Signed runtime enrollment,
macOS execution/signing/JIT policy, installer activation, complete quota behavior
and the full release validation matrix remain separate requirements.
