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
`ubuntu-24.04` GitHub runner when native build inputs change in a PR. Later
unrelated commits skip compilation; manual dispatch is also available once the
workflow exists on the default branch. Only one build runs at a time for that PR,
with one matrix target and one Make job. No source/object cache, repository
secrets, signing key, release upload, or automatic installation is involved.

The explicit `--resource-profile github-linux` leaves the default shared-host
limits unchanged. [Node's upstream build guidance](https://github.com/nodejs/node/blob/v24.20.0/BUILDING.md#prerequisites)
describes an 8 GB machine for four compilation jobs; this profile attempts one
job inside a hard 6 GiB, no-swap, two-CPU, 256-process cgroup. The driver verifies
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
compilation has a 165-minute container timeout and a 180-minute job timeout.
Scratch is outside the checkout and removed after artifact handling.

On success, the workflow retains at most 512 MiB of explicitly
`UNSIGNED-NONRELEASE` libraries, license and provenance for seven days. These
artifacts are publicly accessible CI evidence in the public repository. They
are never enrolled as a trusted runtime release. A Debian 12 build does not
establish compatibility with an older glibc distribution; that release baseline
and native macOS/arm64 builders still need validation.

## Remaining release integration

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
