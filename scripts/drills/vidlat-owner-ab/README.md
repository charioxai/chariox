# MP-08 / MP-10 / MP-11 — owner A/B runtime scripts

The kit runs the frozen Chariox OpenH264 kernel/browser and qualified Selkies
wheel baseline on one quiet Ubuntu26.04 CCX43. The web app uses the private
validation stack's disposable fixture account. Opening its waiting-room link
is the entire owner sign-in step; the coordinator enrolls the kernel through
normal product device approval beforehand. See OWNER_AB_TEST.md and OPERATOR.md.

Ship these public prebuilt assets alongside the scripts:

- `bin/chariox-kernel`, `bin/chariox-cli`, and their actual `bin/OSS_HEAD`.
- Matching built kernel-client in `sdk/`, with runtime dependencies.
- Standalone Python, qualified wheel environment and native libraries under
  `baseline/`, the Selkies lock and license notices under `licenses/`.
- `runtime-libs/`, exact-source `ARTIFACTS.json` with validation qualification,
  and `SHA256SUMS` covering every shipped file except itself.

OpenH264 is fetched directly from Cisco at startup. The archive includes no
provider profile, auth file, token or private key. Normal startup verifies the
pinned private fixture origin and machine API before launching runtimes.
`AB_REHEARSAL=1` changes only the Selkies bind/firewall behavior; it does not
bypass origin qualification. Scripts isolate state, record exact PID/start
identities and remove only marked owned resources. The gate helper edits only
the lane stack's Caddy fragment under the existing lock.

Video optimization remains paused. Startup and public-site rehearsal results
qualify the A/B kit; they do not establish strict latency/CPU/FPS parity,
Mac/home-network ratings, provider/Vault acceptance or multi-hour stability.
