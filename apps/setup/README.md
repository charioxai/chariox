# Chariox Setup (MP-07 / MP-08 / MP-11)

One generic executable contains public release selection and verification inputs,
never a user enrollment code or credential. Linux x86_64 and macOS arm64 share
PR1's marked per-user installer, signed immutable release trees, isolated service
and state roots. Linux uses systemd --user; macOS uses launchd in the user's GUI
domain. Setup requires Python 3 for the existing bounded archive extractor.
The Linux executable carries its JavaScript runtime (no Node/Bun install); its
libc floor is the build target's. macOS .app packaging is an unsigned skeleton
opening the terminal flow; real signing/notarization and launchd are owner gates.

## MP-07 / MP-08 installation and lifecycle

Run `chariox-setup` to download/verify the selected CLI/kernel release, create a
user service, open **Approve this machine**, persist that kernel's key-bound
credential, start it and wait for authenticated local relay readiness.
`--install-only --login` installs the CLI/service definition first, then runs
`chariox login`. Login enrolls the terminal separately; with no local kernel it
offers **Set up a Chariox kernel on this machine?**. Accepting invokes Setup and
the ordinary kernel device flow. The detached TUI offers `/cloud setup` after
login. A later `chariox setup` accepts the same offer. No terminal credential or
provider account is passed to Setup.

`--repair` verifies cached release bytes and restores a missing marked service
file, then enrolls/starts if needed. Edited/foreign files are refused.
`--upgrade --release-version VERSION` verifies the new signed release under the
original public pin, captures current readiness, stops only this service,
atomically switches its release pointer and requires readiness with the same
identity. Failure restores the old pointer/marker/service. A pending upgrade
journal rolls back before the next operation; a process killed while holding
its exclusive install-directory lock needs operator verification/removal of
that exact stale lock. Power-loss/filesystem durability and real reboot recovery
are not established by source tests. `--uninstall` stops/disables only this
service and removes its marked release root and CLI links; private kernel state
and Cloud directory ownership are retained. Unlink/identity retirement remains
an explicit ordinary kernel action, not implicit filesystem deletion.

Default install ID `local` uses `~/.local/share/chariox/ssh-machines/local`,
`~/.chariox/dev/ssh-machines/local`, Linux `chariox-ssh-local.service` or macOS
`com.chariox.kernel.local.plist`, loopback ports 55139/55140, and `~/.local/bin`
CLI links. Other `--id ID --port PORT` installs get independent roots/services
and `chariox-ID` / `chariox-setup-ID` links. Existing installations and
`chariox-md-staging.service` / `~/.chariox/dev/md-staging` are never adopted.
The install ID, kernel/MCP ports and release are explicit public selections.

## MP-07 / MP-11 public bootstrap and owner signing

`deploy/setup/install.sh` is a template. Build a versioned script/executable:

```sh
node scripts/build-chariox-setup.mjs --version VERSION --public-key PUBLIC_HEX \
  --target linux-x64 --output /absolute/task-owned/output
```

The public key must come from the approved release inventory; there is no TOFU
or pin downloaded beside an artifact. Publish rendered `install.sh` and a Setup
executable named `chariox-setup-VERSION-PLATFORM` with a detached `.sig` (128
lowercase hex characters). Bundle artifacts use the existing
`chariox-VERSION-PLATFORM.tar.gz`, `.manifest.json` and `.manifest.sig` layout.
Public asset redirects are bounded and HTTPS only; ticket redemption never
follows redirects. Setup verifies the bundle and nested runtime inventory before
executing any downloaded binary. The existing Path-1 verifier/bootstrap and root
App-runtime installer remain unchanged. Per-user setup doesn't enroll privileged
App helpers or configure Docker/system services.

The unsigned CI artifact is **not** ready for download. On the owner's signing
machine, codesign/notarize/staple macOS artifacts first, then:

```sh
node scripts/sign-chariox-setup.mjs --artifact /path/to/final/executable \
  --signature /path/to/executable.sig --public-key PUBLIC_HEX \
  --signer /absolute/owner-signing-hook
```

The hook signs exact final bytes and writes their Ed25519 signature as lowercase
hex. It keeps all private-key handling outside builders/source. The helper checks
against the approved public pin before reporting success. CI produces generic
unsigned Linux/macOS artifacts and the macOS app skeleton; the release-bundle
workflow includes Setup in the signed CLI/kernel inventory. No CI is invoked by
this lane, and no signing/private key is provisioned here.

## MP-08 / MP-11 single-use browser codes

`--enroll` accepts **no argument value**. Supply the code only through stdin or
the hidden one-time terminal prompt:

```sh
curl -fsSL https://chariox.com/install.sh -o /tmp/chariox-install.sh
sh /tmp/chariox-install.sh --enroll
rm /tmp/chariox-install.sh
```

With `curl | sh --enroll`, Setup prompts via `/dev/tty` when script stdin is
exhausted. No code is included in argv, environment, a file, notices or logs.
The kernel uses the existing ticket variant of `/auth/device/poll` to obtain only
its own credential. Buffers are cleared and ticket references dropped after
redemption. A browser code without an expected-owner hint is refused for an
already-enrolled root rather than silently claiming another user's enrollment;
use ordinary device-flow repair for that root. SSH push retains its strict
expected-owner check and stdin contract. Copy-kernel/context UX is PR4, using the
existing encrypted context transfer; provider logins remain per machine.

## MP-07 / MP-08 / MP-10 / MP-11 evidence limits

Focused fixtures prove signature/digest refusal, nested inventory verification,
exact service/path isolation, repeat behavior, repair, upgrade rollback,
interrupted-journal recovery, stdin-only codes and local kernel enrollment seams.
Real Cloud/device approval, signed published artifacts, macOS signing/launchd,
real systemd, Mac-to-LAN and fresh-machine parity remain coordinator acceptance.
No MP item is closed by the local mocked drills.
