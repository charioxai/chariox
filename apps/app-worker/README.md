# App runtime build foundation

This directory defines the **unsigned native runtime build**, not a working App
launcher. No App backend can be started through this code yet. The native build
recipe has not been compiled or exercised on the target builders.

`runtime.lock.json` pins Node 24.20.0, its module ABI 137, the source archive's
SHA-256, compiler versions, build limits, and four native targets: macOS arm64/x64
and Linux arm64/x64. The checksum was read from the [official Node checksum
list](https://nodejs.org/download/release/latest-v24.x/SHASUMS256.txt) on
2026-09-07; the download URL and digest are fixed to that release. There is no
dependency on a user's system Node, Homebrew libraries, npm install scripts, or
an upstream prebuilt `libnode` package.

The build uses upstream `--shared` and produces two adjacent libraries:

| Target | Internal embedder | Bundled Node |
| --- | --- | --- |
| macOS | `libchariox-app-runtime.dylib` | `libnode.137.dylib` |
| Linux | `libchariox-app-runtime.so` | `libnode.so.137` |

The launcher must apply the OS sandbox and resource limits **before `dlopen` of
the embedder**; dynamic loading brings in Node and its static initializers at
that point. The launcher must not link either library into its own startup
image. This follows the ordering described in [Chromium's Seatbelt
design](https://chromium.googlesource.com/chromium/src/sandbox/+/HEAD/mac/seatbelt_sandbox_design.md).
The small C ABI uses [Node's embedder
API](https://nodejs.org/download/release/latest-v24.x/docs/api/embedding.html).
Only the signed launcher supplies its bounded arguments and trusted bootstrap;
Apps cannot supply either. The process exits after the one-shot entry returns.

## Plan and build

Run the build tooling with Node 24 or later. Planning requires neither a matching
host nor installed build tools, performs no download, and writes no scratch:

```sh
node scripts/build-app-runtime.mjs plan \
  --target linux-arm64 --scratch /build-scratch/chariox-runtime
node --test scripts/build-app-runtime.test.mjs
```

An actual build requires a dedicated native builder matching the locked compiler,
Python, Make, and (on macOS) Xcode versions. Provision those separately. The
default limits require 16 GiB total RAM, 8 GiB currently free RAM, and 32 GiB free
disk; compilation defaults to one job and never permits more than two. The
driver monitors remaining RAM/disk and terminates only its build process group
if reserves are exhausted. It must not run alongside other resource-heavy jobs.

Commit the build inputs first so provenance identifies reviewed source. Give the
builder a fresh empty directory outside every Git checkout, without symlink
ancestors and under a trusted parent that other users cannot replace. Existing
scratch must belong to the builder's user and exclude group/other permissions
(normally mode `0700`). On that prepared builder, explicitly choose a source:

```sh
node scripts/build-app-runtime.mjs build \
  --target linux-x64 --scratch /build-scratch/chariox-runtime \
  --source-archive /source-cache/node-v24.20.0.tar.xz
```

`--download-source` is the alternative to `--source-archive`; it is never implicit.
`--cc`, `--cxx`, and `--python` accept absolute tool paths and still require the
locked versions. The copied/downloaded archive is bounded and hash-verified
before extraction. Upstream source and temporary objects are removed after the
build; the caller's cached archive is preserved. Only `artifacts/` survives a
successful build, containing the two libraries, Node's license, and
`artifact-manifest.json`. Dependency inspection rejects external non-system
libraries and escaping Linux runpaths without executing either library.

## Dedicated hosted Linux build

`app-runtime-native.yml` builds the initial Linux x64 target on a disposable
`ubuntu-24.04` GitHub runner after explicit manual admission. Before these
workflows are merged, adding the exact PR label `app-runtime-build-linux`
admits that labeled event's head commit. After merge, `workflow_dispatch` with
`confirm_build=true` is also available. Admission verifies a same-repository
head and the initiating actor's current write/maintain/admin permission through
GitHub. An ordinary PR push, synchronize event or label already present cannot
admit compilation; those updates run lightweight tests. Later manually admitted runs can reuse recent
successful compilation evidence. One build runs at a time for that ref,
with one matrix target and two Make jobs. No source/object cache, repository
secrets, signing key, release upload, or automatic installation is involved.

The only cache entry is a JSON receipt smaller than 4 KiB, written after a
successful native build and artifact upload. Its fingerprint covers the exact
committed native source, lock, driver, workflow, wrapper, helpers and tests.
GitHub may replace a queued workflow with a newer pending commit; every run
checks its full current fingerprint, so that cannot skip unbuilt native edits.
Receipts and their original artifacts must be less than five days old. Before
reusing a PR-scoped receipt, the workflow checks GitHub's original successful
run, retained artifact and committed input tree. Missing, forged, expired or
unverifiable evidence causes a fresh build only inside an explicitly admitted
workflow. The receipt records the original run
and artifact link; it cannot enroll a runtime or stand in for signed release
verification. No binaries, source trees or compiler objects enter this cache.

The explicit `--resource-profile github-linux` leaves the default shared-host
limits unchanged. [Node's upstream build guidance](https://github.com/nodejs/node/blob/v24.20.0/BUILDING.md#prerequisites)
describes an 8 GB machine for four compilation jobs; the dedicated Linux retry
attempts two jobs inside the same hard 6 GiB, no-swap, two-CPU, 256-process cgroup. The driver verifies
those actual limits before downloading source and throughout compilation. It
requires at least 7 GiB host memory, 3 GiB initially available memory and 24 GiB
free disk, then preserves 768 MiB memory and 4 GiB disk reserves. Memory
availability accounts for reclaimable inactive file cache, as
[Docker's Linux statistics do](https://docs.docker.com/reference/cli/docker/container/stats/).
This is a bounded compilation attempt, not an assumption that linking will fit.

The workflow removes only fixed, unused preinstalled SDK directories from that
disposable VM. It then uses the digest-pinned official
`node:24.20.0-bookworm` amd64 builder in `runtime.lock.json`, with the locked
Debian GCC 12.2.0, Python 3.11.2 and Make 4.3 checks still enforced. No image or
source is downloaded by planning or by the lightweight tooling tests. Actual
compilation has a 300-minute container timeout and a 330-minute job timeout.
Scratch is outside the checkout and removed after artifact handling.

The original one-job [run 34167089795](https://github.com/charioxai/chariox/actions/runs/34167089795)
reached its 165-minute command deadline on September 8, 2026 while compiling
V8 `turbofan-typer.o`, after 3,635 reported C/C++ compilation operations. It
returned timeout exit 124 with no compiler or resource-threshold error in the
retained log; no artifact was produced. The two-job retry is a dedicated hosted
resource experiment, not measured proof that two compilers or linking fit. Its
hard limits and remaining-resource guards are unchanged. The longer deadline
and profile change alter the exact native-input fingerprint, so an earlier
receipt cannot authorize reuse. macOS and ordinary shared-host defaults are
unchanged.

The two-job [run 34176513092](https://github.com/charioxai/chariox/actions/runs/34176513092)
finished compiling Node and the embedder on September 8 at 03:47 UTC within
those hard limits. Artifact inspection then rejected the exact x64 ELF loader
`ld-linux-x86-64.so.2`, already part of the signed platform graph. The validator
now permits only the loader matching its target architecture. That run remains
failed: its outputs were deleted and no usable artifact or execution evidence
was retained. A fresh explicitly admitted build is required.

On success, the workflow retains at most 512 MiB of explicitly
`UNSIGNED-NONRELEASE` libraries, license and provenance for seven days. These
artifacts are publicly accessible CI evidence in the public repository. They
are never enrolled as a trusted runtime release. A Debian 12 build does not
establish compatibility with an older glibc distribution; that release baseline
and native macOS/arm64 builders still need validation.

## Dedicated hosted macOS build

`app-runtime-macos-native.yml` accepts only an explicit `app-runtime-build-macos`
labeled event with the same permission/head checks, or a post-merge dispatch
with `confirm_build=true`. It checks out and records that exact admitted SHA;
an unrelated later push does not change the admitted source. The separate `macos-build-profile.json` selects macOS 15
arm64, Xcode 16.4 build 16F6, SDK 15.5, Apple clang 17.0.0
(`clang-1700.0.13.5`), Python 3.11.9 and one Make job. The command budget is
300 minutes across the whole build; the job has 330 minutes for artifact
handling and cleanup. No Xcode deletion, signing or notarization occurs.

The [observed preflight](https://github.com/charioxai/chariox/actions/runs/34172680900)
reported 7 GiB total RAM, 3.12 GiB available RAM and 42.23 GiB free disk on
image `20260829.0321.1`. Its free-RAM value was only 230 MiB: admission uses
Node's available-memory measurement on this profile. It requires 7 GiB total,
3 GiB available and 24 GiB free disk. During compilation, the existing watcher
samples every second and stops on less than 1 GiB available RAM, less than
4 GiB disk, over 4 GiB summed process-group RSS, over 128 group processes,
over 256 MiB additional swap, telemetry failure or the deadline. Telemetry
has a three-second bound. These are monitored reserves on a disposable VM;
they are not hard RAM, CPU or swap limits. Summed RSS may count shared pages
more than once. First compilation must establish whether Node and linking fit.
The laptop's 16/8/32 GiB admission defaults are unchanged.

A fixed Python owner retains the build process-group leader until the JS owner
acknowledges command completion, after a fresh sample proves no descendant
remains. A private bounded status channel closes on parent failure, terminating
that group. Controlled cancellation and failed commands terminate the group
and reap the owner before scratch cleanup. If an external actor kills the
owner itself first, signaling authority is withdrawn immediately; cleanup of
any surviving descendants then relies on disposal of the hosted VM. No local
descendant-cleanup proof is claimed for that exceptional case. No GitHub token or runner home
is passed to compiler children. Resource extrema and runner-image identity are
retained separately from the four-member unsigned native artifact. Mach-O
architecture, install names and external dependencies are inspected without
loading the resulting libraries. Host available memory, disk and swap are
checked again after artifact copy/hash/inspection before success is published.

Native receipts and artifact provenance select target-specific input graphs:
Mac workflow/profile/watcher changes do not invalidate Linux receipts. This
introduction changes the common driver and receipt code, so it changes the
Linux fingerprint once. The already admitted Linux retry remains evidence for
its exact historical source; it is not relabeled as a new-source build. Neither
cached receipts nor successful compilation supplies release trust.

Shipping on macOS still requires Developer ID Application signing of the final
launcher and libraries, hardened runtime with the required JIT entitlement on
the executable, enabled library validation, a secure timestamp, notarization,
stapling and actual signed execution/containment tests. Apple Development or
ad-hoc signing can support development tests but cannot satisfy that release
gate. Signing credentials stay outside the repository and unsigned build jobs.

## Remaining release integration

The [separate bundle assembler](bundle.md) adds the pinned trusted bootstrap and
complete SDK graph to an existing native artifact without rebuilding Node.
Its deterministic inventory and unsigned integrity verifier are available now;
they do not provide runtime enrollment or embedded Node execution evidence.

The artifact manifest records source and tool digests, Git provenance, commands,
library dependencies, and output digests. It always records **unsigned**,
**not notarized**, **containment not tested**, and **reproducibility not compared**.
Path normalization, fixed source, version checks, and `SOURCE_DATE_EPOCH` make
repeated builds comparable; they do not establish byte-identical output. Release
CI still needs builders/sysroots for the other targets, a declared Linux glibc
baseline, successful native compilation on all four targets, and an independent
rebuild comparison.

The signed native sandbox loader, descriptor/environment filtering, Node
permission arguments, authenticated supervisor handshake, and trusted SDK
bootstrap are separate required work. App code is imported only after those are
established. Removing inspector and `NODE_OPTIONS` support during compilation is
defense in depth; this embedder alone is not a sandbox.

Release packaging must sign the final artifacts using Chariox's runtime signing
contract. macOS additionally needs hardened-runtime signing and notarization of
the complete launcher/library bundle, with the V8 JIT entitlement on the process
that runs it. Code signing changes bytes: recompute the final file digests and
sign the release manifest **after** platform signing. Do not pass the unsigned
build manifest off as a trusted runnable release. Native containment tests and
SDK startup tests must pass before the installer publishes these artifacts.
