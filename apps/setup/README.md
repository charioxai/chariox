# Chariox Setup (MP-07 / MP-08 / MP-11)

An owner's agent (or the owner, with `/machine add`) gets a one-line command from
the kernel's `IssueOwnMachineEnrollment` request; running it in the new machine's
own shell downloads this Setup and redeems its single-use code with `--enroll`.
Chariox does not log in to the machine or install over SSH.

One generic executable contains public release selection and verification inputs,
never a user enrollment code or credential. Linux x86_64 and macOS arm64 share
PR1's marked per-user installer, signed immutable release trees, isolated service
and state roots. Linux uses systemd --user; macOS uses launchd in the user's GUI
domain. Setup requires Python 3 for the existing bounded archive extractor.
For MP-07 / MP-08, Linux Setup checks logind lingering before enrollment/start,
repair or upgrade so the user service survives logout. It enables lingering
for the current user when host policy permits. Otherwise it stops with the exact
administrator command before consuming an enrollment code; it never invokes sudo.
Agents run inside Bubblewrap, so Linux Setup also checks that `bwrap` can make a
mount namespace: run as root it installs the `bubblewrap` package with apt-get or
dnf, otherwise it stops with the administrator command, before consuming an
enrollment code.
A fresh Linux installation provisions Claude Code, Codex CLI (including its
code-mode host, rg and bwrap), and OpenCode before starting the kernel. Setup
uses an explicit provider executable selection first, then the first match in
the invoking user's PATH, then standard user and system locations. It prints the
selected path and version and records the path in the kernel's user service.
An existing executable stays where it is. If its version is older than Chariox's
tested version, Setup shows how to update that installation through the
provider. Missing CLIs are installed under `~/.local/share/chariox/providers`
from the official platform packages pinned in the toolchain lockfile. Setup
does not sign in to providers or change their configuration or account files.
The Codex package's bwrap is for Codex. The kernel still needs the host's
Bubblewrap and namespace support for its own agent confinement.

The Linux executable carries its JavaScript runtime (no Node/Bun install); its
libc floor is the build target's. CI also packages an unsigned amd64 `.deb`
for Linux (libc 2.35+); it contains only the generic executable and metadata,
with no install scripts or service activation. Kernel setup still runs as the
intended user. The executable/install script is the default per-user path without
sudo. macOS .app packaging is an unsigned skeleton
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

Default install ID `local` uses `~/.local/share/chariox/machines/local`,
`~/.chariox/dev/machines/local`, Linux `chariox-machine-local.service` or macOS
`com.chariox.kernel.local.plist`, loopback ports 55139/55140, and `~/.local/bin`
CLI links. Other `--id ID --port PORT` installs get independent roots/services
and `chariox-ID` / `chariox-setup-ID` links. Existing installations and
`chariox-md-staging.service` / `~/.chariox/dev/md-staging` are never adopted.
The install ID, kernel/MCP ports and release are explicit public selections.
Login offers inspect ordinary fresh heartbeat records in these isolated roots
as well as the default registry; stopped/stale kernels remain eligible for repair.

## MP-07 / MP-11 public bootstrap and owner signing

`deploy/setup/install.sh` is a template. Build a versioned script/executable:

```sh
node scripts/build-chariox-setup.mjs --version VERSION --public-key PUBLIC_HEX \
  --target linux-x64 --output /absolute/task-owned/output
```

The connected service Setup enrolls with comes from the build: `--control-plane-url URL`, or by
default `DEFAULT_CONTROL_PLANE_URL` in `packages/kernel-client/src/default-control-plane.ts`, the one
place the repository names a default host. `--api-url` at run time overrides it.

MP-07 / MP-11: the shell bootstrap requires curl, Python 3 and an OpenSSL build
with Ed25519 `pkeyutl -rawin` verification on `PATH`. It verifies a public RFC 8032
test vector before downloading Setup; compatible OpenSSL versions pass, while
macOS's system LibreSSL is refused with a prerequisite diagnostic.
On macOS, install and select a compatible verifier before running the script:

```sh
brew install openssl@3
export PATH="$(brew --prefix openssl@3)/bin:$PATH"
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

### Publish Linux Setup

Prepare the Linux x86_64 `bin/` tree from the release commit with the standalone
`chariox`, `chariox-kernel`, `chariox-relay`, `chariox-app-package`, and
`chariox-app-wasm-worker` executables. Use the Linux build instructions in
[`release-bundle.yml`](../../.github/workflows/release-bundle.yml), including the
Ubuntu 22.04 build for the supported glibc baseline.

From the release checkout, prepare the unsigned bundle with a version, approved
release public pin, and the HTTPS release and control-plane origins. This command
uses Bun to compile Setup and emits the small signing documents separately.

```sh
node scripts/prepare-linux-setup.mjs prepare --version "$VERSION" \
  --public-key "$RELEASE_PUBLIC_HEX" --executables "$UNSIGNED_LINUX_DIRECTORY" \
  --release-base "$RELEASE_BASE" --control-plane-url "$CONTROL_PLANE_URL" \
  --output "$OUTPUT"
```

Transfer only the `signing/` documents and `scripts/sign-linux-setup-documents.mjs`
to the signing machine. Verify the independently recorded request checksum.

```sh
node sign-linux-setup-documents.mjs signing "$REQUEST_SHA256" "$RELEASE_PEM" signed
```

Return the three signatures to the release builder. The Linux bootstrap verifies
its detached signature over the domain-separated SHA-256 of Setup. The bundle
manifest signature covers the original manifest bytes. Assemble and verify the
assets before handing them to the authorized publication process.

```sh
node scripts/prepare-linux-setup.mjs assemble --version "$VERSION" \
  --public-key "$RELEASE_PUBLIC_HEX" --output "$OUTPUT" --signed signed
```

Use a separate staging prerelease version and staging origin for rental
validation. Use a staging artifact host for the drill. Public versions and
production origins are signed from their final release checkout, and public
publication is a separate operator step. The Linux installer starts the
device-code approval flow when run with no arguments.

The Add this machine command passes `--enroll` and supplies a browser-issued code
on stdin. Both forms install and start the kernel. Use `--install-only --login`
explicitly for the separate terminal login flow.

### Publish macOS Setup

macOS publication is blocked on Miguel's Developer ID Application certificate
and Apple notary credentials. The Linux command above can publish while those
are pending. Once available, codesign/notarize/staple the macOS executable before
computing its detached release signature:

```sh
node scripts/sign-chariox-setup.mjs --artifact /path/to/final/executable \
  --signature /path/to/executable.sig --public-key PUBLIC_HEX \
  --signer /absolute/owner-signing-hook
```

The owner hook signs the final bytes and writes their Ed25519 signature as
lowercase hex. The helper verifies it against the approved public pin.

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
use ordinary device-flow repair for that root. Copy-kernel/context UX is PR4, using the
existing encrypted context transfer; provider logins remain per machine.

## MP-07 / MP-08 / MP-10 / MP-11 evidence limits

Focused fixtures prove signature/digest refusal, nested inventory verification,
exact service/path isolation, repeat behavior, repair, upgrade rollback,
interrupted-journal recovery, stdin-only codes and local kernel enrollment seams.
Real Cloud/device approval, signed published artifacts, macOS signing/launchd,
real systemd, Mac-to-LAN and fresh-machine parity remain coordinator acceptance.
No MP item is closed by the local mocked drills.
