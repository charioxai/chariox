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
