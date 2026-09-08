# Hosted Chromium persistence fixture

The dedicated `chromium-profile-drill.yml` workflow runs a real Chromium process
on a Linux hosted runner. Nothing launches a browser or Docker during its small
local contract tests.

The fixture image uses the production slice's immutable Node base, CA bundle,
Debian snapshot, UID/GID 1001, and exact `slice-screen.sh`, `browser-cdp.mjs` and
`chromium-sandbox-probe.mjs` bytes. Preparation fails if those inputs drift.
It excludes the kernel and provider toolchains. The production profile remains
`/home/slice/.config/chariox-slice-chromium` with `--password-store=basic`.

The drill launches through the production desktop command with the committed
seccomp policy, verifies the actual sandbox page and renderer process metadata,
and uses a loopback HTTP fixture to write a persistent HttpOnly authentication
cookie, a visible cookie, localStorage and IndexedDB. It stops the browser cleanly,
archives the whole home as tar.zst, removes the original container, invokes the
production `restore-migration-home` action into a fresh volume, then starts a
fresh browser and verifies the exact data and restored fixture tab. Independent
negative checks revoke the fixture session on the server and use an empty
profile; neither may pass the successful authentication/storage check.
The probe requires observed renderer PID nesting, UID/capability lockdown and
additional seccomp filters. It checks namespace inodes when readable; Chromium's
non-dumpable renderers may deny those ptrace-gated links. In that case network
isolation is attested by `chrome://sandbox`, with no claim of independent network
inode inspection and no added ptrace capability. The result records bounded
renderer and restricted-link counts, so evidence distinguishes these paths.

Browser containers have no external network or published ports, two CPUs,
2 GiB memory without extra swap, and 512 PIDs. The BuildKit container also has
two CPUs and 2 GiB memory, with one build step at a time. The production restore
action receives a Docker shim that adds stricter hosted helper resource limits
and the drill's cleanup label; its restore commands remain unchanged.
[Docker documents the build container resource options](https://docs.docker.com/build/builders/drivers/docker-container/).
The job and individual stages have deadlines. Cleanup inspects ownership labels
and removes only this drill's containers, volumes and image, using immutable IDs
where Docker supplies them. No daemon-wide prune or service restart is used.

Artifacts contain input hashes and revision, browser/package versions, bounded
failure logs, and structured checks. The tar archive, volumes, images and scratch
are temporary. This is production-launcher and home-restoration fixture evidence;
it does not exercise the full kernel migration transaction, macOS-hosted Docker,
App origin separation, or real Google authentication. Those release gates remain
open until their separate drills pass. No live account is used here.

Small checks:

```sh
node --test --test-concurrency=1 scripts/chromium-profile-drill/contract.test.mjs
```

## Recorded hosted result

[Run 34171378772](https://github.com/charioxai/chariox/actions/runs/34171378772)
passed on 2026-09-07 at `12ffd1f5156625ecd82d2c88d1ef31451273abb2`, using
Chromium `147.0.7727.137-1~deb12u1` and Node `22.23.2`. Initial, restored and
empty browsers passed with 3, 4 and 3 inspected renderers. All restricted PID
and network namespace-link counts were zero, so this result includes direct
namespace-inode checks; it did not use the diagnostic-only network path.
Authentication, visible cookies, localStorage, IndexedDB and the saved tab
survived the stop/archive/restore/relaunch. Server-revocation and empty-profile
negatives passed, and owned-resource cleanup succeeded.

The retained result explicitly says `fullKernelMigrationValidated=false` and
`googleAuthenticationValidated=false`. It is evidence for this deterministic
production-launcher fixture, not completion of the remaining release gates.
