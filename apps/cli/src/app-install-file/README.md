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
