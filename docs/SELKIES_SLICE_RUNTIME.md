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

The slice owns Selkies as a display process. The kernel retains Room, actor,
input-ownership, and action authority. The relay forwards scoped display
traffic without decoding it. Selkies command execution, independent file
transfer, and remote device forwarding stay disabled unless a reviewed kernel
capability explicitly enables them. noVNC remains an explicit rollback choice
during validation, not a silent fallback after a Selkies failure.
