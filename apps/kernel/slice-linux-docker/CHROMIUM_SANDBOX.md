# Chromium sandbox and profile continuity

The Linux slice image already installs `chromium-sandbox` and runs as `slice`
(UID/GID 1001). Browser startup and URL recovery now use one launcher, reject UID
zero, and keep Chromium's sandbox enabled. The headless diagnostic uses the same
sandbox requirement. Ordinary containers receive the bundled seccomp policy;
they do not acquire `SYS_ADMIN`, privileged mode, or unconfined fallback.

`chromium-seccomp.json` is the Docker default profile from
[moby/profiles revision 61eaf32614c7c71b60bd8927d3e6a4ffc8ff1f31](https://github.com/moby/profiles/blob/61eaf32614c7c71b60bd8927d3e6a4ffc8ff1f31/seccomp/default.json),
plus one allow rule for `clone`, `setns`, and `unshare`. Its Apache 2.0 license is
beside it. This allows Chromium to create its own unprivileged namespaces while
retaining the other Docker syscall restrictions, following the namespace
approach in [Playwright's Docker guidance](https://playwright.dev/docs/docker).
The exception applies to processes in the container; Chromium must separately
establish its renderer namespace and Seccomp-BPF boundaries, described in the
[Chromium Linux sandbox design](https://chromium.googlesource.com/chromium/src/+/b4730a0c2773d8f6728946013eb812c6d3975bec/docs/linux_sandboxing.md).

The existing explicitly selected managed-provider policy
(`CHARIOX_SLICE_ALLOW_UNCONFINED_SECCOMP=1`) retains its separate rootless-daemon
and inner-provider-sandbox contract. Browser flags remain sandbox-enabled on that
path too. This change does not establish or revise provider isolation.

## Existing profiles and browser recovery

The default profile remains `$HOME/.config/chariox-slice-chromium`. Its explicit
environment override, `--password-store=basic`, loopback CDP endpoint, browser
sign-in preferences, and stable machine identity remain unchanged. Selecting a
different profile path or password backend would require a separate migration.
Chromium documents the [user data directory](https://chromium.googlesource.com/chromium/src/+/HEAD/docs/user_data_dir.md)
and [Linux password backends](https://chromium.googlesource.com/chromium/src/+/main/docs/linux/password_storage.md);
the existing `basic` choice is not OS-backed encrypted secret storage.

Offline startup recognizes nonempty modern `Default/Sessions/Session_*` files and
legacy `Default/Last Session` or `Default/Current Session` files. It requests
session restoration without adding a blank tab. An explicit URL is passed after
`--`, including during recovery, and can accompany a restored session. Warm
fallback never clears live profile locks or rewrites live preferences. A stopped
browser can recover on the running display without restarting Xvfb or the viewer.
CDP navigation and close calls have a 10-second timeout and 2-second kill grace.

The kernel's existing save path quiesces the desktop and archives the whole
`/home/slice` volume separately from the container image. Cookie databases,
`Local State`, session files and any home-local key material must continue to
travel together. No credential export, profile conversion, or Google-specific
workaround is introduced here. Earlier history added `basic` storage and stable
identity while the unsafe browser flag already existed; it does not establish
that enabling the sandbox caused the reported Google logout.

## Existing container migration

Docker security options require container replacement; they are not an in-place
[container update option](https://docs.docker.com/reference/cli/docker/container/update/).
New ordinary containers carry `io.chariox.chromium-seccomp=<policy-file-sha256>`
as a configuration migration marker. It is not a sandbox security attestation.

Before browser activation or an already-requested forced/stale-image recreation,
`ensure_container` inspects this marker with a bounded Docker call. A missing or
different marker returns a nonzero exit with the stable error
`slice.chromium_sandbox_migration_required` before stopping, deleting, restoring
the home volume, or refreshing the old container. A saved-home archive's mere
presence does not establish that current live state has been saved. A failed
inspection also leaves the existing container untouched. Non-browser reuse of a
legacy container remains available; compatible containers retain the existing
recreation behavior.

The kernel now handles this migration inside the existing slice-start operation
in `runtime/slice_command_executor/chromium_migration.rs`. The blocking supervisor
retains that operation guard even if the requesting future is cancelled.

1. Run the current bounded desktop-stop helper against the original container,
   including headless slices, then use the existing full-container snapshot path
   to create a unique migration saved-state generation. Its image, synced home
   archive and manifest remain separate from prior saved-state generations.
   Stop and provider shutdown target the validated immutable source ID, even if
   an external Docker operation rebinds its old name. The archive helper is
   created exclusively and removed only by its returned immutable ID.
2. Commit the saved-state event and active slice reference together in one
   existing-writer SQLite transaction. Publish the volatile state only after
   that transaction commits; queue admission or a partially written event pair
   cannot authorize container replacement or establish completion.
3. Retain the stopped original under a checkpoint-derived rollback name. Its
   immutable container ID is recorded on the checkpoint image as
   `io.chariox.chromium-source-container`. The replacement is labelled with
   `io.chariox.chromium-migration=<checkpoint-id>`. On retry/restart, the kernel
   checks these identities and the expected named home-volume mount before
   mutation; an unrelated container is left untouched.
4. Restore from the checkpoint through the ordinary provisioner. Before browser
   startup, stream the archive and use GNU tar's
   [comparison operation](https://www.gnu.org/s/tar/manual/html_node/compare.html)
   on the archived profile subtree (default `.config/chariox-slice-chromium`,
   or an explicit normalized path under `/home/slice`) and `.local/share/keyrings`.
   The profile override comes from the retained image environment. Other home
   files are restored normally and are not hashed for this
   browser gate. The current launcher and verification helper must overlay
   successfully before the browser can start.
5. Check the actual `chrome://sandbox` PID/network report and independent
   renderer process metadata: non-root real/effective/saved/filesystem UIDs,
   nested `NSpid` hierarchy, no effective capabilities, no-new-privileges, and a
   seccomp filter beyond the browser's container baseline. Readable PID/network
   namespace inodes must also differ from the browser's. Linux
   [ptrace access checks](https://man7.org/linux/man-pages/man2/ptrace.2.html)
   can deny namespace links for Chromium's deliberately non-dumpable sandbox
   processes; this denial does not discard their remaining process evidence.
   In that case PID nesting still comes from kernel
   [`NSpid` metadata](https://man7.org/linux/man-pages/man5/proc_pid_status.5.html),
   while network isolation relies on Chromium's own diagnostic report. This
   probe does not claim independent network-inode observation when access was
   denied and never adds ptrace privileges. Verify the original profile path
   and `basic` password-storage option, then mark the migration complete and
   remove the old stopped container. Retain the checkpoint/image for recovery.

If creation, validation or publication fails, rollback rechecks identities,
removes only the owned replacement by immutable ID, restores the checkpoint home,
and renames the old stopped container back when it still exists. A currently
mounted running home volume cannot be overwritten. Failed rollback is reported
as requiring retry with the retained checkpoint ID; it is never reported as a
successful start. A checkpoint failure can leave the original stopped, while
preserving its container and data. If the original was independently restarted
after failure, the next attempt captures fresh state before replacement.
Restore helpers carry checkpoint and slice labels. A retry checks both labels,
the exact home mount, and the immutable container ID before removing an
interrupted helper; it does not remove unrelated volume holders. Cleanup failure
cannot report restoration success. This recovery runs before both replacement
and rollback restores.
Once completion commits, later audit or cleanup failures cannot cause rollback.
A later start that retries cleanup also preserves the completed replacement if
startup or sandbox verification fails; it cannot restore an older checkpoint
over subsequent user work.

The migration uses existing saved-state records and audit events. It introduces
no client protocol shape or separate runtime authority. Configuration markers
and image labels bind recovery objects; they do not replace actual sandbox
verification. The runtime checks compare archived home-local profile bytes and
the browser's sandbox configuration. Custom profiles outside `/home/slice`, or
symlinks whose data resides outside the home archive, retain their existing
image/workspace storage ownership; their contents are outside this home
comparison gate. Real service authentication remains a separate release
validation requirement.

## Verification and release evidence

Run the deterministic checks without starting a browser or Docker container:

```sh
node --test --test-concurrency=1 apps/kernel/slice-linux-docker/chromium-sandbox.test.mjs apps/kernel/slice-linux-docker/chromium-restore.test.mjs apps/kernel/slice-linux-docker/docker/chromium-sandbox-probe.test.mjs
bash -n apps/kernel/slice-linux-docker/docker/slice-screen.sh apps/kernel/slice-linux-docker/provision-linux-docker-slice.sh
```

The shell fixtures execute production function bodies with recording stubs for
process, display, CDP and Docker operations. They cover the pinned default-deny
profile, creation policy, root rejection, session restoration, warm and cold
recovery, URL argument separation, retained profile bytes, bounded CDP calls,
non-destructive migration errors, interrupted helper ownership checks, and
cleanup failures. Temporary profile trees are removed after
each test. These checks validate launch/configuration behavior. The real GNU tar
restore/corruption fixture runs where GNU tar is available; it is explicitly
skipped on macOS hosts using BSD tar. The released Linux container uses GNU tar.
Rust tests inject transaction and SQLite publication failures and exercise
restart identity checks; these require the kernel test suite.

P1.14 release acceptance remains open until the integrated release image is tested
on supported macOS-hosted and Linux-hosted Docker setups. Required evidence:

- Actual non-root Chromium renderer namespaces, capabilities and seccomp status;
  startup, CDP fallback, crash recovery and repeated restart remain sandboxed.
- Kernel save/restore and complete container/home replacement retain session and
  persistent cookies, localStorage, IndexedDB, Cache Storage and service workers.
  Server revocation and deliberate storage loss must fail the authentication check.
- Legacy `.config` profile upgrade, old-container migration, checkpoint failure
  and failed-recreation rollback preserve the expected current user state.
- Human-confirmed Google login remains authenticated after full save/replacement/
  restore. Fixture cookies and an initial login alone do not satisfy this check.

App origin/storage partitioning, App network policy, kernel-managed App Tabs,
authenticated viewer input and trusted conversation composition have their own
release gates. Enabling the renderer sandbox does not complete them. The existing
local terminal insecure-origin development exception also remains a separate
hardening dependency with its fixture transport replacement.
