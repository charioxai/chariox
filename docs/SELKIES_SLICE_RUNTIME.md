# Selkies slice runtime

## Pinned upstream inputs

`apps/kernel/slice-linux-docker/selkies.lock.json` is the source of truth for
Selkies and its native capture wheels. `fetch-selkies.mjs` verifies SHA-256
before exposing a download to the image build. It also verifies cached files
and refuses unknown architectures. Downloads stay outside the repository.

The selected Selkies revision is
[`3f87241fcd6abc44e205b22f6596e78ef4946670`](https://github.com/selkies-project/selkies/tree/3f87241fcd6abc44e205b22f6596e78ef4946670).
Its [upstream CI run](https://github.com/selkies-project/selkies/actions/runs/33318518640)
passed with pixelflux `6974a8c16a14dc0040d5ed937d633a6ceb4c926d` and pcmflux
`a8fac8d8ea89117a1c2480bdbefbb3b8b717d956`. The lock uses that same combination.
The newer Selkies `ca30aa9` revision failed CI and was not selected.

This is the unreleased WebSocket-first line. The last tagged Selkies release,
`v1.6.2`, uses the older GStreamer implementation and is not the implementation
described by the current WebSocket documentation. Do not replace these pins
with `main`, `latest`, or an unversioned PyPI install. The pinned Selkies code
requires capture package version 2.1.0, which is supplied by the commit-tagged
GitHub releases rather than PyPI's 2.0.0 packages.

The portable baseline uses Python 3.11 on Debian Bookworm, X11, WebSocket
transport, and software H.264. Both `amd64` and `arm64` capture wheels are
pinned. Hardware encoding and WebRTC are not required for this baseline.

The Dockerfile has a `selkies-build` target and a small `selkies-runtime`
target below the full Chariox slice. The build uses a digest-pinned Node image,
the checked-in npm lockfiles, and serial web builds. The runtime uses the
checked-in Python version and wheel-hash requirements for both architectures.
Pip rejects unlisted hashes and source-only dependency fallbacks. The locally
built Selkies wheel and SHA-verified capture wheels install offline without
dependency resolution, followed by `pip check`. Build inputs are bind-mounted
into the install layer, so the final
image does not retain wheel archives, npm dependencies, or source-build tools.
Runtime notices and the input pins live in `/usr/share/doc/chariox-selkies`.

Each download has a unique temporary file. Normal failures remove that file;
an uncatchable process or machine crash can leave it behind. Image builds put
downloads in a disposable build layer. Standalone fetches should use a
task-owned scratch directory and clean it after the run. The downloader does
not sweep another invocation's `.part` files, which may still be in use.

## Local packaging drill

Build the `selkies-runtime` target, then run `validate-selkies.py` inside it
using `/opt/chariox-selkies/bin/python`. Mount the script read-only and use an
unprivileged numeric user, `--network none`, `--init`, `--cpus 1`,
`--memory 768m --memory-swap 768m`, `--pids-limit 128`, and `--shm-size 128m`.
No ports need publishing. The test starts its own Xvfb display and streamer,
checks the packaged web assets and relative PWA manifest, receives a real
640x480 software H.264 keyframe over the public WebSocket endpoint, and checks
that TERM releases the HTTP listener and X11 socket. It removes its scratch
directory and owned processes even on failure.

For a negative control, set `CHARIOX_SLICE_SELKIES_BIN=/bin/false`; the drill
must exit unsuccessfully rather than treating an open display as a working
streamer. This is packaging coverage, not proof of browser decoding, relay
transport, multi-actor input, or production readiness. Those require the
later kernel and client drills.

## License inventory and distribution

Selkies, pixelflux, and pcmflux source files use MPL-2.0. That is not a complete
description of the bundled runtime:

- Selkies vendors Python-Xlib under LGPL-3.0.
- The official pixelflux wheels include x264 under GPL-2.0-or-later and FFmpeg
  shared libraries under LGPL-2.1-or-later.
- The pcmflux wheels include PulseAudio and other replaceable LGPL libraries,
  plus permissively licensed audio libraries.

The exact native dependency inventories are in
[pixelflux LICENSES.md](https://github.com/selkies-project/pixelflux/blob/6974a8c16a14dc0040d5ed937d633a6ceb4c926d/LICENSES.md)
and
[pcmflux LICENSES.md](https://github.com/selkies-project/pcmflux/blob/a8fac8d8ea89117a1c2480bdbefbb3b8b717d956/LICENSES.md).
The image must retain upstream notices and source references. Public rollout
also requires a complete corresponding-source and notice bundle for the
distributed native libraries, including x264. A green functional test is not
evidence that this distribution requirement is complete. No proprietary GPU
driver is bundled or required for software mode.

## Integration boundary

`docker/slice-selkies.py start|status|stop` owns one streamer process. It
serializes lifecycle commands, records PID plus process creation time, refuses
to signal a reused PID, and rotates its private control token on restart.
Startup checks HTTP health and verifies the private control credential before
reporting ready. Stop uses TERM, waits, and reports a forced stop as failure.
Fresh-start cleanup uses `stop --allow-forced`, which permits recovery after
confirmed forced termination. State-capture shutdown uses strict `stop` and
continues to report forced termination as a failure.
An empty viewer table denies anonymous input and video until the kernel
provisions a scoped viewer. Tokens never appear in status output.

State lives under `$XDG_RUNTIME_DIR/selkies` or the slice user's private
`/tmp/chariox-slice-<uid>/selkies` directory. The process record has mode 0600
and is removed on stop. The streamer binds container loopback only. This
private endpoint is not a directly published browser URL.

`CHARIOX_SLICE_VIEWER_BACKEND=selkies` selects it in `slice-screen.sh`.
`novnc` selects the rollback launcher. During this staged implementation the
existing noVNC default is unchanged; switching the product default requires
the shared transport, client validation, and rollout sign-off in the plan.
A selected Selkies failure never falls back silently. Browser tools do not
require a healthy viewer process.

The `slice-runtime-deps` image target contains the real browser and desktop
dependencies without building provider CLIs or the Rust kernel. Run
`validate-slice-viewer.py` inside it to exercise the actual desktop launcher,
streamer crash with continuing Browser tools/screenshots, failed startup,
explicit noVNC rollback, and final listener cleanup. Use one CPU and 1 GiB
memory for this drill. The full kernel image and shared transport still need
their separate end-to-end validation.

The slice owns Selkies as a display process. The kernel retains Room, actor,
input-ownership, and action authority. The relay forwards scoped display
traffic without decoding it. Selkies command execution, independent file
transfer, and remote device forwarding stay disabled unless a reviewed kernel
capability explicitly enables them. noVNC remains an explicit rollback choice
during validation, not a silent fallback after a Selkies failure.
