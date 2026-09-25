# Install a local package into the connected kernel

In an attached local or remote TUI session:

```
/app install "./My App.cxapp"
/app operation
/app cancel
```

The session comes from the attached terminal. Paths resolve from the terminal's
original working directory, including when the native launcher changes its own
child working directory. Quoting allows spaces; paths are never shell commands.
Only the terminal reads the file. The kernel receives the existing upload and
installation requests, and validates the package against its enrolled publisher.
A publisher must already be enrolled; the package cannot enroll its own key.

The terminal displays checking/upload progress and the durable operation ID. Use
`/app operation` to refresh preparation, approval, startup or completion status.
Review and respond through the existing owner-scoped kernel approval panel.
Installation requests never approve capabilities or information-set access.
`/app operation ID` and `/app cancel ID` inspect or cancel a retained operation
from another terminal without supplying local file paths.

`/app update INSTALLATION "./My App.cxapp"` uses the same transfer and operation
to replace an installation's release (same App ID, publisher and data schema;
its data is kept). It reads the installation's generation once and fences the
update on it, so a concurrent change fails as a conflict without effect.

A transfer holds one descriptor, hashes the archive and each bounded chunk, and
checks descriptor/path identity and chunk contents before upload. Files must be
regular, nonempty `.cxapp` files at most 128 MiB. Each request carries at most
512 KiB of decoded bytes. A detected change aborts the terminal's own upload and
prevents installation submission.

Within the same terminal process, reconnect retries keep the original upload and
operation IDs. After a connection error, repeating the same install command
resumes from the kernel's accepted offset. A lost Begin reply is reconciled using
the same operation ID. Cancellation waits for the outstanding request to settle
before aborting its upload; it preserves durable cancelled/committed receipts.
Once verification has durably staged the package, the upload can be released.

Normal terminal exit joins retained transfer/status/cancel work before detaching.
A submitted installation remains kernel-owned; exiting does not auto-cancel it.
Transport failure may leave upload cleanup to the kernel's original expiry.
A killed terminal process loses its in-memory transfer IDs; already submitted
operations remain queryable by the printed ID. No automatic background status
polling, OS file association, standalone `chariox app install`, or real Node App
execution is claimed by these terminal fixtures.

Focused tests use actual local files and a stateful shared-protocol fixture. They
exercise immutable file checks, quoted/current-session dispatch, lost replies,
request ordering on cancellation, reconnect IDs, retained receipts, shutdown and
local-only path history. They do not start a kernel, relay, provider or native App.

## Developer loop

```
/app dev ./my-app [--key PRIVATE]
/app dev stop
```

`/app dev` takes an App source directory as created by `chariox app create`
(`app.json` and `bundle/`). Each cycle runs the same `app pack` helper command
(which verifies the archive with the installer's verifier) into a private
temporary directory, never inside the App directory, then finds the owner's
active installation of the manifest's App ID (`ListAppInstallations`, all pages).
The first cycle installs when none exists; otherwise it updates that installation
through this same transfer and operation, then follows the operation to its
result: `Packed ID VERSION (sha256:…)`, then `Installed`/`Updated ... to
generation N` or the failure (for example, a data schema change fails with
`app_update_migration_required`). App data is kept across updates. A release that
declares the same capabilities is approved by kernel policy without a prompt;
changed capabilities wait for the owner in the approval panel, like install.
The loop never opens views: open one with `/app open INSTALLATION`, and reopen it
after each update because views stay on the generation they were opened on.

The signing key defaults to `~/.chariox/dev/app-publisher/private`, the location
used by the `chariox app keygen` example in `packages/app-package/README.md`;
`--key` selects another. A missing key fails with the keygen and publisher
enrollment steps. The publisher must be enrolled in the kernel (`/app publisher
enroll`) before the first install succeeds.

The directory is watched recursively (`fs.watch`), ignoring dotfiles and
`node_modules`, with a 500 ms debounce. Cycles never overlap: changes during a
cycle produce exactly one more cycle afterwards. There is one loop per terminal;
starting another stops the previous one. `/app dev stop`, terminal exit (the same
drain as installation transfers), a watcher error, or a cycle that finds the
terminal attached to another session stops the loop and removes its temporary
directory. Detaching has no separate hook: the watcher stays open until the next
change (which then stops the loop) or exit. A cycle already running finishes its
current kernel request; a submitted operation remains kernel-owned and visible
with `/app operation`. After a connection failure the attempt stays retained;
the next cycle first settles it: a submitted operation is followed to its result
and reported as `Previous build (sha256:…): ...`, then its local record is
dropped, while an upload that never reached Begin is cancelled. The cycle then
packs and uploads the current sources as a new operation, updating the
installation if the lost operation had installed it. `/app cancel` also works.

Focused tests (`app-dev-loop.test.ts`) inject the watcher, packer, installer and
kernel list; they cover install then update, coalescing, stop and failures. Two
use the real installer with a fixture that drops the connection after Begin.
